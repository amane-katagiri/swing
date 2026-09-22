use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::Config;
use crate::nostr::{self, RelayClient};
use crate::replicas::{self, SiteAddress};
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
    now: u64,
) -> (Option<Event>, Option<&'static str>) {
    // `fetch_follow_set` already drops an implausible fetch; filtering `saved`
    // too closes the recovery path for a poisoned copy saved before this check.
    let fetched = fetched.filter(|e| nostr::plausible_at(e.created_at.as_secs(), now));
    let saved = saved.filter(|e| nostr::plausible_at(e.created_at.as_secs(), now));
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

async fn current_follow_set(
    relay: &RelayClient,
    config: &Config,
) -> Result<(Option<Event>, Option<&'static str>)> {
    let fetched = relay.fetch_follow_set(&config.nostr.mirror_set).await?;
    let saved = load_state(config).await?.follow_set.filter(|ev| {
        nostr::is_follow_set_of(ev, &relay.keys.public_key(), &config.nostr.mirror_set)
    });
    Ok(newest_follow_set(
        fetched,
        saved,
        Timestamp::now().as_secs(),
    ))
}

#[derive(Debug, Clone)]
pub struct MirrorListView {
    pub note: Option<&'static str>,
    pub set: Option<MirrorSet>,
}

pub async fn collect_mirror_list(relay: &RelayClient, config: &Config) -> Result<MirrorListView> {
    let (event, note) = current_follow_set(relay, config).await?;
    Ok(MirrorListView {
        note,
        set: event.as_ref().map(MirrorSet::from_event),
    })
}

fn print_mirror_list(config: &Config, view: &MirrorListView) {
    if let Some(note) = view.note {
        println!("{note}");
    }
    match &view.set {
        Some(set) => print_mirror_set(&config.nostr.mirror_set, set),
        None => println!("(no follow set found)"),
    }
}

pub async fn list(config: &Config) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let view = collect_mirror_list(&relay, config).await?;
    relay.client.shutdown().await;
    print_mirror_list(config, &view);
    Ok(())
}

#[derive(Debug, Clone)]
pub struct MirrorChange {
    pub note: Option<&'static str>,
    pub follow_set_found: bool,
    pub changed: Vec<PublicKey>,
    pub unchanged: Vec<PublicKey>,
    pub published: bool,
    pub relay_results: Vec<nostr::RelaySendResult>,
    pub set: MirrorSet,
}

async fn publish_if_changed(
    relay: &RelayClient,
    config: &Config,
    set: &MirrorSet,
    changed: &[PublicKey],
) -> Result<(bool, Vec<nostr::RelaySendResult>)> {
    if changed.is_empty() {
        return Ok((false, Vec::new()));
    }
    let event = set
        .build_event_builder(&config.nostr.mirror_set)
        .finalize(&relay.keys)
        .context("signing mirror set event")?;
    let output = relay.publish_to_relays(&event).await?;
    Ok((true, nostr::relay_send_results(relay.relays(), &output)))
}

enum MirrorOp {
    Add,
    Remove,
}

async fn apply_change(
    relay: &RelayClient,
    config: &Config,
    inputs: &[String],
    op: MirrorOp,
) -> Result<MirrorChange> {
    let keys = parse_pubkey_inputs(inputs)?;
    let (existing, note) = current_follow_set(relay, config).await?;
    let follow_set_found = existing.is_some();
    let mut set = existing
        .as_ref()
        .map(MirrorSet::from_event)
        .unwrap_or_else(MirrorSet::empty);

    let (changed, unchanged) = match op {
        MirrorOp::Add => {
            let current: HashSet<PublicKey> = set.pubkeys().into_iter().collect();
            let unchanged = keys
                .iter()
                .filter(|k| current.contains(k))
                .copied()
                .collect();
            (set.add(&keys), unchanged)
        }
        MirrorOp::Remove => {
            let changed = set.remove(&keys);
            let changed_set: HashSet<PublicKey> = changed.iter().copied().collect();
            let unchanged = keys
                .iter()
                .filter(|k| !changed_set.contains(k))
                .copied()
                .collect();
            (changed, unchanged)
        }
    };
    let (published, relay_results) = publish_if_changed(relay, config, &set, &changed).await?;
    Ok(MirrorChange {
        note,
        follow_set_found,
        changed,
        unchanged,
        published,
        relay_results,
        set,
    })
}

pub async fn apply_add(
    relay: &RelayClient,
    config: &Config,
    inputs: &[String],
) -> Result<MirrorChange> {
    apply_change(relay, config, inputs, MirrorOp::Add).await
}

pub async fn apply_remove(
    relay: &RelayClient,
    config: &Config,
    inputs: &[String],
) -> Result<MirrorChange> {
    apply_change(relay, config, inputs, MirrorOp::Remove).await
}

fn print_publish_block(config: &Config, change: &MirrorChange) {
    println!();
    println!("Nostr");
    nostr::print_relay_send_result_lines(&change.relay_results);
    println!();
    print_mirror_set(&config.nostr.mirror_set, &change.set);
}

fn print_change_result(
    config: &Config,
    change: &MirrorChange,
    unchanged_label: &str,
    changed_label: &str,
    requires_follow_set: bool,
) {
    if requires_follow_set && !change.follow_set_found {
        println!("(no follow set found); no changes");
        return;
    }
    if let Some(note) = change.note {
        println!("{note}");
    }
    for pk in &change.unchanged {
        println!("{unchanged_label}: {} ({})", npub(pk), pk.to_hex());
    }
    if change.changed.is_empty() {
        println!("no changes; not publishing");
        return;
    }
    for pk in &change.changed {
        println!("{changed_label}: {} ({})", npub(pk), pk.to_hex());
    }
    print_publish_block(config, change);
}

fn print_add_result(config: &Config, change: &MirrorChange) {
    print_change_result(config, change, "already in mirror set", "added", false);
}

pub async fn add(config: &Config, inputs: &[String]) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let change = apply_add(&relay, config, inputs).await?;
    relay.client.shutdown().await;
    print_add_result(config, &change);
    Ok(())
}

fn print_remove_result(config: &Config, change: &MirrorChange) {
    print_change_result(config, change, "not in mirror set", "removed", true);
}

pub async fn remove(config: &Config, inputs: &[String]) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let change = apply_remove(&relay, config, inputs).await?;
    relay.client.shutdown().await;
    print_remove_result(config, &change);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteRow {
    pub d: String,
    pub cid: String,
    pub url: Option<String>,
    pub size: Option<u64>,
    pub created_at: u64,
    pub title: Option<String>,
    pub message: Option<String>,
    pub nip05: Option<String>,
    pub replicas: Option<usize>,
    pub stored: bool,
}

fn format_site_line(row: &SiteRow, status: &str) -> String {
    format!(
        "  d={:<24} cid={:<62} url={:<32} size={:<12} created_at={:<25} nip05={:<14} replicas={:<4} [{}]",
        row.d,
        row.cid,
        row.url.clone().unwrap_or_else(|| "-".to_string()),
        row.size
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string()),
        format_unix_timestamp(row.created_at),
        row.nip05.as_deref().unwrap_or("-"),
        row.replicas
            .map_or_else(|| "-".to_string(), |n| n.to_string()),
        status
    )
}

const MAX_MESSAGE_DISPLAY_CHARS: usize = 200;

fn sanitize_display_text(text: &str, max_chars: usize) -> String {
    let mut shown: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max_chars)
        .collect();
    if text.chars().count() > max_chars {
        shown.push('\u{2026}');
    }
    shown.trim().to_string()
}

fn format_title_line(title: &str) -> String {
    format!(
        "    title: {}",
        sanitize_display_text(title, MAX_MESSAGE_DISPLAY_CHARS)
    )
}

fn format_message_line(message: &str) -> String {
    format!(
        "    message: {}",
        sanitize_display_text(message, MAX_MESSAGE_DISPLAY_CHARS)
    )
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

fn print_account_header(pubkey_hex: &str, suffix: &str) -> Result<PublicKey> {
    let pk = PublicKey::from_hex(pubkey_hex).context("parsing pubkey")?;
    println!("{} ({}){suffix}", npub(&pk), pubkey_hex);
    Ok(pk)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSites {
    pub pubkey: PublicKey,
    pub sites: Vec<SiteRow>,
}

#[derive(Debug, Clone)]
pub struct SitesView {
    pub follow_set_found: bool,
    pub follow_note: Option<&'static str>,
    pub accounts: Vec<AccountSites>,
    pub replicas_error: Option<String>,
    pub remove_on_unfollow: bool,
    pub unfollowed: Vec<AccountSites>,
}

pub async fn collect_sites(relay: &RelayClient, config: &Config) -> Result<SitesView> {
    let (follow_event, follow_note) = current_follow_set(relay, config).await?;
    let follow_set_found = follow_event.is_some();
    let targets: Vec<PublicKey> = follow_event
        .as_ref()
        .map(nostr::extract_follow_set_pubkeys)
        .unwrap_or_default();

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
        nostr::select_latest(&parsed, Timestamp::now().as_secs())
    };
    let latest_sites: Vec<&nostr::SiteEvent> = latest.values().collect();
    let (reports, replicas_error) =
        match replicas::fetch_for_sites(relay, config, &latest_sites).await {
            Ok(reports) => (Some(reports), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };

    let state = load_state(config).await?;

    let mut by_pubkey: BTreeMap<String, Vec<&nostr::SiteEvent>> = BTreeMap::new();
    for pk in &targets {
        by_pubkey.entry(pk.to_hex()).or_default();
    }
    for ev in latest.values() {
        by_pubkey.entry(ev.pubkey.to_hex()).or_default().push(ev);
    }

    let mut accounts = Vec::with_capacity(by_pubkey.len());
    for (pubkey_hex, mut evs) in by_pubkey {
        evs.sort_by(|a, b| a.d.cmp(&b.d));
        let pubkey = PublicKey::from_hex(&pubkey_hex).context("parsing pubkey")?;
        let sites = evs
            .into_iter()
            .map(|ev| {
                let key = state::site_key(&pubkey_hex, &ev.d);
                let stored = state
                    .sites
                    .get(&key)
                    .map(|versions| versions.iter().any(|v| v.cid == ev.cid))
                    .unwrap_or(false);
                let nip05 = state.verifications.get(&key).map(|v| v.status.clone());
                let replicas = replica_count(&reports, ev);
                SiteRow {
                    d: ev.d.clone(),
                    cid: ev.cid.clone(),
                    url: ev.url.clone(),
                    size: ev.size,
                    created_at: ev.created_at,
                    title: ev.title.clone(),
                    message: ev.message.clone(),
                    nip05,
                    replicas,
                    stored,
                }
            })
            .collect();
        accounts.push(AccountSites { pubkey, sites });
    }

    let target_hex: BTreeSet<String> = targets.iter().map(|pk| pk.to_hex()).collect();
    let unfollowed_map = unfollowed_sites(&state, &target_hex);
    let mut unfollowed = Vec::with_capacity(unfollowed_map.len());
    for (pubkey_hex, sites) in unfollowed_map {
        let pubkey = PublicKey::from_hex(&pubkey_hex).context("parsing pubkey")?;
        let sites = sites
            .into_iter()
            .map(|(d, version)| {
                let key = state::site_key(&pubkey_hex, &d);
                let nip05 = state.verifications.get(&key).map(|v| v.status.clone());
                SiteRow {
                    d,
                    cid: version.cid,
                    url: None,
                    size: Some(version.size),
                    created_at: version.created_at,
                    title: None,
                    message: None,
                    nip05,
                    replicas: None,
                    stored: true,
                }
            })
            .collect();
        unfollowed.push(AccountSites { pubkey, sites });
    }

    Ok(SitesView {
        follow_set_found,
        follow_note,
        accounts,
        replicas_error,
        remove_on_unfollow: config.policy.remove_on_unfollow,
        unfollowed,
    })
}

fn replica_count(
    reports: &Option<std::collections::HashMap<SiteAddress, Vec<nostr::ReplicaReport>>>,
    ev: &nostr::SiteEvent,
) -> Option<usize> {
    reports.as_ref().map(|reports| {
        reports.get(&(ev.pubkey, ev.d.clone())).map_or(0, |r| {
            replicas::latest_count(&replicas::replicas_of(r, &ev.cid))
        })
    })
}

fn print_sites(view: &SitesView) -> Result<()> {
    if let Some(note) = view.follow_note {
        println!("{note}");
    }
    if !view.follow_set_found {
        println!("(no follow set found)");
    } else if view.accounts.is_empty() {
        println!("(follow set is empty)");
    }
    if let Some(err) = &view.replicas_error {
        println!("(fetching replica reports failed: {err})");
    }

    for account in &view.accounts {
        print_account_header(&account.pubkey.to_hex(), "")?;
        if account.sites.is_empty() {
            println!("  (no site events)");
            continue;
        }
        for site in &account.sites {
            let status = if site.stored { "stored" } else { "not stored" };
            println!("{}", format_site_line(site, status));
            if let Some(title) = &site.title {
                println!("{}", format_title_line(title));
            }
            if let Some(message) = &site.message {
                println!("{}", format_message_line(message));
            }
        }
    }

    if view.unfollowed.is_empty() {
        return Ok(());
    }
    println!();
    if view.remove_on_unfollow {
        println!("Unfollowed but still stored (the agent removes them on its next poll):");
    } else {
        println!(
            "Unfollowed but still stored (kept because remove_on_unfollow is false; set it to true to remove them):"
        );
    }
    for account in &view.unfollowed {
        print_account_header(&account.pubkey.to_hex(), " [unfollowed]")?;
        for site in &account.sites {
            println!("{}", format_site_line(site, "unfollowed"));
        }
    }
    Ok(())
}

pub async fn sites(config: &Config) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let view = collect_sites(&relay, config).await?;
    relay.client.shutdown().await;
    print_sites(&view)
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

        let (ev, note) = newest_follow_set(Some(old.clone()), Some(new.clone()), 1000);
        assert_eq!(ev.unwrap().id, new.id);
        assert!(note.is_some());

        let (ev, note) = newest_follow_set(Some(new.clone()), Some(old.clone()), 1000);
        assert_eq!(ev.unwrap().id, new.id);
        assert!(note.is_none());

        let (ev, note) = newest_follow_set(None, Some(old.clone()), 1000);
        assert_eq!(ev.unwrap().id, old.id);
        assert!(note.is_some());

        let (ev, note) = newest_follow_set(Some(old.clone()), None, 1000);
        assert_eq!(ev.unwrap().id, old.id);
        assert!(note.is_none());

        assert_eq!(newest_follow_set(None, None, 1000), (None, None));
    }

    #[test]
    fn newest_follow_set_drops_a_poisoned_saved_copy() {
        let k = keys();
        let poisoned_saved = signed_follow_set(&k, 1000 + nostr::MAX_FUTURE_SKEW + 1);
        let fetched = signed_follow_set(&k, 100);

        let (ev, note) = newest_follow_set(Some(fetched.clone()), Some(poisoned_saved), 1000);
        assert_eq!(ev.unwrap().id, fetched.id);
        assert!(note.is_none());

        let poisoned_saved_only = signed_follow_set(&k, 1000 + nostr::MAX_FUTURE_SKEW + 1);
        assert_eq!(
            newest_follow_set(None, Some(poisoned_saved_only), 1000),
            (None, None)
        );
    }
}
