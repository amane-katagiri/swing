use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use serde::Serialize;
use tracing::warn;

use crate::api_client::ApiClient;
use crate::config::Config;
use crate::dashboard::dto;
use crate::nostr::{self, RelayClient};
use crate::replicas::{self, SiteAddress};
use crate::signer::Signer;
use crate::state::{self, State, VersionRecord};

mod print;
mod set;
mod time;

pub use set::MirrorSet;

pub fn parse_pubkey_input(input: &str) -> Result<PublicKey> {
    PublicKey::parse(input.trim()).with_context(|| format!("invalid pubkey: {input}"))
}

pub fn npub(pk: &PublicKey) -> String {
    pk.to_bech32()
        .expect("bech32 encoding of a public key cannot fail")
}

fn dedupe_preserve_order(keys: Vec<PublicKey>) -> Vec<PublicKey> {
    let mut seen = HashSet::new();
    keys.into_iter().filter(|k| seen.insert(*k)).collect()
}

pub fn parse_pubkey_inputs(inputs: &[String]) -> Result<Vec<PublicKey>> {
    let parsed: Result<Vec<PublicKey>> = inputs.iter().map(|s| parse_pubkey_input(s)).collect();
    Ok(dedupe_preserve_order(parsed?))
}

pub async fn load_state(config: &Config) -> Result<State> {
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
    let fetched_was_none = fetched.is_none();
    match nostr::choose_follow_set(fetched, true, saved, now) {
        None => (None, None),
        Some(choice) => {
            let note = if fetched_was_none {
                Some("(follow set not found on relays; using the one saved by the agent)")
            } else if choice.republish {
                Some(
                    "(relays returned an older follow set; using the newer one saved by the agent)",
                )
            } else {
                None
            };
            (Some(choice.event), note)
        }
    }
}

async fn current_follow_set(
    relay: &RelayClient,
    config: &Config,
) -> Result<(Option<Event>, Option<&'static str>)> {
    let fetched = relay.fetch_follow_set(&config.nostr.mirror_set).await?;
    let saved = load_state(config).await?.follow_set.filter(|ev| {
        nostr::is_saved_follow_set_of(ev, &relay.public_key(), &config.nostr.mirror_set)
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

pub async fn list(config: &Config) -> Result<()> {
    let relay = RelayClient::connect(Signer::require(config)?, &config.nostr.relays).await?;
    let view = collect_mirror_list(&relay, config).await?;
    relay.shutdown().await;
    print::print_mirror_list(config, &view);
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
    previous: Option<&Event>,
    changed: &[PublicKey],
) -> Result<(bool, Vec<nostr::RelaySendResult>)> {
    if changed.is_empty() {
        return Ok((false, Vec::new()));
    }
    let created_at = set::next_created_at(Timestamp::now().as_secs(), previous);
    let event = relay
        .sign(
            set.build_event_builder(&config.nostr.mirror_set)
                .custom_created_at(Timestamp::from_secs(created_at)),
        )
        .await
        .context("signing mirror set event")?;
    let output = relay.publish_to_relays(&event).await?;
    Ok((true, nostr::relay_send_results(relay.relays(), &output)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MirrorOp {
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowSetCapExceeded {
    pub total: usize,
}

impl std::fmt::Display for FollowSetCapExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "would grow the follow set to {} entries, over the {}-entry limit; remove some first",
            self.total,
            nostr::budget::MAX_FOLLOW_SET_ENTRIES
        )
    }
}

impl std::error::Error for FollowSetCapExceeded {}

fn ensure_within_follow_set_cap(total: usize) -> Result<(), FollowSetCapExceeded> {
    if total > nostr::budget::MAX_FOLLOW_SET_ENTRIES {
        return Err(FollowSetCapExceeded { total });
    }
    Ok(())
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
            let outcome = set.add(&keys);
            ensure_within_follow_set_cap(outcome.total)?;
            (outcome.added, outcome.already)
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
    let (published, relay_results) =
        publish_if_changed(relay, config, &set, existing.as_ref(), &changed).await?;
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

#[derive(Debug, Serialize)]
struct MirrorKeysBody<'a> {
    keys: &'a [String],
}

async fn api_mirror_change(
    config: &Config,
    path: &str,
    inputs: &[String],
) -> Result<dto::MirrorChangeDto> {
    let client = ApiClient::for_config(config)?;
    client
        .post_json(path, &MirrorKeysBody { keys: inputs })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

pub async fn add(config: &Config, inputs: &[String]) -> Result<()> {
    let change = api_mirror_change(config, "/api/mirror/add", inputs).await?;
    print::print_mirror_change_dto(&change, "already in mirror set", "added", false);
    Ok(())
}

pub async fn remove(config: &Config, inputs: &[String]) -> Result<()> {
    let change = api_mirror_change(config, "/api/mirror/remove", inputs).await?;
    print::print_mirror_change_dto(&change, "not in mirror set", "removed", true);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteRow {
    pub d: String,
    pub cid: String,
    pub url: Option<String>,
    pub size: Option<u64>,
    pub stored_size: Option<u64>,
    pub stored_at: Option<u64>,
    pub created_at: u64,
    pub title: Option<String>,
    pub message: Option<String>,
    pub nip05: Option<String>,
    pub replicas: Option<replicas::ReplicaCounts>,
    pub stored: bool,
    pub previous: Option<VersionRecord>,
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

fn nip05_status(state: &State, key: &str) -> Option<String> {
    state.verifications.get(key).map(|v| v.status.clone())
}

pub async fn collect_sites(relay: &RelayClient, config: &Config) -> Result<SitesView> {
    let (follow_event, follow_note) = current_follow_set(relay, config).await?;
    let follow_set_found = follow_event.is_some();
    let targets = follow_targets(follow_event.as_ref());
    let latest = relay
        .fetch_latest_sites(config.nostr.site_event_kind, &targets)
        .await?;
    let latest_sites =
        nostr::cap_sites_per_author(latest.values(), nostr::budget::MAX_SITES_PER_AUTHOR_LISTED);
    let own_chosen: HashSet<PublicKey> = targets.iter().copied().collect();
    let (replica_data, replicas_error) =
        match fetch_tiered_reports(relay, config, &targets, own_chosen, &latest_sites).await {
            Ok(data) => (Some(data), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
    let state = load_state(config).await?;
    Ok(SitesView {
        follow_set_found,
        follow_note,
        accounts: followed_accounts(&targets, &latest_sites, &state, &replica_data)?,
        replicas_error,
        remove_on_unfollow: config.policy.remove_on_unfollow,
        unfollowed: unfollowed_accounts(&state, &targets)?,
    })
}

fn follow_targets(follow_event: Option<&Event>) -> Vec<PublicKey> {
    let Some(ev) = follow_event else {
        return Vec::new();
    };
    let (targets, truncated) = nostr::follow_set_pubkeys_capped(ev);
    if truncated {
        warn!(
            event_id = %ev.id,
            cap = nostr::budget::MAX_FOLLOW_SET_ENTRIES,
            "follow set has more p tags than the cap; the rest are ignored"
        );
    }
    targets
}

fn followed_accounts(
    targets: &[PublicKey],
    latest_sites: &[&nostr::SiteEvent],
    state: &State,
    replica_data: &Option<TieredReports>,
) -> Result<Vec<AccountSites>> {
    let mut by_pubkey: BTreeMap<String, Vec<&nostr::SiteEvent>> = BTreeMap::new();
    for pk in targets {
        by_pubkey.entry(pk.to_hex()).or_default();
    }
    for ev in latest_sites.iter().copied() {
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
                let versions = state.sites.get(&key).map(Vec::as_slice).unwrap_or_default();
                let stored_version = versions.iter().find(|v| v.cid == ev.cid);
                let previous = match stored_version {
                    Some(_) => None,
                    None => versions.iter().max_by_key(|v| v.created_at).cloned(),
                };
                SiteRow {
                    d: ev.d.clone(),
                    cid: ev.cid.clone(),
                    url: ev.url.clone(),
                    size: ev.size,
                    stored_size: stored_version.map(|v| v.size),
                    stored_at: stored_version.map(|v| v.stored_at),
                    created_at: ev.created_at,
                    title: ev.title.clone(),
                    message: ev.message.clone(),
                    nip05: nip05_status(state, &key),
                    replicas: replica_counts(replica_data, ev),
                    stored: stored_version.is_some(),
                    previous,
                }
            })
            .collect();
        accounts.push(AccountSites { pubkey, sites });
    }
    Ok(accounts)
}

fn unfollowed_accounts(state: &State, targets: &[PublicKey]) -> Result<Vec<AccountSites>> {
    let target_hex: BTreeSet<String> = targets.iter().map(|pk| pk.to_hex()).collect();
    let unfollowed_map = unfollowed_sites(state, &target_hex);
    let mut unfollowed = Vec::with_capacity(unfollowed_map.len());
    for (pubkey_hex, sites) in unfollowed_map {
        let pubkey = PublicKey::from_hex(&pubkey_hex).context("parsing pubkey")?;
        let sites = sites
            .into_iter()
            .map(|(d, version)| {
                let key = state::site_key(&pubkey_hex, &d);
                SiteRow {
                    nip05: nip05_status(state, &key),
                    d,
                    cid: version.cid,
                    url: None,
                    size: Some(version.size),
                    stored_size: Some(version.size),
                    stored_at: Some(version.stored_at),
                    created_at: version.created_at,
                    title: None,
                    message: None,
                    replicas: None,
                    stored: true,
                    previous: None,
                }
            })
            .collect();
        unfollowed.push(AccountSites { pubkey, sites });
    }
    Ok(unfollowed)
}

type TieredReports = (
    HashMap<SiteAddress, replicas::SiteReportSet>,
    replicas::Chosen,
);

async fn fetch_tiered_reports(
    relay: &RelayClient,
    config: &Config,
    targets: &[PublicKey],
    own_chosen: HashSet<PublicKey>,
    latest_sites: &[&nostr::SiteEvent],
) -> Result<TieredReports> {
    let chosen = replicas::fetch_chosen_with_own(relay, config, targets, own_chosen).await?;
    let reports = replicas::fetch_for_sites(relay, config, latest_sites, &chosen).await?;
    Ok((reports, chosen))
}

fn replica_counts(
    replica_data: &Option<TieredReports>,
    ev: &nostr::SiteEvent,
) -> Option<replicas::ReplicaCounts> {
    let (reports, chosen) = replica_data.as_ref()?;
    Some(reports.get(&(ev.pubkey, ev.d.clone())).map_or_else(
        replicas::ReplicaCounts::default,
        |r| {
            replicas::count_replicas(&replicas::replicas_of(
                &r.reports, &ev.cid, &ev.pubkey, chosen,
            ))
        },
    ))
}

pub async fn sites(config: &Config) -> Result<()> {
    let relay = RelayClient::connect(Signer::require(config)?, &config.nostr.relays).await?;
    let view = collect_sites(&relay, config).await?;
    relay.shutdown().await;
    print::print_sites(&view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::keys;

    #[test]
    fn ensure_within_follow_set_cap_allows_exactly_the_budget() {
        assert!(ensure_within_follow_set_cap(nostr::budget::MAX_FOLLOW_SET_ENTRIES).is_ok());
    }

    #[test]
    fn ensure_within_follow_set_cap_rejects_growing_past_the_budget() {
        let err =
            ensure_within_follow_set_cap(nostr::budget::MAX_FOLLOW_SET_ENTRIES + 1).unwrap_err();
        assert!(err.to_string().contains("over the"));
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

    #[test]
    fn followed_accounts_reports_the_latest_stored_version_while_the_event_is_not_stored() {
        let pk = keys().public_key();
        let hex = pk.to_hex();
        let record = |cid: &str, created_at| VersionRecord {
            cid: cid.into(),
            size: created_at,
            created_at,
            stored_at: created_at,
        };
        let mut state = State::default();
        state.apply_store(&state::site_key(&hex, "pending.example"), record("old", 1));
        state.apply_store(
            &state::site_key(&hex, "pending.example"),
            record("older", 0),
        );
        state.apply_store(&state::site_key(&hex, "stored.example"), record("old", 1));
        state.apply_store(
            &state::site_key(&hex, "stored.example"),
            record(crate::test_support::CID_A, 2),
        );
        let events = [
            crate::test_support::site_event_fixture(pk, "pending.example", 3),
            crate::test_support::site_event_fixture(pk, "stored.example", 2),
            crate::test_support::site_event_fixture(pk, "new.example", 3),
        ];
        let latest: Vec<&nostr::SiteEvent> = events.iter().collect();

        let accounts = followed_accounts(&[pk], &latest, &state, &None).unwrap();

        let rows: Vec<(&str, bool, Option<&str>)> = accounts[0]
            .sites
            .iter()
            .map(|s| {
                (
                    s.d.as_str(),
                    s.stored,
                    s.previous.as_ref().map(|v| v.cid.as_str()),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("new.example", false, None),
                ("pending.example", false, Some("old")),
                ("stored.example", true, None),
            ]
        );
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
