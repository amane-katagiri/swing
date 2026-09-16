use std::collections::HashMap;
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

    pub async fn fetch_follow_set(&self, mirror_set: &str) -> Result<Option<Event>> {
        let filter = Filter::new()
            .kind(Kind::Custom(30000))
            .author(self.keys.public_key())
            .identifier(mirror_set);
        let events = self
            .client
            .fetch_events(filter)
            .timeout(Duration::from_secs(30))
            .await
            .context("fetching follow set")?;
        // Defense in depth against a relay that ignores the author filter.
        Ok(events
            .into_iter()
            .filter(|e| e.pubkey == self.keys.public_key())
            .max_by_key(|e| e.created_at))
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
        let events = self
            .client
            .fetch_events(filter)
            .timeout(Duration::from_secs(30))
            .await
            .context("fetching site events")?;
        Ok(events.into_iter().collect())
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

pub fn print_relay_send_results(
    relays: &[String],
    output: &Output<EventId, EventSendStatus, String>,
) {
    for relay_url in relays {
        let sent_ok = RelayUrl::parse(relay_url)
            .map(|u| output.success.contains_key(&u))
            .unwrap_or(false);
        if sent_ok {
            println!("  \u{2713} {relay_url}");
        } else {
            println!("  \u{2717} {relay_url}");
        }
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

fn validate_d_tag(d: &str) -> Result<()> {
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

fn valid_http_url(url: &str) -> bool {
    if url.len() > MAX_URL_TAG_BYTES {
        return false;
    }
    match reqwest::Url::parse(url) {
        Ok(parsed) => parsed.scheme() == "http" || parsed.scheme() == "https",
        Err(_) => false,
    }
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
    Ok(SiteEvent {
        pubkey: event.pubkey,
        d,
        cid,
        url,
        size,
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

pub fn build_site_event_builder(
    site_event_kind: u16,
    d: &str,
    cid: &str,
    url: Option<&str>,
    size: Option<u64>,
) -> EventBuilder {
    let mut builder = EventBuilder::new(Kind::Custom(site_event_kind), "")
        .tag(Tag::identifier(d))
        .tag(Tag::custom("cid", [cid.to_string()]));
    if let Some(url) = url {
        builder = builder.tag(Tag::custom("url", [url.to_string()]));
    }
    if let Some(size) = size {
        builder = builder.tag(Tag::custom("size", [size.to_string()]));
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        Keys::generate()
    }

    fn make_site_event(keys: &Keys, kind: u16, d: &str, cid: &str, created_at: u64) -> Event {
        build_site_event_builder(kind, d, cid, Some("https://example.com/"), Some(1234))
            .custom_created_at(Timestamp::from_secs(created_at))
            .finalize(keys)
            .unwrap()
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
                created_at: 100,
            },
            SiteEvent {
                pubkey: k1.public_key(),
                d: "site-a".into(),
                cid: "bafy-new".into(),
                url: None,
                size: None,
                created_at: 200,
            },
            SiteEvent {
                pubkey: k1.public_key(),
                d: "site-b".into(),
                cid: "bafy-other-site".into(),
                url: None,
                size: None,
                created_at: 50,
            },
            SiteEvent {
                pubkey: k2.public_key(),
                d: "site-a".into(),
                cid: "bafy-k2".into(),
                url: None,
                size: None,
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
}
