use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

pub const SITE_SUBSCRIPTION_ID: &str = "swing-sites";

pub struct RelayClient {
    pub client: Client,
    pub keys: Keys,
    relays: Vec<String>,
}

impl RelayClient {
    pub async fn connect(secret_key: &str, relays: &[String]) -> Result<Self> {
        let keys = Keys::parse(secret_key).context("parsing Nostr secret key")?;
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
            keys,
            relays: relays.to_vec(),
        })
    }

    pub fn relays(&self) -> &[String] {
        &self.relays
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
            .author(self.keys.public_key())
            .identifier(mirror_set);
        let events = self.fetch(filter, "fetching follow set").await?;
        // Defense in depth against a relay that ignores the filter.
        Ok(events
            .into_iter()
            .filter(|e| is_follow_set_of(e, &self.keys.public_key(), mirror_set))
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
        let filter = Filter::new()
            .kind(Kind::Custom(site_event_kind))
            .authors(authors.iter().copied());
        self.fetch(filter, "fetching site events").await
    }

    pub async fn fetch_replica_reports(
        &self,
        report_kind: u16,
        sites: &[Coordinate],
    ) -> Result<Vec<Event>> {
        if sites.is_empty() {
            return Ok(Vec::new());
        }
        let filter = Filter::new()
            .kind(Kind::Custom(report_kind))
            .coordinates(sites);
        self.fetch(filter, "fetching replica reports").await
    }

    pub async fn fetch_follow_sets(
        &self,
        mirror_set: &str,
        authors: &[PublicKey],
    ) -> Result<HashMap<PublicKey, Event>> {
        if authors.is_empty() {
            return Ok(HashMap::new());
        }
        let filter = Filter::new()
            .kind(Kind::Custom(30000))
            .authors(authors.iter().copied())
            .identifier(mirror_set);
        let events = self.fetch(filter, "fetching follow sets").await?;
        let mut newest: HashMap<PublicKey, Event> = HashMap::new();
        for event in events {
            if !is_follow_set_of(&event, &event.pubkey, mirror_set) {
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
            .pubkeys(targets.iter().copied());
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
    fn send_report(&self, report: EventBuilder) -> impl Future<Output = Result<bool>> + Send;
}

impl ReportRelay for RelayClient {
    fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Vec<Event>> {
        let filter = Filter::new()
            .kind(Kind::Custom(report_kind))
            .author(self.keys.public_key());
        self.fetch(filter, "fetching own replica reports").await
    }

    async fn send_report(&self, report: EventBuilder) -> Result<bool> {
        let event = report
            .finalize(&self.keys)
            .context("signing replica report")?;
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

pub fn print_relay_send_result_lines(results: &[RelaySendResult]) {
    for result in results {
        if result.ok {
            println!("  \u{2713} {}", result.relay);
        } else {
            println!("  \u{2717} {}", result.relay);
        }
    }
}

// NIP-01: for replaceable events the later created_at wins, and on a tie
// the lowest id is kept.
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
) -> Option<FollowSetChoice> {
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
    event.tags.public_keys().collect()
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
    if url.len() > MAX_URL_TAG_BYTES {
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

pub fn parse_site_event(event: &Event, expected_kind: u16) -> Result<SiteEvent> {
    if event.kind != Kind::Custom(expected_kind) {
        anyhow::bail!("unexpected kind {}", event.kind);
    }
    let d = event.tags.identifier().context("missing d tag")?;
    validate_d_tag(&d)?;
    let cid = tag_value(event, "cid")
        .context("missing cid tag")?
        .to_string();
    cid::Cid::try_from(cid.as_str()).context("invalid cid tag")?;
    // A malformed url tag is untrusted input from another party's event, not a
    // reason to drop an otherwise-valid site update; only the url is discarded.
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

pub fn select_latest(events: &[SiteEvent]) -> HashMap<(String, String), SiteEvent> {
    let mut latest: HashMap<(String, String), SiteEvent> = HashMap::new();
    for ev in events {
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

// `expiration` is self-declared like `created_at`, so a report that never
// re-signs must still age out; only re-signing proves the reporter is alive.
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
        self.created_at <= now.saturating_add(crate::policy::MAX_FUTURE_SKEW)
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
        .map(|t| {
            let value = t.content().context("empty cid tag")?;
            cid::Cid::try_from(value).context("invalid cid tag")?;
            Ok(value.to_string())
        })
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

pub fn newest_by_address(events: impl IntoIterator<Item = Event>) -> Vec<Event> {
    let mut newest: HashMap<(PublicKey, Kind, Option<String>), Event> = HashMap::new();
    for event in events {
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

    fn keys() -> Keys {
        Keys::generate()
    }

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
    fn accepts_cidv0() {
        let k = keys();
        let ev = make_site_event(
            &k,
            35980,
            "example.com",
            "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR",
            1000,
        );
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.cid, "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR");
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
        let latest = select_latest(&events);
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

        assert_eq!(choose_follow_set(None, true, None), None);

        let c = choose_follow_set(Some(new.clone()), true, None).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, true, false));

        let c = choose_follow_set(Some(new.clone()), true, Some(old.clone())).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, true, false));

        let c = choose_follow_set(Some(old.clone()), true, Some(new.clone())).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, true));

        let c = choose_follow_set(Some(new.clone()), true, Some(new.clone())).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, false));

        let c = choose_follow_set(None, true, Some(new.clone())).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, true));

        let c = choose_follow_set(None, false, Some(new.clone())).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, false));
    }

    const CID_A: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
    const CID_B: &str = "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR";

    fn report(
        reporter: &Keys,
        author: &PublicKey,
        d: &str,
        cids: &[&str],
        created_at: u64,
    ) -> Event {
        build_replica_report_builder(
            35981,
            35980,
            author,
            d,
            &cids.iter().map(|c| c.to_string()).collect(),
            Timestamp::from_secs(created_at + 100),
        )
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(reporter)
        .unwrap()
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
            BTreeSet::from([CID_A.to_string(), CID_B.to_string()])
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
        let report = report_at(1000 + crate::policy::MAX_FUTURE_SKEW, None);
        assert!(report.counts_at(1000));

        let report = report_at(1000 + crate::policy::MAX_FUTURE_SKEW + 1, None);
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

        let mut ids: Vec<EventId> = newest_by_address(vec![
            new.clone(),
            old,
            other_site.clone(),
            other_reporter.clone(),
        ])
        .into_iter()
        .map(|e| e.id)
        .collect();
        ids.sort();
        let mut expected = vec![new.id, other_site.id, other_reporter.id];
        expected.sort();
        assert_eq!(ids, expected);
    }
}
