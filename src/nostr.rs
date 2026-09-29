use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::signer::Signer;

pub const SITE_SUBSCRIPTION_ID: &str = "swing-sites";

// `created_at` is self-declared, so this tolerance is what separates honest clock skew from a forged timestamp.
pub const MAX_FUTURE_SKEW: u64 = 900;

pub fn plausible_at(created_at: u64, now: u64) -> bool {
    created_at <= now.saturating_add(MAX_FUTURE_SKEW)
}

// Relay events come from anyone under throwaway keys, so the read-only views get hard ceilings.
pub mod budget {
    pub const MAX_FOLLOW_SET_ENTRIES: usize = 500;
    pub const MAX_SITES_PER_AUTHOR_LISTED: usize = 50;
    pub const MAX_REPORTS_PER_SITE: usize = 200;
    pub const MAX_CRAWL_NODES: usize = 1000;
    pub const MAX_REFERENCING_LISTED: usize = 50;
    pub const MAX_RELAY_FETCH_LIMIT: usize = 20_000;
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
        let client = Client::new();
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

    async fn fetch(&self, filter: Filter, context: &'static str) -> Result<Vec<Event>> {
        Ok(self
            .client
            .fetch_events(filter)
            .timeout(Duration::from_secs(30))
            .await
            .context(context)?
            .into_iter()
            .collect())
    }

    pub async fn fetch_follow_set(&self, mirror_set: &str) -> Result<Option<Event>> {
        let filter = Filter::new()
            .kind(Kind::Custom(30000))
            .author(self.public_key())
            .identifier(mirror_set)
            // 2x: a relay may hand back a stale duplicate of this single replaceable event.
            .limit(capped_limit(1, 2));
        let events = self.fetch(filter, "fetching follow set").await?;
        let now = Timestamp::now().as_secs();
        // Defense in depth against a relay that ignores the filter.
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
        if authors.is_empty() {
            return Ok(Vec::new());
        }
        let kind = Kind::Custom(site_event_kind);
        let filter = Filter::new()
            .kind(kind)
            .authors(authors.iter().copied())
            .limit(capped_limit(
                authors.len(),
                budget::MAX_SITES_PER_AUTHOR_LISTED,
            ));
        let events = self.fetch(filter, "fetching site events").await?;
        let requested: HashSet<PublicKey> = authors.iter().copied().collect();
        Ok(events
            .into_iter()
            .filter(|e| e.kind == kind && requested.contains(&e.pubkey))
            .collect())
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
            // 2x: a relay may hand back a stale duplicate of this single replaceable event.
            .limit(capped_limit(1, 2));
        let events = self
            .fetch(filter, "fetching your latest site event")
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
        let filter = Filter::new()
            .kind(kind)
            .coordinates(sites)
            .limit(capped_limit(sites.len(), budget::MAX_REPORTS_PER_SITE));
        let events = self.fetch(filter, "fetching replica reports").await?;
        let requested: HashSet<String> = sites.iter().map(|c| c.to_string()).collect();
        Ok(events
            .into_iter()
            .filter(|e| {
                e.kind == kind
                    && e.tags.iter().any(|t| {
                        t.kind() == "a" && t.content().is_some_and(|a| requested.contains(a))
                    })
            })
            .collect())
    }

    pub async fn fetch_follow_sets(
        &self,
        mirror_set: &str,
        authors: &[PublicKey],
    ) -> Result<HashMap<PublicKey, Event>> {
        if authors.is_empty() {
            return Ok(HashMap::new());
        }
        // 2x: a relay may hand back a stale duplicate of a replaceable event.
        let filter = Filter::new()
            .kind(Kind::Custom(30000))
            .authors(authors.iter().copied())
            .identifier(mirror_set)
            .limit(capped_limit(authors.len(), 2));
        let events = self.fetch(filter, "fetching follow sets").await?;
        let now = Timestamp::now().as_secs();
        let requested: HashSet<PublicKey> = authors.iter().copied().collect();
        let mut newest: HashMap<PublicKey, Event> = HashMap::new();
        for event in events {
            if !requested.contains(&event.pubkey)
                || !is_follow_set_of(&event, &event.pubkey, mirror_set)
                || !plausible_at(event.created_at.as_secs(), now)
            {
                continue;
            }
            match newest.get(&event.pubkey) {
                Some(current) if !is_newer_replaceable(&event, current) => {}
                _ => {
                    newest.insert(event.pubkey, event);
                }
            }
        }
        Ok(newest)
    }

    pub async fn fetch_follow_set_authors_referencing(
        &self,
        mirror_set: &str,
        targets: &[PublicKey],
    ) -> Result<HashSet<PublicKey>> {
        if targets.is_empty() {
            return Ok(HashSet::new());
        }
        let filter = Filter::new()
            .kind(Kind::Custom(30000))
            .identifier(mirror_set)
            .pubkeys(targets.iter().copied())
            .limit(capped_limit(targets.len(), 100));
        let events = self
            .fetch(filter, "fetching follow sets that reference accounts")
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
        since: Option<u64>,
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn send_report(&self, report: EventBuilder) -> impl Future<Output = Result<bool>> + Send;
}

impl ReportRelay for RelayClient {
    fn public_key(&self) -> PublicKey {
        RelayClient::public_key(self)
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Vec<Event>> {
        // One addressable report per site this account hosts; a single author can't list more
        // sites than MAX_SITES_PER_AUTHOR_LISTED elsewhere, so reuse that with the same 2x margin
        // for stale duplicates of a replaceable event.
        let filter = Filter::new()
            .kind(Kind::Custom(report_kind))
            .author(RelayClient::public_key(self))
            .limit(capped_limit(budget::MAX_SITES_PER_AUTHOR_LISTED, 2));
        self.fetch(filter, "fetching own replica reports").await
    }

    async fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        since: Option<u64>,
    ) -> Result<Vec<Event>> {
        let mut filter = Filter::new()
            .kind(Kind::Custom(report_kind))
            .pubkey(author)
            .limit(capped_limit(
                budget::MAX_SITES_PER_AUTHOR_LISTED,
                budget::MAX_REPORTS_PER_SITE,
            ));
        if let Some(since) = since {
            filter = filter.since(Timestamp::from_secs(since));
        }
        self.fetch(filter, "fetching replica reports about own sites")
            .await
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
        since: Option<u64>,
    ) -> Result<Vec<Event>> {
        T::fetch_reports_about(self, report_kind, author, since).await
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

// NIP-01: for replaceable events the later created_at wins, ties broken by the lowest id.
pub fn is_newer_replaceable(a: &Event, b: &Event) -> bool {
    (a.created_at, std::cmp::Reverse(a.id)) > (b.created_at, std::cmp::Reverse(b.id))
}

pub fn is_follow_set_of(event: &Event, author: &PublicKey, mirror_set: &str) -> bool {
    event.kind == Kind::Custom(30000)
        && event.pubkey == *author
        && event.tags.identifier().as_deref() == Some(mirror_set)
        && event.verify().is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowSetChoice {
    pub event: Event,
    pub save: bool,
    pub republish: bool,
}

pub fn choose_follow_set(
    fetched: Option<Event>,
    fetch_succeeded: bool,
    stored: Option<Event>,
    now: u64,
) -> Option<FollowSetChoice> {
    // The stored copy is filtered too, or a poisoned one could never be displaced by a plausible fetch.
    let fetched = fetched.filter(|e| plausible_at(e.created_at.as_secs(), now));
    let stored = stored.filter(|e| plausible_at(e.created_at.as_secs(), now));
    match (fetched, stored) {
        (None, None) => None,
        (Some(event), None) => Some(FollowSetChoice {
            event,
            save: true,
            republish: false,
        }),
        (None, Some(event)) => Some(FollowSetChoice {
            event,
            save: false,
            republish: fetch_succeeded,
        }),
        (Some(fetched), Some(stored)) if fetched.id == stored.id => Some(FollowSetChoice {
            event: stored,
            save: false,
            republish: false,
        }),
        (Some(fetched), Some(stored)) if is_newer_replaceable(&fetched, &stored) => {
            Some(FollowSetChoice {
                event: fetched,
                save: true,
                republish: false,
            })
        }
        (Some(_), Some(stored)) => Some(FollowSetChoice {
            event: stored,
            save: false,
            republish: true,
        }),
    }
}

pub fn extract_follow_set_pubkeys(event: &Event) -> Vec<PublicKey> {
    follow_set_pubkeys_capped(event).0
}

pub fn follow_set_pubkeys_capped(event: &Event) -> (Vec<PublicKey>, bool) {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for pk in event.tags.public_keys() {
        if !seen.insert(pk) {
            continue;
        }
        if out.len() == budget::MAX_FOLLOW_SET_ENTRIES {
            return (out, true);
        }
        out.push(pk);
    }
    (out, false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteEvent {
    pub pubkey: PublicKey,
    pub d: String,
    pub cid: String,
    pub url: Option<String>,
    pub size: Option<u64>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub created_at: u64,
}

fn tag_value<'a>(event: &'a Event, kind: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|t| t.kind() == kind)
        .and_then(|t| t.content())
}

const MAX_D_TAG_BYTES: usize = 253;
const MAX_URL_TAG_BYTES: usize = 2048;
const MAX_TITLE_TAG_BYTES: usize = 256;

pub fn validate_d_tag(d: &str) -> Result<()> {
    if d.is_empty() {
        anyhow::bail!("empty d tag");
    }
    if d.len() > MAX_D_TAG_BYTES {
        anyhow::bail!("d tag too long: {} bytes", d.len());
    }
    if d.chars().any(|c| c.is_control()) {
        anyhow::bail!("d tag contains control characters");
    }
    Ok(())
}

pub fn valid_http_url(url: &str) -> bool {
    if url.len() > MAX_URL_TAG_BYTES || url.chars().any(|c| c.is_control()) {
        return false;
    }
    match reqwest::Url::parse(url) {
        Ok(parsed) => parsed.scheme() == "http" || parsed.scheme() == "https",
        Err(_) => false,
    }
}

pub fn valid_title(title: &str) -> bool {
    !title.is_empty()
        && title.len() <= MAX_TITLE_TAG_BYTES
        && !title.chars().any(|c| c.is_control())
}

// A UnixFS directory root is always dag-pb, so any other codec is rejected before a wasted fetch.
const DAG_PB_CODEC: u64 = 0x70;

// One spelling per content, so the string comparisons downstream see one version.
pub fn canonical_cid(s: &str) -> Result<String> {
    let cid = cid::Cid::try_from(s).context("invalid cid tag")?;
    if cid.codec() != DAG_PB_CODEC {
        anyhow::bail!(
            "invalid cid tag: codec {:#x} is not dag-pb, cannot be a UnixFS directory",
            cid.codec()
        );
    }
    let cid = cid.into_v1().context("invalid cid tag")?;
    Ok(cid.to_string())
}

pub fn parse_site_event(event: &Event, expected_kind: u16) -> Result<SiteEvent> {
    if event.kind != Kind::Custom(expected_kind) {
        anyhow::bail!("unexpected kind {}", event.kind);
    }
    let d = event.tags.identifier().context("missing d tag")?;
    validate_d_tag(&d)?;
    let cid = canonical_cid(tag_value(event, "cid").context("missing cid tag")?)?;
    // A malformed url doesn't invalidate an otherwise-valid site update; only the url is discarded.
    let url = tag_value(event, "url")
        .map(|s| s.to_string())
        .filter(|u| valid_http_url(u));
    let size = tag_value(event, "size").and_then(|s| s.parse::<u64>().ok());
    let title = tag_value(event, "title")
        .map(|s| s.to_string())
        .filter(|t| valid_title(t));
    Ok(SiteEvent {
        pubkey: event.pubkey,
        d,
        cid,
        url,
        size,
        title,
        message: Some(event.content.clone()).filter(|m| !m.is_empty()),
        created_at: event.created_at.as_secs(),
    })
}

pub fn select_latest(events: &[SiteEvent], now: u64) -> HashMap<(String, String), SiteEvent> {
    let mut latest: HashMap<(String, String), SiteEvent> = HashMap::new();
    for ev in events {
        if !plausible_at(ev.created_at, now) {
            continue;
        }
        let key = (ev.pubkey.to_hex(), ev.d.clone());
        match latest.get(&key) {
            Some(existing) if existing.created_at >= ev.created_at => {}
            _ => {
                latest.insert(key, ev.clone());
            }
        }
    }
    latest
}

pub fn cap_sites_per_author<'a>(
    sites: impl IntoIterator<Item = &'a SiteEvent>,
    max: usize,
) -> Vec<&'a SiteEvent> {
    let mut by_author: BTreeMap<PublicKey, Vec<&'a SiteEvent>> = BTreeMap::new();
    for ev in sites {
        by_author.entry(ev.pubkey).or_default().push(ev);
    }
    let mut out = Vec::new();
    for evs in by_author.values_mut() {
        evs.sort_by(|a, b| a.d.cmp(&b.d));
        out.extend(evs.iter().take(max).copied());
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn build_site_event_builder(
    site_event_kind: u16,
    d: &str,
    cid: &str,
    url: Option<&str>,
    size: Option<u64>,
    title: Option<&str>,
    message: Option<&str>,
) -> EventBuilder {
    let mut builder = EventBuilder::new(Kind::Custom(site_event_kind), message.unwrap_or(""))
        .tag(Tag::identifier(d))
        .tag(Tag::custom("cid", [cid.to_string()]));
    if let Some(url) = url {
        builder = builder.tag(Tag::custom("url", [url.to_string()]));
    }
    if let Some(size) = size {
        builder = builder.tag(Tag::custom("size", [size.to_string()]));
    }
    if let Some(title) = title {
        builder = builder.tag(Tag::custom("title", [title.to_string()]));
    }
    builder.tag(Tag::custom(
        "alt",
        [format!("SWING site announcement: {d}")],
    ))
}

// `expiration` is self-declared, so only re-signing (not just the field) proves the reporter is still alive.
pub const MAX_REPORT_AGE: u64 = 7 * 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaReport {
    pub reporter: PublicKey,
    pub author: PublicKey,
    pub d: String,
    pub cids: BTreeSet<String>,
    pub created_at: u64,
    pub expiration: Option<u64>,
}

impl ReplicaReport {
    pub fn counts_at(&self, now: u64) -> bool {
        plausible_at(self.created_at, now)
            && now.saturating_sub(self.created_at) <= MAX_REPORT_AGE
            && self.expiration.is_none_or(|exp| exp > now)
    }
}

pub fn site_coordinate(site_event_kind: u16, author: &PublicKey, d: &str) -> Coordinate {
    Coordinate::new(Kind::Custom(site_event_kind), *author).identifier(d)
}

pub fn parse_replica_report(
    event: &Event,
    report_kind: u16,
    site_event_kind: u16,
) -> Result<ReplicaReport> {
    if event.kind != Kind::Custom(report_kind) {
        anyhow::bail!("unexpected kind {}", event.kind);
    }
    let identifier = event.tags.identifier().context("missing d tag")?;
    let (author_hex, d) = identifier
        .split_once(':')
        .context("d tag is not <pubkey>:<site>")?;
    let author = PublicKey::from_hex(author_hex).context("invalid author in d tag")?;
    if author.to_hex() != author_hex {
        anyhow::bail!("author in d tag is not lowercase hex");
    }
    validate_d_tag(d)?;
    let expected = site_coordinate(site_event_kind, &author, d).to_string();
    if tag_value(event, "a") != Some(expected.as_str()) {
        anyhow::bail!("a tag does not match d tag");
    }
    let cids = event
        .tags
        .iter()
        .filter(|t| t.kind() == "cid")
        .map(|t| canonical_cid(t.content().context("empty cid tag")?))
        .collect::<Result<BTreeSet<String>>>()?;
    let expiration = tag_value(event, "expiration")
        .map(|s| s.parse::<u64>().context("invalid expiration tag"))
        .transpose()?;
    Ok(ReplicaReport {
        reporter: event.pubkey,
        author,
        d: d.to_string(),
        cids,
        created_at: event.created_at.as_secs(),
        expiration,
    })
}

pub fn newest_by_address(events: impl IntoIterator<Item = Event>, now: u64) -> Vec<Event> {
    let mut newest: HashMap<(PublicKey, Kind, Option<String>), Event> = HashMap::new();
    for event in events {
        if !plausible_at(event.created_at.as_secs(), now) {
            continue;
        }
        let key = (event.pubkey, event.kind, event.tags.identifier());
        match newest.get(&key) {
            Some(current) if !is_newer_replaceable(&event, current) => {}
            _ => {
                newest.insert(key, event);
            }
        }
    }
    newest.into_values().collect()
}

pub fn build_replica_report_builder(
    report_kind: u16,
    site_event_kind: u16,
    author: &PublicKey,
    d: &str,
    cids: &BTreeSet<String>,
    expiration: Timestamp,
) -> EventBuilder {
    let author_hex = author.to_hex();
    EventBuilder::new(Kind::Custom(report_kind), "")
        .tag(Tag::identifier(format!("{author_hex}:{d}")))
        .tag(Tag::custom(
            "a",
            [site_coordinate(site_event_kind, author, d).to_string()],
        ))
        .tag(Tag::public_key(*author))
        .tags(cids.iter().map(|cid| Tag::custom("cid", [cid.clone()])))
        .tag(Tag::expiration(expiration))
        .tag(Tag::custom("alt", [format!("SWING replica report: {d}")]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, CID_A, CID_B, keys, site_event_fixture as site_event};

    fn make_site_event(keys: &Keys, kind: u16, d: &str, cid: &str, created_at: u64) -> Event {
        build_site_event_builder(
            kind,
            d,
            cid,
            Some("https://example.com/"),
            Some(1234),
            None,
            None,
        )
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(keys)
        .unwrap()
    }

    #[test]
    fn site_event_content_is_the_message() {
        let k = keys();
        let cid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
        let ev = build_site_event_builder(
            35980,
            "example.com",
            cid,
            None,
            None,
            None,
            Some("Add posts"),
        )
        .finalize(&k)
        .unwrap();
        assert_eq!(ev.content, "Add posts");
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.message.as_deref(), Some("Add posts"));
    }

    #[test]
    fn parses_valid_site_event() {
        let k = keys();
        let ev = make_site_event(
            &k,
            35980,
            "example.com",
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
            1000,
        );
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.d, "example.com");
        assert_eq!(
            parsed.cid,
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
        );
        assert_eq!(parsed.url.as_deref(), Some("https://example.com/"));
        assert_eq!(parsed.size, Some(1234));
        assert_eq!(parsed.created_at, 1000);
        assert_eq!(parsed.pubkey, k.public_key());
        assert_eq!(parsed.message, None);
        assert_eq!(parsed.title, None);
        assert!(
            ev.tags
                .iter()
                .any(|t| t.as_slice() == ["alt", "SWING site announcement: example.com"])
        );
    }

    #[test]
    fn accepts_cidv0_and_canonicalizes_it() {
        let k = keys();
        let ev = make_site_event(
            &k,
            35980,
            "example.com",
            "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR",
            1000,
        );
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(
            parsed.cid,
            "bafybeie5nqv6kd3qnfjuphmab6atx72bbz674e35siysg2di3q5jltctqq"
        );
    }

    #[test]
    fn canonical_cid_normalizes_every_spelling_to_cidv1_base32() {
        assert_eq!(
            canonical_cid("QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR").unwrap(),
            "bafybeie5nqv6kd3qnfjuphmab6atx72bbz674e35siysg2di3q5jltctqq"
        );
        assert_eq!(
            canonical_cid("zdj7Wic6KcJAfWz1c9o4M6kq9Lwd5BfbxkVafnrojaaGiSFxM").unwrap(),
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
        );
        assert_eq!(
            canonical_cid("bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi").unwrap(),
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
        );
        assert!(canonical_cid("not-a-cid").is_err());
    }

    #[test]
    fn canonical_cid_rejects_non_dag_pb_codecs() {
        assert!(
            canonical_cid("bafkreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi").is_err()
        );
        assert!(
            canonical_cid("bafyreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi").is_err()
        );
    }

    #[test]
    fn rejects_a_site_event_whose_cid_codec_is_not_dag_pb() {
        let k = keys();
        let ev = make_site_event(
            &k,
            35980,
            "example.com",
            "bafkreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
            1000,
        );
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn rejects_wrong_kind() {
        let k = keys();
        let ev = make_site_event(&k, 1, "example.com", "bafyxyz", 1000);
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn rejects_missing_cid_tag() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .finalize(&k)
            .unwrap();
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn rejects_invalid_cid_value() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom("cid", ["not-a-cid".to_string()]))
            .finalize(&k)
            .unwrap();
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn rejects_missing_d_tag() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::custom("cid", ["bafyxyz".to_string()]))
            .finalize(&k)
            .unwrap();
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn rejects_empty_d_tag() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::custom("d", [String::new()]))
            .tag(Tag::custom("cid", ["bafyxyz".to_string()]))
            .finalize(&k)
            .unwrap();
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn rejects_d_tag_too_long() {
        let k = keys();
        let long_d = "a".repeat(254);
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::custom("d", [long_d]))
            .tag(Tag::custom("cid", ["bafyxyz".to_string()]))
            .finalize(&k)
            .unwrap();
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn rejects_d_tag_with_control_characters() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::custom("d", ["example.com\ncom".to_string()]))
            .tag(Tag::custom("cid", ["bafyxyz".to_string()]))
            .finalize(&k)
            .unwrap();
        assert!(parse_site_event(&ev, 35980).is_err());
    }

    #[test]
    fn accepts_d_tag_at_max_length() {
        let k = keys();
        let max_d = "a".repeat(253);
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::custom("d", [max_d.clone()]))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.d, max_d);
    }

    #[test]
    fn drops_invalid_url_but_keeps_event() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .tag(Tag::custom("url", ["not a url".to_string()]))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.url, None);
    }

    #[test]
    fn drops_url_with_control_characters() {
        let k = keys();
        let url = "https://example.com/\x1b]0;pwned\x07";
        assert!(reqwest::Url::parse(url).is_ok());
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .tag(Tag::custom("url", [url.to_string()]))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.url, None);
    }

    #[test]
    fn drops_non_http_scheme_url() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .tag(Tag::custom("url", ["ftp://example.com/".to_string()]))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.url, None);
    }

    #[test]
    fn drops_too_long_url() {
        let k = keys();
        let long_url = format!("https://example.com/{}", "a".repeat(2048));
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .tag(Tag::custom("url", [long_url]))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.url, None);
    }

    #[test]
    fn keeps_valid_https_url() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .tag(Tag::custom("url", ["https://example.com/".to_string()]))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.url.as_deref(), Some("https://example.com/"));
    }

    #[test]
    fn title_round_trips() {
        let k = keys();
        let cid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
        let ev = build_site_event_builder(
            35980,
            "example.com",
            cid,
            None,
            None,
            Some("あまねけ！"),
            None,
        )
        .finalize(&k)
        .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("あまねけ！"));
    }

    #[test]
    fn drops_oversized_title_but_keeps_event() {
        let k = keys();
        let long_title = "a".repeat(257);
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .tag(Tag::custom("title", [long_title]))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.title, None);
    }

    #[test]
    fn drops_title_with_control_characters() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .tag(Tag::custom("title", ["hello\nworld".to_string()]))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.title, None);
    }

    #[test]
    fn absent_title_is_none() {
        let k = keys();
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom(
                "cid",
                ["bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string()],
            ))
            .finalize(&k)
            .unwrap();
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.title, None);
    }

    #[test]
    fn extracts_follow_set_pubkeys() {
        let author = keys();
        let target1 = keys().public_key();
        let target2 = keys().public_key();
        let ev = EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier("site-mirror"))
            .tag(Tag::public_key(target1))
            .tag(Tag::public_key(target2))
            .finalize(&author)
            .unwrap();
        let pubkeys = extract_follow_set_pubkeys(&ev);
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&target1));
        assert!(pubkeys.contains(&target2));
    }

    #[test]
    fn follow_set_pubkeys_capped_dedups_repeated_p_tags() {
        let author = keys();
        let target = keys().public_key();
        let ev = EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier("site-mirror"))
            .tag(Tag::public_key(target))
            .tag(Tag::public_key(target))
            .finalize(&author)
            .unwrap();

        let (pubkeys, truncated) = follow_set_pubkeys_capped(&ev);

        assert_eq!(pubkeys, vec![target]);
        assert!(!truncated);
    }

    #[test]
    fn follow_set_pubkeys_capped_stops_at_the_budget() {
        let author = keys();
        let first = keys().public_key();
        let mut builder = EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier("site-mirror"))
            .tag(Tag::public_key(first));
        let extra: Vec<PublicKey> = (0..budget::MAX_FOLLOW_SET_ENTRIES)
            .map(|_| Keys::generate().public_key())
            .collect();
        for pk in &extra {
            builder = builder.tag(Tag::public_key(*pk));
        }
        let ev = builder.finalize(&author).unwrap();

        let (pubkeys, truncated) = follow_set_pubkeys_capped(&ev);

        assert!(truncated);
        assert_eq!(pubkeys.len(), budget::MAX_FOLLOW_SET_ENTRIES);
        assert_eq!(pubkeys[0], first);
        assert_eq!(
            extract_follow_set_pubkeys(&ev).len(),
            budget::MAX_FOLLOW_SET_ENTRIES
        );
    }

    #[test]
    fn cap_sites_per_author_keeps_the_first_n_by_d() {
        let a = keys().public_key();
        let b = keys().public_key();
        let sites = vec![
            site_event(a, "c.example", 1),
            site_event(a, "a.example", 1),
            site_event(a, "b.example", 1),
            site_event(b, "only.example", 1),
        ];

        let capped = cap_sites_per_author(&sites, 2);

        let a_ds: Vec<&str> = capped
            .iter()
            .filter(|s| s.pubkey == a)
            .map(|s| s.d.as_str())
            .collect();
        assert_eq!(a_ds, vec!["a.example", "b.example"]);
        assert_eq!(capped.iter().filter(|s| s.pubkey == b).count(), 1);
    }

    #[test]
    fn select_latest_keeps_max_created_at_per_pubkey_and_d() {
        let k1 = keys();
        let k2 = keys();
        let events = vec![
            SiteEvent {
                pubkey: k1.public_key(),
                d: "site-a".into(),
                cid: "bafy-old".into(),
                url: None,
                size: None,
                title: None,
                message: None,
                created_at: 100,
            },
            SiteEvent {
                pubkey: k1.public_key(),
                d: "site-a".into(),
                cid: "bafy-new".into(),
                url: None,
                size: None,
                title: None,
                message: None,
                created_at: 200,
            },
            SiteEvent {
                pubkey: k1.public_key(),
                d: "site-b".into(),
                cid: "bafy-other-site".into(),
                url: None,
                size: None,
                title: None,
                message: None,
                created_at: 50,
            },
            SiteEvent {
                pubkey: k2.public_key(),
                d: "site-a".into(),
                cid: "bafy-k2".into(),
                url: None,
                size: None,
                title: None,
                message: None,
                created_at: 999,
            },
        ];
        let latest = select_latest(&events, 1000);
        assert_eq!(latest.len(), 3);
        assert_eq!(
            latest[&(k1.public_key().to_hex(), "site-a".to_string())].cid,
            "bafy-new"
        );
        assert_eq!(
            latest[&(k1.public_key().to_hex(), "site-b".to_string())].cid,
            "bafy-other-site"
        );
        assert_eq!(
            latest[&(k2.public_key().to_hex(), "site-a".to_string())].cid,
            "bafy-k2"
        );
    }

    #[test]
    fn select_latest_ignores_an_implausible_future_created_at() {
        let k = keys();
        fn ev(pubkey: PublicKey, cid: &str, created_at: u64) -> SiteEvent {
            SiteEvent {
                pubkey,
                d: "site-a".into(),
                cid: cid.into(),
                url: None,
                size: None,
                title: None,
                message: None,
                created_at,
            }
        }
        let events = vec![
            ev(k.public_key(), "bafy-plausible", 1000 + MAX_FUTURE_SKEW),
            ev(k.public_key(), "bafy-forged", 1000 + MAX_FUTURE_SKEW + 1),
        ];
        let latest = select_latest(&events, 1000);
        assert_eq!(
            latest[&(k.public_key().to_hex(), "site-a".to_string())].cid,
            "bafy-plausible"
        );
    }

    fn follow_set(keys: &Keys, d: &str, created_at: u64, marker: &str) -> Event {
        EventBuilder::new(Kind::Custom(30000), marker)
            .tag(Tag::identifier(d))
            .custom_created_at(Timestamp::from_secs(created_at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn newer_replaceable_uses_created_at_then_lowest_id() {
        let k = keys();
        let old = follow_set(&k, "swing", 100, "a");
        let new = follow_set(&k, "swing", 200, "b");
        assert!(is_newer_replaceable(&new, &old));
        assert!(!is_newer_replaceable(&old, &new));
        assert!(!is_newer_replaceable(&old, &old));

        let x = follow_set(&k, "swing", 100, "x");
        let y = follow_set(&k, "swing", 100, "y");
        let (low, high) = if x.id < y.id { (x, y) } else { (y, x) };
        assert!(is_newer_replaceable(&low, &high));
        assert!(!is_newer_replaceable(&high, &low));
    }

    #[test]
    fn follow_set_identity_checks_kind_author_d_and_signature() {
        let k = keys();
        let ev = follow_set(&k, "swing", 100, "");
        assert!(is_follow_set_of(&ev, &k.public_key(), "swing"));
        assert!(!is_follow_set_of(&ev, &k.public_key(), "other"));
        assert!(!is_follow_set_of(&ev, &keys().public_key(), "swing"));

        let mut tampered = ev.clone();
        tampered.content = "changed".to_string();
        assert!(!is_follow_set_of(&tampered, &k.public_key(), "swing"));

        let site = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("swing"))
            .finalize(&k)
            .unwrap();
        assert!(!is_follow_set_of(&site, &k.public_key(), "swing"));
    }

    #[test]
    fn choose_follow_set_prefers_the_newest_and_repairs_relays() {
        let k = keys();
        let old = follow_set(&k, "swing", 100, "old");
        let new = follow_set(&k, "swing", 200, "new");

        assert_eq!(choose_follow_set(None, true, None, 1000), None);

        let c = choose_follow_set(Some(new.clone()), true, None, 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, true, false));

        let c = choose_follow_set(Some(new.clone()), true, Some(old.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, true, false));

        let c = choose_follow_set(Some(old.clone()), true, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, true));

        let c = choose_follow_set(Some(new.clone()), true, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, false));

        let c = choose_follow_set(None, true, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, true));

        let c = choose_follow_set(None, false, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, false));
    }

    #[test]
    fn choose_follow_set_drops_an_implausible_future_fetch_and_keeps_stored() {
        let k = keys();
        let stored = follow_set(&k, "swing", 100, "stored");
        let poisoned_fetch = follow_set(&k, "swing", 1000 + MAX_FUTURE_SKEW + 1, "poisoned");

        let c = choose_follow_set(
            Some(poisoned_fetch.clone()),
            true,
            Some(stored.clone()),
            1000,
        )
        .unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (stored.id, false, true));
    }

    #[test]
    fn choose_follow_set_drops_a_poisoned_stored_copy_and_saves_the_fetched_one() {
        let k = keys();
        let poisoned_stored = follow_set(&k, "swing", 1000 + MAX_FUTURE_SKEW + 1, "poisoned");
        let fetched = follow_set(&k, "swing", 100, "fetched");

        let c =
            choose_follow_set(Some(fetched.clone()), true, Some(poisoned_stored), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (fetched.id, true, false));
    }

    #[test]
    fn choose_follow_set_returns_none_when_only_a_poisoned_stored_copy_exists() {
        let k = keys();
        let poisoned_stored = follow_set(&k, "swing", 1000 + MAX_FUTURE_SKEW + 1, "poisoned");

        assert_eq!(
            choose_follow_set(None, true, Some(poisoned_stored), 1000),
            None
        );
    }

    fn report(
        reporter: &Keys,
        author: &PublicKey,
        d: &str,
        cids: &[&str],
        created_at: u64,
    ) -> Event {
        test_support::replica_report_event(reporter, author, d, cids, created_at, created_at + 100)
    }

    fn report_with_tags(reporter: &Keys, d: &str, a: &str, cid: &str) -> Event {
        EventBuilder::new(Kind::Custom(35981), "")
            .tag(Tag::identifier(d))
            .tag(Tag::custom("a", [a.to_string()]))
            .tag(Tag::custom("cid", [cid.to_string()]))
            .finalize(reporter)
            .unwrap()
    }

    #[test]
    fn replica_report_round_trips() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(&reporter, &author, "a:b.example", &[CID_B, CID_A], 1000);

        assert_eq!(
            ev.tags.identifier().unwrap(),
            format!("{}:a:b.example", author.to_hex())
        );
        assert_eq!(
            tag_value(&ev, "a").unwrap(),
            format!("35980:{}:a:b.example", author.to_hex())
        );
        assert_eq!(ev.tags.public_keys().collect::<Vec<_>>(), vec![author]);
        assert_eq!(
            tag_value(&ev, "alt"),
            Some("SWING replica report: a:b.example")
        );

        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert_eq!(parsed.reporter, reporter.public_key());
        assert_eq!(parsed.author, author);
        assert_eq!(parsed.d, "a:b.example");
        assert_eq!(
            parsed.cids,
            BTreeSet::from([CID_A.to_string(), canonical_cid(CID_B).unwrap()])
        );
        assert_eq!(parsed.created_at, 1000);
        assert_eq!(parsed.expiration, Some(1100));
        assert!(parsed.counts_at(1099));
        assert!(!parsed.counts_at(1100));
    }

    #[test]
    fn replica_report_without_cids_is_a_withdrawal() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(&reporter, &author, "example.com", &[], 1000);
        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert!(parsed.cids.is_empty());
    }

    #[test]
    fn replica_report_collapses_the_same_cid_written_in_two_spellings() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(
            &reporter,
            &author,
            "example.com",
            &[CID_A, "zdj7Wic6KcJAfWz1c9o4M6kq9Lwd5BfbxkVafnrojaaGiSFxM"],
            1000,
        );
        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert_eq!(parsed.cids, BTreeSet::from([CID_A.to_string()]));
    }

    #[test]
    fn replica_report_rejects_a_cid_tag_whose_codec_is_not_dag_pb() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(
            &reporter,
            &author,
            "example.com",
            &["bafyreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"],
            1000,
        );
        assert!(parse_replica_report(&ev, 35981, 35980).is_err());
    }

    #[test]
    fn replica_report_rejects_inconsistent_or_invalid_tags() {
        let reporter = keys();
        let author = keys().public_key();
        let hex = author.to_hex();
        let d = format!("{hex}:example.com");
        let a = format!("35980:{hex}:example.com");

        assert!(
            parse_replica_report(&report_with_tags(&reporter, &d, &a, CID_A), 35981, 35980).is_ok()
        );
        assert!(
            parse_replica_report(&report_with_tags(&reporter, &d, &a, CID_A), 35982, 35980)
                .is_err()
        );

        let other_site = format!("35980:{hex}:other.example");
        let other_kind = format!("30023:{hex}:example.com");
        let other_author = format!("35980:{}:example.com", keys().public_key().to_hex());
        for bad_a in [other_site, other_kind, other_author] {
            let ev = report_with_tags(&reporter, &d, &bad_a, CID_A);
            assert!(parse_replica_report(&ev, 35981, 35980).is_err(), "{bad_a}");
        }

        let upper = format!("{}:example.com", hex.to_uppercase());
        for bad_d in [
            "example.com".to_string(),
            "abc:example.com".to_string(),
            upper,
            format!("{hex}:"),
        ] {
            let ev = report_with_tags(&reporter, &bad_d, &a, CID_A);
            assert!(parse_replica_report(&ev, 35981, 35980).is_err(), "{bad_d}");
        }

        let ev = report_with_tags(&reporter, &d, &a, "not-a-cid");
        assert!(parse_replica_report(&ev, 35981, 35980).is_err());
    }

    fn report_with_expiration(reporter: &Keys, d: &str, a: &str, expiration: &str) -> Event {
        EventBuilder::new(Kind::Custom(35981), "")
            .tag(Tag::identifier(d))
            .tag(Tag::custom("a", [a.to_string()]))
            .tag(Tag::custom("expiration", [expiration.to_string()]))
            .finalize(reporter)
            .unwrap()
    }

    #[test]
    fn replica_report_rejects_a_malformed_expiration_tag() {
        let reporter = keys();
        let author = keys().public_key();
        let hex = author.to_hex();
        let d = format!("{hex}:example.com");
        let a = format!("35980:{hex}:example.com");

        for bad in ["not-a-number", "", "12.5", "-1"] {
            let ev = report_with_expiration(&reporter, &d, &a, bad);
            assert!(parse_replica_report(&ev, 35981, 35980).is_err(), "{bad}");
        }
    }

    #[test]
    fn replica_report_without_an_expiration_tag_parses_as_none() {
        let reporter = keys();
        let author = keys().public_key();
        let hex = author.to_hex();
        let d = format!("{hex}:example.com");
        let a = format!("35980:{hex}:example.com");

        let ev = report_with_tags(&reporter, &d, &a, CID_A);
        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert_eq!(parsed.expiration, None);
    }

    fn report_at(created_at: u64, expiration: Option<u64>) -> ReplicaReport {
        ReplicaReport {
            reporter: keys().public_key(),
            author: keys().public_key(),
            d: "example.com".to_string(),
            cids: BTreeSet::from([CID_A.to_string()]),
            created_at,
            expiration,
        }
    }

    #[test]
    fn counts_at_accepts_a_fresh_report_with_no_expiration() {
        assert!(report_at(1000, None).counts_at(1000));
    }

    #[test]
    fn counts_at_rejects_a_report_older_than_max_report_age() {
        let report = report_at(1000, None);
        assert!(report.counts_at(1000 + MAX_REPORT_AGE));
        assert!(!report.counts_at(1000 + MAX_REPORT_AGE + 1));

        let far_future_expiration = report_at(1000, Some(u64::MAX));
        assert!(!far_future_expiration.counts_at(1000 + MAX_REPORT_AGE + 1));
    }

    #[test]
    fn counts_at_rejects_created_at_beyond_the_future_skew() {
        let report = report_at(1000 + MAX_FUTURE_SKEW, None);
        assert!(report.counts_at(1000));

        let report = report_at(1000 + MAX_FUTURE_SKEW + 1, None);
        assert!(!report.counts_at(1000));
    }

    #[test]
    fn counts_at_rejects_expiration_exactly_at_now() {
        let report = report_at(1000, Some(2000));
        assert!(report.counts_at(1999));
        assert!(!report.counts_at(2000));
    }

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

    #[test]
    fn newest_by_address_keeps_one_event_per_author_kind_and_d() {
        let r1 = keys();
        let r2 = keys();
        let author = keys().public_key();
        let old = report(&r1, &author, "example.com", &[CID_A], 100);
        let new = report(&r1, &author, "example.com", &[CID_B], 200);
        let other_site = report(&r1, &author, "other.example", &[CID_A], 50);
        let other_reporter = report(&r2, &author, "example.com", &[CID_A], 50);

        let mut ids: Vec<EventId> = newest_by_address(
            vec![new.clone(), old, other_site.clone(), other_reporter.clone()],
            1000,
        )
        .into_iter()
        .map(|e| e.id)
        .collect();
        ids.sort();
        let mut expected = vec![new.id, other_site.id, other_reporter.id];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn newest_by_address_ignores_an_implausible_future_created_at() {
        let r = keys();
        let author = keys().public_key();
        let plausible = report(&r, &author, "example.com", &[CID_A], 1000 + MAX_FUTURE_SKEW);
        let forged = report(
            &r,
            &author,
            "example.com",
            &[CID_B],
            1000 + MAX_FUTURE_SKEW + 1,
        );

        let ids: Vec<EventId> = newest_by_address(vec![forged, plausible.clone()], 1000)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec![plausible.id]);
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
            .fetch(
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

        let referencing = client
            .fetch_follow_set_authors_referencing("swing", &[wanted.public_key()])
            .await
            .unwrap();
        assert!(referencing.is_empty());

        client.shutdown().await;
    }
}
