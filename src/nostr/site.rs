use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use super::{budget, plausible_at, replaceable_is_newer, tag_value};

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
    pub id: EventId,
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
    if d.chars().any(is_unsafe_char) {
        anyhow::bail!("d tag contains control or invisible formatting characters");
    }
    Ok(())
}

pub fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
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
    !title.is_empty() && title.len() <= MAX_TITLE_TAG_BYTES && !title.chars().any(is_unsafe_char)
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
    let url = tag_value(event, "url")
        .map(|s| s.to_string())
        .filter(|u| valid_http_url(u));
    let size = tag_value(event, "size").and_then(super::parse_decimal);
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
        message: Some(event.content.clone())
            .filter(|m| !m.is_empty() && m.len() <= budget::MAX_CONTENT_BYTES),
        created_at: event.created_at.as_secs(),
        id: event.id,
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
            Some(existing)
                if !replaceable_is_newer(
                    (ev.created_at, ev.id),
                    (existing.created_at, existing.id),
                ) => {}
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

#[cfg(test)]
mod tests {
    use super::super::MAX_FUTURE_SKEW;
    use super::super::fixtures::{make_site_event, site_event_with};
    use super::*;
    use crate::test_support::{CID_A, CID_B, keys, site_event_fixture as site_event};

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
    fn a_size_tag_must_be_plain_decimal_digits() {
        let k = keys();
        for (raw, want) in [
            ("1234", Some(1234)),
            ("+1234", None),
            ("-1", None),
            (" 1234", None),
            ("", None),
            ("1e3", None),
        ] {
            let ev = EventBuilder::new(Kind::Custom(35980), "")
                .tag(Tag::identifier("example.com"))
                .tag(Tag::custom("cid", [CID_A.to_string()]))
                .tag(Tag::custom("size", [raw.to_string()]))
                .finalize(&k)
                .unwrap();
            assert_eq!(parse_site_event(&ev, 35980).unwrap().size, want, "{raw}");
        }
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
                id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
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
                id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
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
                id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
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
                id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
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
                id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
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

    #[test]
    fn rejects_d_tags_and_drops_titles_with_invisible_formatting_characters() {
        let k = keys();
        for c in [
            '\u{00AD}',
            '\u{200B}',
            '\u{200F}',
            '\u{2028}',
            '\u{2029}',
            '\u{202A}',
            '\u{202E}',
            '\u{2066}',
            '\u{2069}',
            '\u{FEFF}',
            '\u{061C}',
            '\u{180E}',
            '\u{2060}',
            '\u{2064}',
            '\u{FFF9}',
            '\u{FFFB}',
            '\u{E0001}',
            '\u{E007F}',
            '\u{0085}',
            '\u{009B}',
        ] {
            let d = format!("exa{c}mple.com");
            assert!(parse_site_event(&site_event_with(&k, &d, "ok", ""), 35980).is_err());
            let title = format!("My{c}Site");
            let parsed =
                parse_site_event(&site_event_with(&k, "example.com", &title, ""), 35980).unwrap();
            assert_eq!(parsed.title, None);
        }
        let parsed = parse_site_event(
            &site_event_with(&k, "例え.example", "サイト — 日記", ""),
            35980,
        )
        .unwrap();
        assert_eq!(parsed.title.as_deref(), Some("サイト — 日記"));
    }

    #[test]
    fn drops_an_oversized_message_but_keeps_the_event() {
        let k = keys();
        let at_limit = "a".repeat(budget::MAX_CONTENT_BYTES);
        let parsed =
            parse_site_event(&site_event_with(&k, "example.com", "t", &at_limit), 35980).unwrap();
        assert_eq!(parsed.message.as_deref(), Some(at_limit.as_str()));

        let over = "a".repeat(budget::MAX_CONTENT_BYTES + 1);
        let parsed =
            parse_site_event(&site_event_with(&k, "example.com", "t", &over), 35980).unwrap();
        assert_eq!(parsed.message, None);
    }

    #[test]
    fn select_latest_breaks_created_at_ties_by_the_lowest_id() {
        let k = keys();
        let a = parse_site_event(
            &make_site_event(&k, 35980, "example.com", CID_A, 500),
            35980,
        )
        .unwrap();
        let b = parse_site_event(
            &make_site_event(&k, 35980, "example.com", CID_B, 500),
            35980,
        )
        .unwrap();
        let expected = if a.id < b.id {
            a.cid.clone()
        } else {
            b.cid.clone()
        };
        for events in [vec![a.clone(), b.clone()], vec![b, a]] {
            let latest = select_latest(&events, 1000);
            assert_eq!(
                latest[&(k.public_key().to_hex(), "example.com".to_string())].cid,
                expected
            );
        }
    }
}
