use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::Config;
use crate::nostr::{self, RelayClient};
use crate::replicas;
use crate::state::{self, State, VersionRecord};

const FOLLOW_SET_KIND: u16 = 30000;
const DEFAULT_TITLE: &str = "SWING mirror list";

pub fn parse_pubkey_input(input: &str) -> Result<PublicKey> {
    PublicKey::parse(input.trim()).with_context(|| format!("invalid pubkey: {input}"))
}

pub fn npub(pk: &PublicKey) -> String {
    pk.to_bech32()
        .expect("bech32 encoding of a public key cannot fail")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorSet {
    other_tags: Vec<Tag>,
    p_tags: Vec<Tag>,
    content: String,
}

impl MirrorSet {
    pub fn empty() -> Self {
        Self {
            other_tags: vec![Tag::custom("title", [DEFAULT_TITLE.to_string()])],
            p_tags: Vec::new(),
            content: String::new(),
        }
    }

    pub fn from_event(event: &Event) -> Self {
        let mut other_tags = Vec::new();
        let mut p_tags = Vec::new();
        for tag in event.tags.iter() {
            match tag.kind() {
                "p" => p_tags.push(tag.clone()),
                "d" => {}
                _ => other_tags.push(tag.clone()),
            }
        }
        Self {
            other_tags,
            p_tags,
            content: event.content.clone(),
        }
    }

    pub fn pubkeys(&self) -> Vec<PublicKey> {
        self.p_tags
            .iter()
            .filter_map(|t| t.content().and_then(|s| PublicKey::parse(s).ok()))
            .collect()
    }

    pub fn title(&self) -> Option<&str> {
        self.other_tags
            .iter()
            .find(|t| t.kind() == "title")
            .and_then(|t| t.content())
    }

    pub fn add(&mut self, keys: &[PublicKey]) -> Vec<PublicKey> {
        let mut present: HashSet<PublicKey> = self.pubkeys().into_iter().collect();
        let mut added = Vec::new();
        for &key in keys {
            if present.insert(key) {
                self.p_tags.push(Tag::public_key(key));
                added.push(key);
            }
        }
        added
    }

    pub fn remove(&mut self, keys: &[PublicKey]) -> Vec<PublicKey> {
        let to_remove: HashSet<PublicKey> = keys.iter().copied().collect();
        let mut removed = Vec::new();
        self.p_tags.retain(
            |t| match t.content().and_then(|s| PublicKey::parse(s).ok()) {
                Some(pk) if to_remove.contains(&pk) => {
                    removed.push(pk);
                    false
                }
                _ => true,
            },
        );
        removed
    }

    pub fn build_event_builder(&self, mirror_set: &str) -> EventBuilder {
        EventBuilder::new(Kind::Custom(FOLLOW_SET_KIND), self.content.clone())
            .tag(Tag::identifier(mirror_set))
            .tags(self.other_tags.clone())
            .tags(self.p_tags.clone())
    }
}

fn dedupe_preserve_order(keys: Vec<PublicKey>) -> Vec<PublicKey> {
    let mut seen = HashSet::new();
    keys.into_iter().filter(|k| seen.insert(*k)).collect()
}

pub fn parse_pubkey_inputs(inputs: &[String]) -> Result<Vec<PublicKey>> {
    let parsed: Result<Vec<PublicKey>> = inputs.iter().map(|s| parse_pubkey_input(s)).collect();
    Ok(dedupe_preserve_order(parsed?))
}

fn print_mirror_set(mirror_set_name: &str, set: &MirrorSet) {
    println!("Mirror set: {mirror_set_name} (kind {FOLLOW_SET_KIND})");
    if let Some(title) = set.title() {
        println!("Title: {title}");
    }
    let pubkeys = set.pubkeys();
    println!("{} pubkey(s):", pubkeys.len());
    for pk in pubkeys {
        println!("  {}  {}", npub(&pk), pk.to_hex());
    }
}

async fn load_state(config: &Config) -> Result<State> {
    let state_path = config.agent.state_dir.join("state.json");
    if state_path.exists() {
        State::load(&state_path).await
    } else {
        Ok(State::default())
    }
}

fn newest_follow_set(
    fetched: Option<Event>,
    saved: Option<Event>,
) -> (Option<Event>, Option<&'static str>) {
    match (fetched, saved) {
        (Some(fetched), Some(saved)) if nostr::is_newer_replaceable(&saved, &fetched) => (
            Some(saved),
            Some("(relays returned an older follow set; using the newer one saved by the agent)"),
        ),
        (None, Some(saved)) => (
            Some(saved),
            Some("(follow set not found on relays; using the one saved by the agent)"),
        ),
        (fetched, _) => (fetched, None),
    }
}

async fn current_follow_set(relay: &RelayClient, config: &Config) -> Result<Option<Event>> {
    let fetched = relay.fetch_follow_set(&config.nostr.mirror_set).await?;
    let saved = load_state(config).await?.follow_set.filter(|ev| {
        nostr::is_follow_set_of(ev, &relay.keys.public_key(), &config.nostr.mirror_set)
    });
    let (event, note) = newest_follow_set(fetched, saved);
    if let Some(note) = note {
        println!("{note}");
    }
    Ok(event)
}

pub async fn list(config: &Config) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let event = current_follow_set(&relay, config).await?;
    match event {
        Some(ev) => print_mirror_set(&config.nostr.mirror_set, &MirrorSet::from_event(&ev)),
        None => println!("(no follow set found)"),
    }
    relay.client.shutdown().await;
    Ok(())
}

async fn publish_mirror_set(relay: &RelayClient, config: &Config, set: &MirrorSet) -> Result<()> {
    let event = set
        .build_event_builder(&config.nostr.mirror_set)
        .finalize(&relay.keys)
        .context("signing mirror set event")?;
    let output = relay.publish_to_relays(&event).await?;
    println!();
    println!("Nostr");
    nostr::print_relay_send_results(relay.relays(), &output);
    println!();
    print_mirror_set(&config.nostr.mirror_set, set);
    Ok(())
}

pub async fn add(config: &Config, inputs: &[String]) -> Result<()> {
    let keys = parse_pubkey_inputs(inputs)?;
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let existing = current_follow_set(&relay, config).await?;
    let mut set = existing
        .as_ref()
        .map(MirrorSet::from_event)
        .unwrap_or_else(MirrorSet::empty);

    let already_present: Vec<PublicKey> = {
        let current: HashSet<PublicKey> = set.pubkeys().into_iter().collect();
        keys.iter()
            .filter(|k| current.contains(k))
            .copied()
            .collect()
    };
    for pk in &already_present {
        println!("already in mirror set: {} ({})", npub(pk), pk.to_hex());
    }

    let added = set.add(&keys);
    if added.is_empty() {
        println!("no changes; not publishing");
        relay.client.shutdown().await;
        return Ok(());
    }
    for pk in &added {
        println!("added: {} ({})", npub(pk), pk.to_hex());
    }

    publish_mirror_set(&relay, config, &set).await?;
    relay.client.shutdown().await;
    Ok(())
}

pub async fn remove(config: &Config, inputs: &[String]) -> Result<()> {
    let keys = parse_pubkey_inputs(inputs)?;
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let existing = current_follow_set(&relay, config).await?;
    let mut set = match existing {
        Some(ev) => MirrorSet::from_event(&ev),
        None => {
            println!("(no follow set found); no changes");
            relay.client.shutdown().await;
            return Ok(());
        }
    };

    let removed = set.remove(&keys);
    let removed_set: HashSet<PublicKey> = removed.iter().copied().collect();
    for pk in &keys {
        if !removed_set.contains(pk) {
            println!("not in mirror set: {} ({})", npub(pk), pk.to_hex());
        }
    }
    if removed.is_empty() {
        println!("no changes; not publishing");
        relay.client.shutdown().await;
        return Ok(());
    }
    for pk in &removed {
        println!("removed: {} ({})", npub(pk), pk.to_hex());
    }

    publish_mirror_set(&relay, config, &set).await?;
    relay.client.shutdown().await;
    Ok(())
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// nostr_sdk 0.45's Timestamp has no human-readable formatter (only Display,
// which prints raw seconds), so this hand-rolls UTC civil-date math instead
// of pulling in a chrono-sized dependency for one display line.
fn format_unix_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
}

fn format_site_line(
    ev: &nostr::SiteEvent,
    status: &str,
    verification: Option<&str>,
    replicas: Option<usize>,
) -> String {
    format!(
        "  d={:<24} cid={:<62} url={:<32} size={:<12} created_at={:<25} nip05={:<14} replicas={:<4} [{}]",
        ev.d,
        ev.cid,
        ev.url.clone().unwrap_or_else(|| "-".to_string()),
        ev.size
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string()),
        format_unix_timestamp(ev.created_at),
        verification.unwrap_or("-"),
        replicas.map_or_else(|| "-".to_string(), |n| n.to_string()),
        status
    )
}

const MAX_MESSAGE_DISPLAY_CHARS: usize = 200;

fn format_message_line(message: &str) -> String {
    let mut shown: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_MESSAGE_DISPLAY_CHARS)
        .collect();
    if message.chars().count() > MAX_MESSAGE_DISPLAY_CHARS {
        shown.push('\u{2026}');
    }
    format!("    message: {}", shown.trim())
}

fn unfollowed_sites(
    state: &State,
    targets: &BTreeSet<String>,
) -> BTreeMap<String, Vec<(String, VersionRecord)>> {
    let mut out: BTreeMap<String, Vec<(String, VersionRecord)>> = BTreeMap::new();
    for (key, versions) in &state.sites {
        let Some((pubkey_hex, d)) = state::split_site_key(key) else {
            continue;
        };
        if targets.contains(pubkey_hex) {
            continue;
        }
        if let Some(latest) = versions.iter().max_by_key(|v| v.created_at) {
            out.entry(pubkey_hex.to_string())
                .or_default()
                .push((d.to_string(), latest.clone()));
        }
    }
    out
}

pub fn print_account_header(pubkey_hex: &str, suffix: &str) -> Result<PublicKey> {
    let pk = PublicKey::from_hex(pubkey_hex).context("parsing pubkey")?;
    println!("{} ({}){suffix}", npub(&pk), pubkey_hex);
    Ok(pk)
}

pub async fn sites(config: &Config) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let follow_event = current_follow_set(&relay, config).await?;
    let targets = match &follow_event {
        Some(event) => nostr::extract_follow_set_pubkeys(event),
        None => {
            println!("(no follow set found)");
            Vec::new()
        }
    };
    if follow_event.is_some() && targets.is_empty() {
        println!("(follow set is empty)");
    }

    let latest = if targets.is_empty() {
        Default::default()
    } else {
        let raw_events = relay
            .fetch_site_events(config.nostr.site_event_kind, &targets)
            .await?;
        let parsed: Vec<nostr::SiteEvent> = raw_events
            .iter()
            .filter_map(|e| nostr::parse_site_event(e, config.nostr.site_event_kind).ok())
            .collect();
        nostr::select_latest(&parsed)
    };
    let latest_sites: Vec<&nostr::SiteEvent> = latest.values().collect();
    let reports = match replicas::fetch_for_sites(&relay, config, &latest_sites).await {
        Ok(reports) => Some(reports),
        Err(e) => {
            println!("(fetching replica reports failed: {e:#})");
            None
        }
    };
    relay.client.shutdown().await;

    let state = load_state(config).await?;

    let mut by_pubkey: BTreeMap<String, Vec<&nostr::SiteEvent>> = BTreeMap::new();
    for pk in &targets {
        by_pubkey.entry(pk.to_hex()).or_default();
    }
    for ev in latest.values() {
        by_pubkey.entry(ev.pubkey.to_hex()).or_default().push(ev);
    }

    for (pubkey_hex, mut evs) in by_pubkey {
        evs.sort_by(|a, b| a.d.cmp(&b.d));
        print_account_header(&pubkey_hex, "")?;
        if evs.is_empty() {
            println!("  (no site events)");
            continue;
        }
        for ev in evs {
            let key = state::site_key(&pubkey_hex, &ev.d);
            let stored = state
                .sites
                .get(&key)
                .map(|versions| versions.iter().any(|v| v.cid == ev.cid))
                .unwrap_or(false);
            let verification = state.verifications.get(&key).map(|v| v.status.as_str());
            let status = if stored { "stored" } else { "not stored" };
            let replica_count = reports.as_ref().map(|reports| {
                reports.get(&(ev.pubkey, ev.d.clone())).map_or(0, |r| {
                    replicas::latest_count(&replicas::replicas_of(r, &ev.cid))
                })
            });
            println!(
                "{}",
                format_site_line(ev, status, verification, replica_count)
            );
            if let Some(message) = &ev.message {
                println!("{}", format_message_line(message));
            }
        }
    }

    let target_hex: BTreeSet<String> = targets.iter().map(|pk| pk.to_hex()).collect();
    let unfollowed = unfollowed_sites(&state, &target_hex);
    if unfollowed.is_empty() {
        return Ok(());
    }
    println!();
    if config.policy.remove_on_unfollow {
        println!("Unfollowed but still stored (the agent removes them on its next poll):");
    } else {
        println!(
            "Unfollowed but still stored (kept because remove_on_unfollow is false; set it to true to remove them):"
        );
    }
    for (pubkey_hex, sites) in unfollowed {
        let pk = print_account_header(&pubkey_hex, " [unfollowed]")?;
        for (d, version) in sites {
            let ev = nostr::SiteEvent {
                pubkey: pk,
                d: d.clone(),
                cid: version.cid,
                url: None,
                size: Some(version.size),
                message: None,
                created_at: version.created_at,
            };
            let key = state::site_key(&pubkey_hex, &d);
            let verification = state.verifications.get(&key).map(|v| v.status.as_str());
            println!(
                "{}",
                format_site_line(&ev, "unfollowed", verification, None)
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        Keys::generate()
    }

    #[test]
    fn message_line_neutralizes_control_chars_and_truncates() {
        assert_eq!(
            format_message_line("Add posts\n\u{1b}[31mred"),
            "    message: Add posts  [31mred"
        );
        let long = "あ".repeat(MAX_MESSAGE_DISPLAY_CHARS + 1);
        let line = format_message_line(&long);
        assert!(line.ends_with("あ\u{2026}"));
        assert_eq!(
            line.chars().count(),
            "    message: ".len() + MAX_MESSAGE_DISPLAY_CHARS + 1
        );
    }

    #[test]
    fn format_unix_timestamp_known_values() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_unix_timestamp(1_000_000), "1970-01-12 13:46:40 UTC");
        assert_eq!(
            format_unix_timestamp(1_700_000_000),
            "2023-11-14 22:13:20 UTC"
        );
    }

    #[test]
    fn parses_hex_npub_and_nprofile() {
        let k = keys();
        let hex = k.public_key().to_hex();
        let npub_str = k.public_key().to_bech32().unwrap();
        let nprofile_str = Nip19Profile::new(k.public_key(), Vec::<RelayUrl>::new())
            .to_bech32()
            .unwrap();

        assert_eq!(parse_pubkey_input(&hex).unwrap(), k.public_key());
        assert_eq!(parse_pubkey_input(&npub_str).unwrap(), k.public_key());
        assert_eq!(parse_pubkey_input(&nprofile_str).unwrap(), k.public_key());
    }

    #[test]
    fn parse_pubkey_input_rejects_garbage() {
        assert!(parse_pubkey_input("not-a-key").is_err());
        assert!(parse_pubkey_input("").is_err());
    }

    #[test]
    fn parse_pubkey_inputs_dedupes_preserving_order() {
        let k1 = keys().public_key();
        let k2 = keys().public_key();
        let inputs = vec![k1.to_hex(), k2.to_hex(), k1.to_hex()];
        let parsed = parse_pubkey_inputs(&inputs).unwrap();
        assert_eq!(parsed, vec![k1, k2]);
    }

    fn make_follow_set(
        author: &Keys,
        d: &str,
        title: Option<&str>,
        content: &str,
        p: &[PublicKey],
    ) -> Event {
        let mut builder =
            EventBuilder::new(Kind::Custom(FOLLOW_SET_KIND), content).tag(Tag::identifier(d));
        if let Some(title) = title {
            builder = builder.tag(Tag::custom("title", [title.to_string()]));
        }
        for pk in p {
            builder = builder.tag(Tag::public_key(*pk));
        }
        builder.finalize(author).unwrap()
    }

    #[test]
    fn from_event_preserves_other_tags_and_content() {
        let author = keys();
        let p1 = keys().public_key();
        let ev = make_follow_set(&author, "swing", Some("my list"), "encrypted-blob", &[p1]);
        let set = MirrorSet::from_event(&ev);
        assert_eq!(set.content, "encrypted-blob");
        assert_eq!(set.title(), Some("my list"));
        assert_eq!(set.pubkeys(), vec![p1]);
    }

    #[test]
    fn add_is_idempotent_and_reports_no_new_keys() {
        let author = keys();
        let p1 = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        let added = set.add(&[p1]);
        assert!(added.is_empty());
        assert_eq!(set.pubkeys(), vec![p1]);
    }

    #[test]
    fn add_appends_new_keys_keeping_existing() {
        let author = keys();
        let p1 = keys().public_key();
        let p2 = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        let added = set.add(&[p1, p2]);
        assert_eq!(added, vec![p2]);
        let pubkeys = set.pubkeys();
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&p1));
        assert!(pubkeys.contains(&p2));
    }

    #[test]
    fn remove_drops_matching_keys_and_reports_absent() {
        let author = keys();
        let p1 = keys().public_key();
        let p2 = keys().public_key();
        let absent = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1, p2]);
        let mut set = MirrorSet::from_event(&ev);
        let removed = set.remove(&[p1, absent]);
        assert_eq!(removed, vec![p1]);
        assert_eq!(set.pubkeys(), vec![p2]);
    }

    #[test]
    fn remove_absent_key_is_no_op() {
        let author = keys();
        let p1 = keys().public_key();
        let absent = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        let removed = set.remove(&[absent]);
        assert!(removed.is_empty());
        assert_eq!(set.pubkeys(), vec![p1]);
    }

    #[test]
    fn build_event_builder_keeps_content_and_non_p_tags_byte_for_byte() {
        let author = keys();
        let p1 = keys().public_key();
        let p2 = keys().public_key();
        let ev = make_follow_set(&author, "swing", Some("my list"), "private-content", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        set.add(&[p2]);
        let rebuilt = set.build_event_builder("swing").finalize(&author).unwrap();

        assert_eq!(rebuilt.content, "private-content");
        assert_eq!(rebuilt.tags.identifier().as_deref(), Some("swing"));
        assert_eq!(
            rebuilt
                .tags
                .iter()
                .find(|t| t.kind() == "title")
                .and_then(|t| t.content()),
            Some("my list")
        );
        let pubkeys: Vec<PublicKey> = rebuilt.tags.public_keys().collect();
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&p1));
        assert!(pubkeys.contains(&p2));
    }

    #[test]
    fn empty_mirror_set_has_default_title_and_no_pubkeys() {
        let set = MirrorSet::empty();
        assert_eq!(set.title(), Some(DEFAULT_TITLE));
        assert!(set.pubkeys().is_empty());
    }

    #[test]
    fn unfollowed_sites_lists_the_latest_version_of_accounts_outside_the_follow_set() {
        let mut state = State::default();
        let record = |cid: &str, created_at| VersionRecord {
            cid: cid.into(),
            size: 1,
            created_at,
            stored_at: created_at,
        };
        state.apply_store(&state::site_key("aa", "x.example"), record("old", 1));
        state.apply_store(&state::site_key("aa", "x.example"), record("new", 2));
        state.apply_store(&state::site_key("aa", "y:z"), record("yz", 1));
        state.apply_store(&state::site_key("bb", "x.example"), record("kept", 1));
        let targets = BTreeSet::from(["bb".to_string()]);

        let unfollowed = unfollowed_sites(&state, &targets);

        assert_eq!(unfollowed.keys().collect::<Vec<_>>(), vec!["aa"]);
        let sites: Vec<(&str, &str)> = unfollowed["aa"]
            .iter()
            .map(|(d, v)| (d.as_str(), v.cid.as_str()))
            .collect();
        assert_eq!(sites, vec![("x.example", "new"), ("y:z", "yz")]);
    }

    fn signed_follow_set(keys: &Keys, created_at: u64) -> Event {
        MirrorSet::empty()
            .build_event_builder("swing")
            .custom_created_at(Timestamp::from_secs(created_at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn newest_follow_set_prefers_a_newer_saved_copy() {
        let k = keys();
        let old = signed_follow_set(&k, 100);
        let new = signed_follow_set(&k, 200);

        let (ev, note) = newest_follow_set(Some(old.clone()), Some(new.clone()));
        assert_eq!(ev.unwrap().id, new.id);
        assert!(note.is_some());

        let (ev, note) = newest_follow_set(Some(new.clone()), Some(old.clone()));
        assert_eq!(ev.unwrap().id, new.id);
        assert!(note.is_none());

        let (ev, note) = newest_follow_set(None, Some(old.clone()));
        assert_eq!(ev.unwrap().id, old.id);
        assert!(note.is_some());

        let (ev, note) = newest_follow_set(Some(old.clone()), None);
        assert_eq!(ev.unwrap().id, old.id);
        assert!(note.is_none());

        assert_eq!(newest_follow_set(None, None), (None, None));
    }
}
