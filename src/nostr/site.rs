use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use super::{budget, tag_value};

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
    super::newest_per_key(
        events,
        now,
        |ev| (ev.pubkey, ev.d.as_str()),
        |ev| (ev.created_at, ev.id),
    )
    .into_values()
    .map(|ev| ((ev.pubkey.to_hex(), ev.d.clone()), ev.clone()))
    .collect()
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

#[derive(Debug, Clone, Copy, Default)]
pub struct SiteFields<'a> {
    pub d: &'a str,
    pub cid: &'a str,
    pub url: Option<&'a str>,
    pub size: Option<u64>,
    pub title: Option<&'a str>,
    pub message: Option<&'a str>,
}

pub fn build_site_event_builder(site_event_kind: u16, site: &SiteFields<'_>) -> EventBuilder {
    let d = site.d;
    let mut builder = EventBuilder::new(Kind::Custom(site_event_kind), site.message.unwrap_or(""))
        .tag(Tag::identifier(d))
        .tag(Tag::custom("cid", [site.cid.to_string()]));
    if let Some(url) = site.url {
        builder = builder.tag(Tag::custom("url", [url.to_string()]));
    }
    if let Some(size) = site.size {
        builder = builder.tag(Tag::custom("size", [size.to_string()]));
    }
    if let Some(title) = site.title {
        builder = builder.tag(Tag::custom("title", [title.to_string()]));
    }
    builder.tag(Tag::custom(
        "alt",
        [format!("SWING site announcement: {d}")],
    ))
}

const SITE_NOTE_HASHTAG: &str = "swingpublish";

pub fn site_note_content(title: Option<&str>, url: &str, message: Option<&str>) -> String {
    let hashtag = format!("#{SITE_NOTE_HASHTAG}");
    [title, Some(url), message, Some(hashtag.as_str())]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn build_site_note_builder(
    site_event_kind: u16,
    author: &PublicKey,
    d: &str,
    content: &str,
) -> EventBuilder {
    EventBuilder::new(Kind::TextNote, content)
        .tag(Tag::custom(
            "a",
            [super::site_coordinate(site_event_kind, author, d).to_string()],
        ))
        .tag(Tag::hashtag(SITE_NOTE_HASHTAG))
}

#[cfg(test)]
mod tests;
