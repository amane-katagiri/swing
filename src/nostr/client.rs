use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{Stream, StreamExt};
use nostr_sdk::prelude::*;

use super::{
    SITE_SUBSCRIPTION_ID, SiteEvent, budget, is_follow_set_of, is_newer_replaceable,
    newest_by_address, parse_site_event, plausible_at, select_latest,
};
use crate::signer::Signer;

pub fn bounded_client(max_event_bytes: u32) -> Client {
    client_with(relay_limits(max_event_bytes))
}

fn client_with(limits: RelayLimits) -> Client {
    Client::builder()
        .relay_limits(limits)
        .admit_policy(MatchingIds)
        .build()
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
        Kind::Custom(FOLLOW_SET_KIND),
        Some(budget::MAX_FOLLOW_SET_EVENT_BYTES),
    );
    limits
}

const FOLLOW_SET_KIND: u16 = 30000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy)]
struct FetchCap {
    events: usize,
    bytes: usize,
}

impl FetchCap {
    const TOTAL: Self = Self {
        events: budget::MAX_FETCH_TOTAL_EVENTS,
        bytes: budget::MAX_FETCH_TOTAL_BYTES,
    };

    const PER_REQ: Self = Self {
        events: budget::MAX_RELAY_FETCH_LIMIT,
        bytes: budget::MAX_FETCH_TOTAL_BYTES / budget::FETCH_CONCURRENCY,
    };
}

fn event_bytes(event: &Event) -> usize {
    event.as_json().len()
}

fn capped_limit(count: usize, per: usize) -> usize {
    count.saturating_mul(per).min(budget::MAX_RELAY_FETCH_LIMIT)
}

pub struct RelayClient {
    pub client: Client,
    pub signer: Signer,
    relays: Vec<String>,
}

impl RelayClient {
    pub async fn connect(signer: Signer, relays: &[String]) -> Result<Self> {
        let client = client_with(relay_client_limits());
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

    // fetch_events drops everything once its buffer overflows, so over-cap results are truncated here instead.
    async fn fetch(&self, filters: Vec<Filter>, context: &'static str) -> Result<Vec<Event>> {
        let stream = self
            .client
            .stream_events(filters)
            .timeout(FETCH_TIMEOUT)
            .await
            .context(context)?;
        Ok(collect_newest(stream, FetchCap::PER_REQ).await)
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
        self.fetch_all(reqs, context).await
    }

    pub async fn fetch_follow_set(&self, mirror_set: &str) -> Result<Option<Event>> {
        let filter = Filter::new()
            .kind(Kind::Custom(FOLLOW_SET_KIND))
            .author(self.public_key())
            .identifier(mirror_set)
            // 2x: a relay may hand back a stale duplicate of a replaceable event.
            .limit(capped_limit(1, 2));
        let events = self.fetch_one(filter, "fetching follow set").await?;
        let now = Timestamp::now().as_secs();
        Ok(events
            .into_iter()
            .filter(|e| is_follow_set_of(e, &self.public_key(), mirror_set))
            .filter(|e| plausible_at(e.created_at.as_secs(), now))
            .reduce(|a, b| if is_newer_replaceable(&b, &a) { b } else { a }))
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
        let requested: HashSet<PublicKey> = authors.iter().copied().collect();
        Ok(events
            .into_iter()
            .filter(|e| e.kind == kind && requested.contains(&e.pubkey))
            .collect())
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
        let requested: HashSet<PublicKey> = reporters.iter().copied().collect();
        Ok(reports_for_sites(events, kind, sites)
            .into_iter()
            .filter(|e| requested.contains(&e.pubkey))
            .collect())
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
        let requested: HashSet<PublicKey> = authors.iter().copied().collect();
        let sets = events.into_iter().filter(|e| {
            requested.contains(&e.pubkey) && is_follow_set_of(e, &e.pubkey, mirror_set)
        });
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

pub trait ReportRelay {
    fn public_key(&self) -> PublicKey;
    fn fetch_own_reports(
        &self,
        report_kind: u16,
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn send_report(&self, report: EventBuilder) -> impl Future<Output = Result<bool>> + Send;
}

impl ReportRelay for RelayClient {
    fn public_key(&self) -> PublicKey {
        RelayClient::public_key(self)
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Vec<Event>> {
        let filter = Filter::new()
            .kind(Kind::Custom(report_kind))
            .author(RelayClient::public_key(self))
            .limit(capped_limit(budget::MAX_SITES_PER_AUTHOR_LISTED, 2));
        self.fetch_one(filter, "fetching own replica reports").await
    }

    async fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> Result<Vec<Event>> {
        let kind = Kind::Custom(report_kind);
        let events = self
            .fetch_per_author(
                reporters,
                "fetching replica reports about own sites",
                |reporter| {
                    let filter = Filter::new()
                        .kind(kind)
                        .author(reporter)
                        .pubkey(author)
                        .limit(budget::MAX_SITES_PER_AUTHOR_LISTED * 2);
                    match since {
                        Some(since) => filter.since(Timestamp::from_secs(since)),
                        None => filter,
                    }
                },
            )
            .await?;
        let requested: HashSet<PublicKey> = reporters.iter().copied().collect();
        Ok(events
            .into_iter()
            .filter(|e| {
                e.kind == kind
                    && requested.contains(&e.pubkey)
                    && e.tags.public_keys().any(|pk| pk == author)
            })
            .collect())
    }

    async fn send_report(&self, report: EventBuilder) -> Result<bool> {
        let event = self.sign(report).await.context("signing replica report")?;
        let output = self.publish_to_relays(&event).await?;
        Ok(!output.success.is_empty())
    }
}

impl<T: ReportRelay + Send + Sync> ReportRelay for std::sync::Arc<T> {
    fn public_key(&self) -> PublicKey {
        T::public_key(self)
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Vec<Event>> {
        T::fetch_own_reports(self, report_kind).await
    }

    async fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> Result<Vec<Event>> {
        T::fetch_reports_about(self, report_kind, author, reporters, since).await
    }

    async fn send_report(&self, report: EventBuilder) -> Result<bool> {
        T::send_report(self, report).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaySendResult {
    pub relay: String,
    pub ok: bool,
    pub error: Option<String>,
}

pub fn relay_send_results(
    relays: &[String],
    output: &Output<EventId, EventSendStatus, String>,
) -> Vec<RelaySendResult> {
    relays
        .iter()
        .map(|relay_url| {
            let parsed = RelayUrl::parse(relay_url).ok();
            let ok = parsed
                .as_ref()
                .is_some_and(|u| output.success.contains_key(u));
            let error = if ok {
                None
            } else {
                parsed.as_ref().and_then(|u| output.failed.get(u)).cloned()
            };
            RelaySendResult {
                relay: relay_url.clone(),
                ok,
                error,
            }
        })
        .collect()
}

pub fn print_relay_line(relay: &str, ok: bool) {
    if ok {
        println!("  \u{2713} {relay}");
    } else {
        println!("  \u{2717} {relay}");
    }
}

pub fn print_relay_send_result_lines(results: &[RelaySendResult]) {
    for result in results {
        print_relay_line(&result.relay, result.ok);
    }
}

async fn collect_newest<E: Display>(
    mut stream: impl Stream<Item = (RelayUrl, Result<Event, E>)> + Unpin,
    cap: FetchCap,
) -> Vec<Event> {
    let mut newest: BTreeSet<Event> = BTreeSet::new();
    let mut bytes = 0usize;
    let now = Timestamp::now().as_secs();
    while let Some((url, item)) = stream.next().await {
        match item {
            Ok(event) if !plausible_at(event.created_at.as_secs(), now) => {
                tracing::debug!(relay = %url, event_id = %event.id, "skipping an event dated too far ahead");
            }
            Ok(event) => {
                let size = event_bytes(&event);
                if newest.insert(event) {
                    bytes += size;
                }
                while newest.len() > cap.events || bytes > cap.bytes {
                    let Some(oldest) = newest.pop_last() else {
                        break;
                    };
                    bytes -= event_bytes(&oldest);
                }
            }
            Err(e) => tracing::debug!(relay = %url, error = %e, "skipping a streamed event"),
        }
    }
    newest.into_iter().collect()
}

async fn gather(
    batches: impl Stream<Item = Result<Vec<Event>>>,
    cap: FetchCap,
    deadline: Duration,
    context: &'static str,
) -> Result<Vec<Event>> {
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let collect = async {
        let mut batches = std::pin::pin!(batches);
        while let Some(batch) = batches.next().await {
            for event in batch? {
                let size = event_bytes(&event);
                if out.len() >= cap.events || bytes.saturating_add(size) > cap.bytes {
                    tracing::warn!(
                        context,
                        "relay answers exceed the fetch budget; keeping what fits"
                    );
                    return Ok(());
                }
                bytes += size;
                out.push(event);
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    match tokio::time::timeout(deadline, collect).await {
        Ok(result) => result?,
        Err(_) => tracing::warn!(
            context,
            "relays did not finish within the fetch deadline; keeping what arrived"
        ),
    }
    Ok(out)
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
mod tests {
    use super::super::fixtures::{follow_set, make_site_event, report, site_event_with};
    use super::*;
    use crate::nostr::site_coordinate;
    use crate::test_support::{CID_A, CID_B, keys};

    #[test]
    fn relay_send_results_reports_success_and_failure_per_relay() {
        let id = EventBuilder::new(Kind::TextNote, "")
            .finalize(&keys())
            .unwrap()
            .id;
        let mut output: Output<EventId, EventSendStatus, String> = Output::new(id);
        let ok_url = RelayUrl::parse("wss://ok.example").unwrap();
        let failed_url = RelayUrl::parse("wss://failed.example").unwrap();
        output.success.insert(ok_url, EventSendStatus::Sent);
        output
            .failed
            .insert(failed_url, "connection refused".to_string());

        let relays = vec![
            "wss://ok.example".to_string(),
            "wss://failed.example".to_string(),
            "wss://unknown.example".to_string(),
        ];
        let results = relay_send_results(&relays, &output);

        assert_eq!(
            results,
            vec![
                RelaySendResult {
                    relay: "wss://ok.example".to_string(),
                    ok: true,
                    error: None,
                },
                RelaySendResult {
                    relay: "wss://failed.example".to_string(),
                    ok: false,
                    error: Some("connection refused".to_string()),
                },
                RelaySendResult {
                    relay: "wss://unknown.example".to_string(),
                    ok: false,
                    error: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn the_relay_client_drops_events_over_the_size_or_tag_limits() {
        let local = LocalRelayBuilder::default()
            .max_event_size(1024 * 1024)
            .build();
        local.run().await.unwrap();
        let url = local.url().await.to_string();
        let k = keys();
        let now = Timestamp::now().as_secs();
        let small = make_site_event(&k, 35980, "small.example", CID_A, now);
        let big = site_event_with(
            &k,
            "big.example",
            "t",
            &"a".repeat(budget::MAX_EVENT_BYTES as usize),
        );
        let tagged = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("tagged.example"))
            .tag(Tag::custom("cid", [CID_A.to_string()]))
            .tags((0..budget::MAX_EVENT_TAGS).map(|i| Tag::custom("x", [i.to_string()])))
            .finalize(&k)
            .unwrap();
        let seeder = Client::default();
        seeder.add_relay(url.as_str()).await.unwrap();
        seeder.connect().await;
        for event in [&small, &big, &tagged] {
            seeder.send_event(event).await.unwrap();
        }
        let stored = seeder
            .fetch_events(Filter::new().kind(Kind::Custom(35980)))
            .timeout(Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(stored.len(), 3);
        seeder.shutdown().await;

        let client = RelayClient::connect(Signer::Local(keys()), &[url])
            .await
            .unwrap();
        let ids: Vec<EventId> = client
            .fetch_site_events(35980, &[k.public_key()])
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec![small.id]);
        client.shutdown().await;
    }

    #[test]
    fn reports_for_sites_keeps_only_the_requested_coordinates() {
        let author = keys().public_key();
        let reporter = keys();
        let wanted = report(&reporter, &author, "wanted.example", &[CID_A], 1);
        let other = report(&reporter, &author, "other.example", &[CID_A], 1);
        let sites = [site_coordinate(35980, &author, "wanted.example")];
        let kept: Vec<EventId> =
            reports_for_sites(vec![wanted.clone(), other], Kind::Custom(35981), &sites)
                .into_iter()
                .map(|e| e.id)
                .collect();
        assert_eq!(kept, vec![wanted.id]);
    }

    #[tokio::test]
    async fn collect_newest_truncates_to_the_newest_instead_of_failing() {
        let k = keys();
        let url = RelayUrl::parse("wss://a.example").unwrap();
        let events: Vec<Event> = (1..=5)
            .map(|at| make_site_event(&k, 35980, &format!("s{at}.example"), CID_A, at))
            .collect();
        let items: Vec<(RelayUrl, Result<Event, String>)> = events
            .iter()
            .chain(&events)
            .cloned()
            .map(|e| (url.clone(), Ok(e)))
            .chain([(url.clone(), Err("bad event".to_string()))])
            .collect();
        let cap = FetchCap {
            events: 3,
            bytes: usize::MAX,
        };
        let kept: Vec<u64> = collect_newest(futures_util::stream::iter(items), cap)
            .await
            .into_iter()
            .map(|e| e.created_at.as_secs())
            .collect();
        assert_eq!(kept, vec![5, 4, 3]);

        let two = event_bytes(&events[4]) + event_bytes(&events[3]);
        let items = events
            .iter()
            .cloned()
            .map(|e| (url.clone(), Ok::<_, String>(e)));
        let cap = FetchCap {
            events: 100,
            bytes: two,
        };
        let kept: Vec<u64> = collect_newest(futures_util::stream::iter(items), cap)
            .await
            .into_iter()
            .map(|e| e.created_at.as_secs())
            .collect();
        assert_eq!(kept, vec![5, 4]);

        let far = Timestamp::now().as_secs() + super::super::MAX_FUTURE_SKEW + 3600;
        let future = make_site_event(&k, 35980, "future.example", CID_A, far);
        let items = [future, events[4].clone()]
            .into_iter()
            .map(|e| (url.clone(), Ok::<_, String>(e)));
        let cap = FetchCap {
            events: 1,
            bytes: usize::MAX,
        };
        let kept: Vec<u64> = collect_newest(futures_util::stream::iter(items), cap)
            .await
            .into_iter()
            .map(|e| e.created_at.as_secs())
            .collect();
        assert_eq!(kept, vec![5]);
    }

    #[tokio::test]
    async fn gather_stops_at_the_total_budget_and_keeps_what_arrived_by_the_deadline() {
        let k = keys();
        let batch: Vec<Event> = (1..=3)
            .map(|at| make_site_event(&k, 35980, &format!("s{at}.example"), CID_A, at))
            .collect();
        let batches = futures_util::stream::iter([Ok(batch.clone()), Ok(batch.clone())]);
        let cap = FetchCap {
            events: 4,
            bytes: usize::MAX,
        };
        let out = gather(batches, cap, Duration::from_secs(5), "test")
            .await
            .unwrap();
        assert_eq!(out.len(), 4);

        let slow =
            futures_util::stream::iter([Ok(batch.clone())]).chain(futures_util::stream::pending());
        let out = gather(slow, FetchCap::TOTAL, Duration::from_millis(100), "test")
            .await
            .unwrap();
        assert_eq!(out.len(), 3);

        let failing =
            futures_util::stream::iter([Ok(batch), Err(anyhow::anyhow!("relay refused"))]);
        assert!(
            gather(failing, FetchCap::TOTAL, Duration::from_secs(5), "test")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn follow_sets_get_more_room_than_site_events() {
        let local = LocalRelayBuilder::default()
            .max_event_size(1024 * 1024)
            .build();
        local.run().await.unwrap();
        let url = local.url().await.to_string();
        let k = keys();
        let mut tags = vec![Tag::identifier("swing")];
        tags.extend(
            (0..budget::MAX_FOLLOW_SET_ENTRIES).map(|_| Tag::public_key(keys().public_key())),
        );
        let full = EventBuilder::new(Kind::Custom(30000), "")
            .tags(tags)
            .finalize(&k)
            .unwrap();
        assert!(event_bytes(&full) > budget::MAX_EVENT_BYTES as usize);
        let seeder = Client::default();
        seeder.add_relay(url.as_str()).await.unwrap();
        seeder.connect().await;
        seeder.send_event(&full).await.unwrap();
        seeder.shutdown().await;

        let client = RelayClient::connect(Signer::Local(keys()), &[url])
            .await
            .unwrap();
        let sets = client
            .fetch_follow_sets("swing", &[k.public_key()])
            .await
            .unwrap();
        assert_eq!(sets[&k.public_key()].id, full.id);
        client.shutdown().await;
    }

    #[derive(Debug)]
    struct IgnoresFilter;

    impl QueryPolicy for IgnoresFilter {
        fn admit_query<'a>(
            &'a self,
            query: &'a mut Filter,
            _addr: &'a std::net::SocketAddr,
        ) -> std::pin::Pin<Box<dyn Future<Output = QueryPolicyResult> + Send + 'a>> {
            Box::pin(async move {
                *query = Filter::new();
                QueryPolicyResult::Accept
            })
        }
    }

    async fn relay_that_ignores_filters(events: &[Event]) -> (LocalRelay, RelayClient) {
        let local = LocalRelayBuilder::default()
            .query_policy(IgnoresFilter)
            .build();
        local.run().await.unwrap();
        let url = local.url().await.to_string();
        let seeder = Client::default();
        seeder.add_relay(url.as_str()).await.unwrap();
        seeder.connect().await;
        for event in events {
            seeder.send_event(event).await.unwrap();
        }
        seeder.shutdown().await;
        let client = RelayClient::connect(Signer::Local(keys()), &[url])
            .await
            .unwrap();
        (local, client)
    }

    #[tokio::test]
    async fn fetches_drop_events_the_relay_returns_for_unrequested_authors() {
        let now = Timestamp::now().as_secs();
        let wanted = keys();
        let stranger = keys();
        let wanted_site = make_site_event(&wanted, 35980, "wanted.example", CID_A, now);
        let stranger_site = make_site_event(&stranger, 35980, "stranger.example", CID_B, now);
        let wanted_set = follow_set(&wanted, "swing", now, "wanted");
        let stranger_set = follow_set(&stranger, "swing", now, "stranger");
        let wanted_report = report(
            &stranger,
            &wanted.public_key(),
            "wanted.example",
            &[CID_A],
            now,
        );
        let stranger_report = report(
            &wanted,
            &stranger.public_key(),
            "stranger.example",
            &[CID_B],
            now,
        );
        let all = [
            wanted_site.clone(),
            stranger_site.clone(),
            wanted_set.clone(),
            stranger_set.clone(),
            wanted_report.clone(),
            stranger_report.clone(),
        ];
        let (_local, client) = relay_that_ignores_filters(&all).await;

        let unfiltered = client
            .fetch_one(
                Filter::new()
                    .kind(Kind::Custom(35980))
                    .author(wanted.public_key()),
                "probe",
            )
            .await
            .unwrap();
        assert!(unfiltered.iter().any(|e| e.id == stranger_site.id));

        let sites = client
            .fetch_site_events(35980, &[wanted.public_key()])
            .await
            .unwrap();
        assert_eq!(
            sites.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![wanted_site.id]
        );

        let sets = client
            .fetch_follow_sets("swing", &[wanted.public_key()])
            .await
            .unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[&wanted.public_key()].id, wanted_set.id);

        let reports = client
            .fetch_replica_reports(
                35981,
                &[site_coordinate(
                    35980,
                    &wanted.public_key(),
                    "wanted.example",
                )],
            )
            .await
            .unwrap();
        assert_eq!(
            reports.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![wanted_report.id]
        );

        let wanted_coordinate = [site_coordinate(
            35980,
            &wanted.public_key(),
            "wanted.example",
        )];
        let by_stranger = client
            .fetch_replica_reports_by(35981, &wanted_coordinate, &[stranger.public_key()])
            .await
            .unwrap();
        assert_eq!(
            by_stranger.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![wanted_report.id]
        );
        let by_wanted = client
            .fetch_replica_reports_by(35981, &wanted_coordinate, &[wanted.public_key()])
            .await
            .unwrap();
        assert!(by_wanted.is_empty());

        let referencing = client
            .fetch_follow_set_authors_referencing("swing", &[wanted.public_key()])
            .await
            .unwrap();
        assert!(referencing.is_empty());

        client.shutdown().await;
    }

    type DbFuture<'a, T> = std::pin::Pin<
        Box<dyn Future<Output = Result<T, nostr_database::error::Error>> + Send + 'a>,
    >;

    #[derive(Debug)]
    struct ServesForgeries(Vec<Event>);

    impl NostrDatabase for ServesForgeries {
        fn backend(&self) -> &'static str {
            "forgeries"
        }

        fn features(&self) -> Features {
            Features {
                persistent: false,
                event_expiration: false,
                full_text_search: false,
                request_to_vanish: false,
            }
        }

        fn save_event<'a>(&'a self, _event: &'a Event) -> DbFuture<'a, SaveEventStatus> {
            Box::pin(async { Ok(SaveEventStatus::Success) })
        }

        fn check_id<'a>(&'a self, _event_id: &'a EventId) -> DbFuture<'a, DatabaseEventStatus> {
            Box::pin(async { Ok(DatabaseEventStatus::NotExistent) })
        }

        fn event_by_id<'a>(&'a self, _event_id: &'a EventId) -> DbFuture<'a, Option<Event>> {
            Box::pin(async { Ok(None) })
        }

        fn count(&self, _filter: Filter) -> DbFuture<'_, usize> {
            Box::pin(async { Ok(self.0.len()) })
        }

        fn query(&self, _filter: Filter) -> DbFuture<'_, BTreeSet<Event>> {
            Box::pin(async { Ok(self.0.iter().cloned().collect()) })
        }

        fn delete(&self, _filter: Filter) -> DbFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }

        fn wipe(&self) -> DbFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn a_forged_copy_of_a_verified_event_does_not_hide_the_real_one() {
        let k = keys();
        let now = Timestamp::now().as_secs();
        let genuine = make_site_event(&k, 35980, "a.example", CID_A, now);
        let mut forged = genuine.clone();
        forged.content = "forged".to_string();

        let honest = LocalRelay::new();
        honest.run().await.unwrap();
        let honest_url = honest.url().await;
        let seeder = Client::default();
        seeder.add_relay(honest_url.clone()).await.unwrap();
        seeder.connect().await;
        seeder.send_event(&genuine).await.unwrap();
        seeder.shutdown().await;
        let hostile = LocalRelayBuilder::default()
            .database(ServesForgeries(vec![forged]))
            .build();
        hostile.run().await.unwrap();
        let hostile_url = hostile.url().await;

        let client = RelayClient::connect(Signer::Local(keys()), &[honest_url.to_string()])
            .await
            .unwrap();
        let first = client
            .fetch_site_events(35980, &[k.public_key()])
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].content, genuine.content);

        client.client.add_relay(hostile_url.clone()).await.unwrap();
        client
            .client
            .connect_relay(hostile_url.clone())
            .await
            .unwrap();
        let filter = Filter::new()
            .kind(Kind::Custom(35980))
            .author(k.public_key());
        let from_hostile = client
            .client
            .fetch_events(ReqTarget::single(hostile_url, [filter]))
            .timeout(Duration::from_secs(5))
            .await
            .unwrap();
        assert!(from_hostile.is_empty());

        let both = client
            .fetch_site_events(35980, &[k.public_key()])
            .await
            .unwrap();
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].content, genuine.content);
        client.shutdown().await;
    }
}
