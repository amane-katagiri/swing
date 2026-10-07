use std::collections::{HashMap, HashSet};
use std::future::Future;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use nostr_sdk::prelude::*;

use super::{
    FOLLOW_SET_KIND, SITE_SUBSCRIPTION_ID, SiteEvent, budget, is_follow_set_of, newest_by_address,
    parse_site_event, select_latest,
};
use crate::signer::Signer;

mod fetch;
mod report_relay;
mod send;

use fetch::{
    FETCH_DEADLINE, FETCH_TIMEOUT, FetchCap, capped_limit, collect_newest, gather, relay_stream,
};
pub use report_relay::ReportRelay;
pub use send::{
    RelaySendResult, print_relay_line, print_relay_send_result_lines, relay_send_results,
};

pub fn bounded_client(max_event_bytes: u32) -> Client {
    builder_with(relay_limits(max_event_bytes)).build()
}

fn builder_with(limits: RelayLimits) -> ClientBuilder {
    Client::builder()
        .relay_limits(limits)
        .admit_policy(MatchingIds)
}

// nostr-sdk skips the signature check for an id it has verified before, so a copy whose content no longer matches the id must be dropped before the pool keeps it as the first copy.
#[derive(Debug)]
struct MatchingIds;

impl AdmitPolicy for MatchingIds {
    fn admit_event<'a>(
        &'a self,
        relay_url: &'a RelayUrl,
        _subscription_id: &'a SubscriptionId,
        event: &'a Event,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<AdmitStatus, nostr_sdk::error::Error>> + Send + 'a>,
    > {
        Box::pin(async move {
            if event.verify_id() {
                Ok(AdmitStatus::success())
            } else {
                tracing::debug!(relay = %relay_url, event_id = %event.id, "dropping an event whose id does not match it");
                Ok(AdmitStatus::rejected("id does not match the event"))
            }
        })
    }
}

fn relay_limits(max_event_bytes: u32) -> RelayLimits {
    let mut limits = RelayLimits::default();
    limits.messages.max_size = Some(budget::MAX_RELAY_MESSAGE_BYTES.max(max_event_bytes));
    limits.events.max_size = Some(max_event_bytes);
    limits.events.max_num_tags = Some(budget::MAX_EVENT_TAGS);
    limits
}

// Site and report kinds are configurable, so they take the default ceiling and only follow sets get more room.
fn relay_client_limits() -> RelayLimits {
    let mut limits = relay_limits(budget::MAX_EVENT_BYTES);
    limits.messages.max_size =
        Some(budget::MAX_RELAY_MESSAGE_BYTES.max(budget::MAX_FOLLOW_SET_EVENT_BYTES));
    limits.events = limits.events.set_max_size_per_kind(
        Kind::Custom(super::FOLLOW_SET_KIND),
        Some(budget::MAX_FOLLOW_SET_EVENT_BYTES),
    );
    limits
}

pub struct RelayClient {
    pub client: Client,
    pub signer: Signer,
    relays: Vec<String>,
}

impl RelayClient {
    pub async fn connect(signer: Signer, relays: &[String]) -> Result<Self> {
        // Without this a relay could flood fresh non-matching events and push the real answers out of the per-request cap.
        let client = builder_with(relay_client_limits())
            .verify_subscriptions(true)
            .build();
        for url in relays {
            client
                .add_relay(url.as_str())
                .await
                .with_context(|| format!("adding relay {url}"))?;
        }
        client.connect().await;
        Ok(Self {
            client,
            signer,
            relays: relays.to_vec(),
        })
    }

    pub fn relays(&self) -> &[String] {
        &self.relays
    }

    pub fn public_key(&self) -> PublicKey {
        self.signer.public_key()
    }

    pub async fn sign(&self, builder: EventBuilder) -> Result<Event> {
        self.signer.sign(builder).await
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
        self.signer.shutdown().await;
    }

    // Client::stream_events hides which relays reached EOSE, so each relay is streamed on its own to tell "nothing found" from "no relay answered".
    async fn fetch(&self, filters: Vec<Filter>, context: &'static str) -> Result<Vec<Event>> {
        let relays = self
            .client
            .relays()
            .with_capabilities(RelayCapabilities::READ)
            .await;
        let streams = relays
            .into_values()
            .map(|relay| relay_stream(relay, filters.clone()));
        let merged =
            futures_util::stream::select_all(streams).take_until(tokio::time::sleep(FETCH_TIMEOUT));
        let collected = collect_newest(std::pin::pin!(merged), FetchCap::PER_REQ).await;
        if collected.completed == 0 {
            anyhow::bail!("{context}: no relay answered");
        }
        Ok(collected.events)
    }

    async fn fetch_one(&self, filter: Filter, context: &'static str) -> Result<Vec<Event>> {
        self.fetch(vec![filter], context).await
    }

    async fn fetch_all(&self, reqs: Vec<Vec<Filter>>, context: &'static str) -> Result<Vec<Event>> {
        let batches = futures_util::stream::iter(reqs)
            .map(|filters| self.fetch(filters, context))
            .buffer_unordered(budget::FETCH_CONCURRENCY);
        gather(batches, FetchCap::TOTAL, FETCH_DEADLINE, context).await
    }

    async fn fetch_by_authors(
        &self,
        authors: &[PublicKey],
        context: &'static str,
        filter_for: impl Fn(&[PublicKey]) -> Filter,
    ) -> Result<Vec<Event>> {
        let reqs = authors
            .chunks(budget::AUTHORS_PER_FILTER)
            .map(|batch| vec![filter_for(batch)])
            .collect();
        self.fetch_all(reqs, context).await
    }

    // A limit shared by the whole batch would let one author's events crowd out the others.
    async fn fetch_per_author(
        &self,
        authors: &[PublicKey],
        context: &'static str,
        filter_for: impl Fn(PublicKey) -> Filter,
    ) -> Result<Vec<Event>> {
        let reqs = authors
            .chunks(budget::AUTHORS_PER_SPLIT_REQ)
            .map(|batch| batch.iter().map(|a| filter_for(*a)).collect())
            .collect();
        let events = self.fetch_all(reqs, context).await?;
        Ok(by_authors(events, authors))
    }

    pub async fn fetch_follow_set(&self, mirror_set: &str) -> Result<Option<Event>> {
        let filter = Filter::new()
            .kind(Kind::Custom(FOLLOW_SET_KIND))
            .author(self.public_key())
            .identifier(mirror_set)
            // 2x: a relay may hand back a stale duplicate of a replaceable event.
            .limit(capped_limit(1, 2));
        let events = self.fetch_one(filter, "fetching follow set").await?;
        let own = events
            .into_iter()
            .filter(|e| is_follow_set_of(e, &self.public_key(), mirror_set));
        Ok(newest_by_address(own, Timestamp::now().as_secs())
            .into_iter()
            .next())
    }

    pub async fn fetch_site_events(
        &self,
        site_event_kind: u16,
        authors: &[PublicKey],
    ) -> Result<Vec<Event>> {
        let kind = Kind::Custom(site_event_kind);
        let events = self
            .fetch_per_author(authors, "fetching site events", |author| {
                Filter::new()
                    .kind(kind)
                    .author(author)
                    .limit(budget::MAX_SITES_PER_AUTHOR_LISTED)
            })
            .await?;
        Ok(events.into_iter().filter(|e| e.kind == kind).collect())
    }

    pub async fn fetch_latest_sites(
        &self,
        site_event_kind: u16,
        authors: &[PublicKey],
    ) -> Result<HashMap<(String, String), SiteEvent>> {
        let events = self.fetch_site_events(site_event_kind, authors).await?;
        let parsed: Vec<SiteEvent> = events
            .iter()
            .filter_map(|e| match parse_site_event(e, site_event_kind) {
                Ok(site) => Some(site),
                Err(err) => {
                    tracing::debug!(event_id = %e.id, error = %err, "skipping invalid site event");
                    None
                }
            })
            .collect();
        Ok(select_latest(&parsed, Timestamp::now().as_secs()))
    }

    pub async fn fetch_own_latest_site(
        &self,
        site_event_kind: u16,
        d: &str,
    ) -> Result<Option<SiteEvent>> {
        let kind = Kind::Custom(site_event_kind);
        let own = self.public_key();
        let filter = Filter::new()
            .kind(kind)
            .author(own)
            .identifier(d)
            .limit(capped_limit(1, 2));
        let events = self
            .fetch_one(filter, "fetching your latest site event")
            .await?;
        let parsed: Vec<SiteEvent> = events
            .iter()
            .filter(|e| e.pubkey == own)
            .filter_map(|e| parse_site_event(e, site_event_kind).ok())
            .filter(|ev| ev.d == d)
            .collect();
        Ok(select_latest(&parsed, Timestamp::now().as_secs())
            .into_values()
            .next())
    }

    pub async fn fetch_replica_reports(
        &self,
        report_kind: u16,
        sites: &[Coordinate],
    ) -> Result<Vec<Event>> {
        if sites.is_empty() {
            return Ok(Vec::new());
        }
        let kind = Kind::Custom(report_kind);
        let reqs = sites
            .chunks(budget::COORDINATES_PER_FILTER)
            .map(|batch| {
                vec![
                    Filter::new()
                        .kind(kind)
                        .coordinates(batch)
                        .limit(capped_limit(batch.len(), budget::MAX_REPORTS_PER_SITE)),
                ]
            })
            .collect();
        let events = self.fetch_all(reqs, "fetching replica reports").await?;
        Ok(reports_for_sites(events, kind, sites))
    }

    pub async fn fetch_replica_reports_by(
        &self,
        report_kind: u16,
        sites: &[Coordinate],
        reporters: &[PublicKey],
    ) -> Result<Vec<Event>> {
        if sites.is_empty() {
            return Ok(Vec::new());
        }
        let kind = Kind::Custom(report_kind);
        let reqs = sites
            .chunks(budget::COORDINATES_PER_FILTER)
            .flat_map(|coordinates| {
                reporters
                    .chunks(budget::AUTHORS_PER_FILTER)
                    .map(move |batch| {
                        vec![
                            Filter::new()
                                .kind(kind)
                                .authors(batch.iter().copied())
                                .coordinates(coordinates)
                                .limit(capped_limit(
                                    batch.len().saturating_mul(coordinates.len()),
                                    2,
                                )),
                        ]
                    })
            })
            .collect();
        let events = self
            .fetch_all(reqs, "fetching replica reports by trusted reporters")
            .await?;
        Ok(by_authors(
            reports_for_sites(events, kind, sites),
            reporters,
        ))
    }

    pub async fn fetch_follow_sets(
        &self,
        mirror_set: &str,
        authors: &[PublicKey],
    ) -> Result<HashMap<PublicKey, Event>> {
        let events = self
            .fetch_by_authors(authors, "fetching follow sets", |batch| {
                Filter::new()
                    .kind(Kind::Custom(FOLLOW_SET_KIND))
                    .authors(batch.iter().copied())
                    .identifier(mirror_set)
                    .limit(capped_limit(batch.len(), 2))
            })
            .await?;
        let sets = by_authors(events, authors)
            .into_iter()
            .filter(|e| is_follow_set_of(e, &e.pubkey, mirror_set));
        Ok(newest_by_address(sets, Timestamp::now().as_secs())
            .into_iter()
            .map(|e| (e.pubkey, e))
            .collect())
    }

    pub async fn fetch_follow_set_authors_referencing(
        &self,
        mirror_set: &str,
        targets: &[PublicKey],
    ) -> Result<HashSet<PublicKey>> {
        let events = self
            .fetch_by_authors(
                targets,
                "fetching follow sets that reference accounts",
                |batch| {
                    Filter::new()
                        .kind(Kind::Custom(FOLLOW_SET_KIND))
                        .identifier(mirror_set)
                        .pubkeys(batch.iter().copied())
                        .limit(capped_limit(batch.len(), 100))
                },
            )
            .await?;
        Ok(events
            .into_iter()
            .filter(|e| {
                is_follow_set_of(e, &e.pubkey, mirror_set)
                    && e.tags.public_keys().any(|pk| targets.contains(&pk))
            })
            .map(|e| e.pubkey)
            .collect())
    }

    pub async fn subscribe_site_events(
        &self,
        site_event_kind: u16,
        authors: &[PublicKey],
    ) -> Result<()> {
        let id = SubscriptionId::new(SITE_SUBSCRIPTION_ID);
        if authors.is_empty() {
            let _ = self.client.unsubscribe(&id).await;
            return Ok(());
        }
        let filter = Filter::new()
            .kind(Kind::Custom(site_event_kind))
            .authors(authors.iter().copied());
        self.client
            .subscribe(filter)
            .with_id(id)
            .await
            .context("subscribing to site events")?;
        Ok(())
    }

    pub fn notifications(
        &self,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = ClientNotification> + Send>> {
        self.client.notifications()
    }

    pub async fn publish_to_relays(
        &self,
        event: &Event,
    ) -> Result<Output<EventId, EventSendStatus, String>> {
        let out = self
            .client
            .send_event(event)
            .to(self.relays.iter().map(|s| s.as_str()))
            .await
            .context("sending event to relays")?;
        Ok(out)
    }
}

fn by_authors(events: Vec<Event>, authors: &[PublicKey]) -> Vec<Event> {
    let requested: HashSet<PublicKey> = authors.iter().copied().collect();
    events
        .into_iter()
        .filter(|e| requested.contains(&e.pubkey))
        .collect()
}

fn reports_for_sites(events: Vec<Event>, kind: Kind, sites: &[Coordinate]) -> Vec<Event> {
    let requested: HashSet<String> = sites.iter().map(|c| c.to_string()).collect();
    events
        .into_iter()
        .filter(|e| {
            e.kind == kind
                && e.tags
                    .iter()
                    .any(|t| t.kind() == "a" && t.content().is_some_and(|a| requested.contains(a)))
        })
        .collect()
}

#[cfg(test)]
mod tests;
