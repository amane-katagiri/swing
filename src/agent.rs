use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use futures_util::StreamExt;
use nostr_sdk::prelude::*;
use tracing::{debug, error, info, warn};

use crate::config::{Config, Nip05Mode};
use crate::ipfs::{IpfsClient, KuboPins};
use crate::nip05::{HttpNip05Verifier, Nip05Verify};
use crate::nostr::{self, RelayClient, SiteEvent};
use crate::policy::{self, CandidateEvent, VersionInfo};
use crate::state::{self, SiteKey, State, Verification, VersionRecord};

fn now_secs() -> u64 {
    Timestamp::now().as_secs()
}

pub async fn run(config: Config) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    info!(relays = ?relay.relays(), "connected to relays");

    let ipfs = IpfsClient::new(config.ipfs.api.clone());
    let nip05 = HttpNip05Verifier::public_only();
    let state_path = config.agent.state_dir.join("state.json");
    let mut state = State::load(&state_path).await?;
    info!(path = %state_path.display(), sites = state.sites.len(), "loaded state");

    reconcile_with_kubo(&ipfs, &state).await;

    let mut targets: HashSet<PublicKey> = HashSet::new();
    let mut notifications = relay.notifications();
    let mut poll_timer = tokio::time::interval(config.agent.poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    refresh_follow_set(
        &relay,
        &config,
        &ipfs,
        &nip05,
        &mut state,
        &state_path,
        &mut targets,
    )
    .await;

    loop {
        tokio::select! {
            maybe_note = notifications.next() => {
                match maybe_note {
                    Some(ClientNotification::Event { event, subscription_id, .. }) => {
                        if subscription_id.as_str() == nostr::SITE_SUBSCRIPTION_ID
                            && event.kind == Kind::Custom(config.nostr.site_event_kind)
                        {
                            process_raw_event(&config, &ipfs, &nip05, &mut state, &state_path, &event, &targets).await;
                        } else {
                            debug!(
                                kind = %event.kind,
                                subscription_id = %subscription_id,
                                "ignoring notification outside the site subscription"
                            );
                        }
                    }
                    Some(ClientNotification::Shutdown) | None => {
                        anyhow::bail!("relay notification stream ended");
                    }
                    Some(_) => {}
                }
            }
            _ = poll_timer.tick() => {
                refresh_follow_set(&relay, &config, &ipfs, &nip05, &mut state, &state_path, &mut targets).await;
            }
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown requested");
                relay.client.shutdown().await;
                break;
            }
        }
    }
    Ok(())
}

async fn reconcile_with_kubo(ipfs: &IpfsClient, state: &State) {
    match ipfs.pin_ls().await {
        Ok(kubo_pins) => {
            let state_pins: HashSet<String> =
                state.all_pinned_cids().map(|s| s.to_string()).collect();
            for cid in state_pins.difference(&kubo_pins) {
                warn!(cid = %cid, "state.json references a CID not pinned in Kubo");
            }
            for cid in kubo_pins.difference(&state_pins) {
                warn!(cid = %cid, "Kubo has a pin not tracked in state.json");
            }
        }
        Err(e) => warn!(error = %e, "could not query Kubo pin/ls for reconciliation"),
    }
}

async fn refresh_follow_set<N: Nip05Verify>(
    relay: &RelayClient,
    config: &Config,
    ipfs: &IpfsClient,
    nip05: &N,
    state: &mut State,
    state_path: &Path,
    targets: &mut HashSet<PublicKey>,
) {
    let follow_event = match relay.fetch_follow_set(&config.nostr.mirror_set).await {
        Ok(Some(ev)) => ev,
        Ok(None) => {
            warn!(mirror_set = %config.nostr.mirror_set, "no follow set found yet; will retry");
            return;
        }
        Err(e) => {
            warn!(error = %e, "fetching follow set failed; will retry");
            return;
        }
    };

    let new_targets: HashSet<PublicKey> = nostr::extract_follow_set_pubkeys(&follow_event)
        .into_iter()
        .collect();

    let removed: Vec<PublicKey> = targets.difference(&new_targets).copied().collect();
    if config.policy.unpin_on_unfollow {
        for pk in removed {
            unfollow_pubkey(ipfs, state, state_path, pk).await;
        }
    }
    *targets = new_targets;

    let target_list: Vec<PublicKey> = targets.iter().copied().collect();
    if let Err(e) = relay
        .subscribe_site_events(config.nostr.site_event_kind, &target_list)
        .await
    {
        warn!(error = %e, "subscribing to site events failed");
    }

    match relay
        .fetch_site_events(config.nostr.site_event_kind, &target_list)
        .await
    {
        Ok(events) => {
            let parsed: Vec<SiteEvent> = events
                .iter()
                .filter_map(
                    |e| match nostr::parse_site_event(e, config.nostr.site_event_kind) {
                        Ok(se) => Some(se),
                        Err(err) => {
                            warn!(error = %err, "skipping invalid historical site event");
                            None
                        }
                    },
                )
                .collect();
            let latest = nostr::select_latest(&parsed);
            for ev in latest.into_values() {
                apply_site_event(config, ipfs, nip05, state, state_path, &ev, targets).await;
            }
        }
        Err(e) => warn!(error = %e, "fetching historical site events failed"),
    }
}

async fn unfollow_pubkey(
    ipfs: &IpfsClient,
    state: &mut State,
    state_path: &Path,
    pubkey: PublicKey,
) {
    let prefix = format!("{}:", pubkey.to_hex());
    let keys: Vec<String> = state
        .sites
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();
    for key in keys {
        let versions = state.remove_site(&key);
        for v in versions {
            if let Err(e) = ipfs.pin_rm(&v.cid).await {
                warn!(cid = %v.cid, error = %e, "pin_rm failed while unfollowing");
            }
        }
        if let Err(e) = state.save(state_path).await {
            error!(error = %e, "saving state after unfollow failed");
        }
        info!(site_key = %key, "unfollowed and unpinned");
    }
}

async fn process_raw_event<C: KuboPins, N: Nip05Verify>(
    config: &Config,
    ipfs: &C,
    nip05: &N,
    state: &mut State,
    state_path: &Path,
    event: &Event,
    targets: &HashSet<PublicKey>,
) {
    match nostr::parse_site_event(event, config.nostr.site_event_kind) {
        Ok(site_event) => {
            apply_site_event(config, ipfs, nip05, state, state_path, &site_event, targets).await
        }
        Err(e) => warn!(error = %e, "skipping invalid site event"),
    }
}

async fn apply_site_event<C: KuboPins, N: Nip05Verify>(
    config: &Config,
    ipfs: &C,
    nip05: &N,
    state: &mut State,
    state_path: &Path,
    ev: &SiteEvent,
    targets: &HashSet<PublicKey>,
) {
    if !targets.contains(&ev.pubkey) {
        warn!(
            site = %ev.d,
            pubkey = %ev.pubkey.to_hex(),
            "ignoring site event from a pubkey outside the current follow set"
        );
        return;
    }

    let key = state::site_key(&ev.pubkey.to_hex(), &ev.d);

    if config.policy.nip05 != Nip05Mode::Off {
        let pubkey_hex = ev.pubkey.to_hex();
        let result = nip05.verify(&ev.d, &pubkey_hex).await;
        let verified = result.is_verified();
        if !verified {
            warn!(
                site = %ev.d,
                pubkey = %pubkey_hex,
                status = result.as_state_str(),
                "nip05 verification did not pass"
            );
        }
        state.set_verification(
            &key,
            Verification {
                status: result.as_state_str().to_string(),
                detail: result.detail(),
                checked_at: now_secs(),
            },
        );
        if let Err(e) = state.save(state_path).await {
            error!(error = %e, "saving state after nip05 verification failed");
        }
        if config.policy.nip05 == Nip05Mode::Require && !verified {
            return;
        }
    }

    let existing_versions: Vec<VersionInfo> = state
        .sites
        .get(&key)
        .map(|vs| {
            vs.iter()
                .map(|v| VersionInfo {
                    cid: v.cid.clone(),
                    size: v.size,
                    created_at: v.created_at,
                })
                .collect()
        })
        .unwrap_or_default();
    let other_total = state.total_bytes() - state.site_bytes(&key);

    match ev.size {
        Some(size) => {
            let candidate = CandidateEvent {
                cid: ev.cid.clone(),
                size: Some(size),
                created_at: ev.created_at,
            };
            let decision = policy::decide(
                &existing_versions,
                other_total,
                &candidate,
                &config.policy,
                now_secs(),
            );
            let Some(cid) = decision.pin else {
                info!(site = %ev.d, pubkey = %ev.pubkey.to_hex(), reason = %decision.reason, "skip");
                return;
            };
            if let Err(e) = ipfs.pin_add(&cid, config.agent.pin_timeout).await {
                error!(cid = %cid, error = %e, "pin_add failed");
                return;
            }
            let accepted = Accepted {
                cid,
                size,
                unpin: decision.unpin,
            };
            apply_decision(ipfs, &key, state, state_path, ev, accepted).await;
        }
        None => {
            let cid = ev.cid.clone();
            if let Err(e) = ipfs.pin_add(&cid, config.agent.pin_timeout).await {
                error!(cid = %cid, error = %e, "pin_add failed");
                return;
            }
            let size = match ipfs.files_stat(&cid).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(cid = %cid, error = %e, "files/stat failed after pin; unpinning and retrying later");
                    if let Err(e) = ipfs.pin_rm(&cid).await {
                        warn!(cid = %cid, error = %e, "pin_rm failed after stat failure");
                    }
                    return;
                }
            };
            let candidate = CandidateEvent {
                cid: cid.clone(),
                size: Some(size),
                created_at: ev.created_at,
            };
            let decision = policy::decide(
                &existing_versions,
                other_total,
                &candidate,
                &config.policy,
                now_secs(),
            );
            let Some(decided_cid) = decision.pin else {
                warn!(cid = %cid, site = %ev.d, size, reason = %decision.reason, "rejected after stat; unpinning");
                if let Err(e) = ipfs.pin_rm(&cid).await {
                    warn!(cid = %cid, error = %e, "pin_rm failed after rejection");
                }
                return;
            };
            let accepted = Accepted {
                cid: decided_cid,
                size,
                unpin: decision.unpin,
            };
            apply_decision(ipfs, &key, state, state_path, ev, accepted).await;
        }
    }
}

struct Accepted {
    cid: String,
    size: u64,
    unpin: Vec<String>,
}

async fn apply_decision<C: KuboPins>(
    ipfs: &C,
    key: &SiteKey,
    state: &mut State,
    state_path: &Path,
    ev: &SiteEvent,
    accepted: Accepted,
) {
    let Accepted { cid, size, unpin } = accepted;
    let mut unpinned = Vec::new();
    for unpin_cid in &unpin {
        match ipfs.pin_rm(unpin_cid).await {
            Ok(()) => {
                unpinned.push(unpin_cid.clone());
                info!(cid = %unpin_cid, site = %ev.d, "unpinned");
            }
            Err(e) => warn!(cid = %unpin_cid, error = %e, "pin_rm failed"),
        }
    }
    state.apply_unpins(key, &unpinned);

    state.apply_pin(
        key,
        VersionRecord {
            cid: cid.clone(),
            size,
            created_at: ev.created_at,
            pinned_at: now_secs(),
        },
    );
    if let Err(e) = state.save(state_path).await {
        error!(error = %e, "saving state after pin failed");
    }
    info!(cid = %cid, site = %ev.d, pubkey = %ev.pubkey.to_hex(), size, "pinned");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::config::{AgentConfig, IpfsConfig, Nip05Mode, NostrConfig, PolicyConfig};
    use crate::nip05::VerificationResult;

    #[derive(Default)]
    struct FakeKuboState {
        pinned: HashSet<String>,
        fail_pin_add: HashSet<String>,
        fail_stat: HashSet<String>,
        stat_sizes: HashMap<String, u64>,
    }

    #[derive(Default)]
    struct FakeKubo(Mutex<FakeKuboState>);

    impl KuboPins for FakeKubo {
        async fn pin_add(&self, cid: &str, _timeout: Duration) -> Result<()> {
            let mut s = self.0.lock().unwrap();
            if s.fail_pin_add.contains(cid) {
                anyhow::bail!("simulated pin_add failure");
            }
            s.pinned.insert(cid.to_string());
            Ok(())
        }

        async fn pin_rm(&self, cid: &str) -> Result<()> {
            let mut s = self.0.lock().unwrap();
            s.pinned.remove(cid);
            Ok(())
        }

        async fn files_stat(&self, cid: &str) -> Result<u64> {
            let s = self.0.lock().unwrap();
            if s.fail_stat.contains(cid) {
                anyhow::bail!("simulated files/stat failure");
            }
            Ok(*s.stat_sizes.get(cid).unwrap_or(&0))
        }
    }

    #[derive(Default)]
    struct FakeNip05(Mutex<HashMap<String, VerificationResult>>);

    impl FakeNip05 {
        fn set(&self, d: &str, pubkey_hex: &str, result: VerificationResult) {
            self.0
                .lock()
                .unwrap()
                .insert(format!("{d}:{pubkey_hex}"), result);
        }
    }

    impl Nip05Verify for FakeNip05 {
        async fn verify(&self, d: &str, pubkey_hex: &str) -> VerificationResult {
            let key = format!("{d}:{pubkey_hex}");
            self.0
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| panic!("unexpected nip05 verify call for {key}"))
        }
    }

    fn test_config(policy: PolicyConfig) -> Config {
        Config {
            nostr: NostrConfig {
                secret_key: "unused".to_string().into(),
                relays: vec![],
                mirror_set: "site-mirror".to_string(),
                site_event_kind: 35980,
            },
            ipfs: IpfsConfig {
                api: "http://127.0.0.1:5001".to_string(),
            },
            policy,
            agent: AgentConfig {
                state_dir: std::path::PathBuf::from("./data"),
                poll_interval: Duration::from_secs(300),
                pin_timeout: Duration::from_secs(60),
            },
            publish: crate::config::PublishConfig {
                nip05: Nip05Mode::Off,
            },
        }
    }

    fn default_policy() -> PolicyConfig {
        PolicyConfig {
            max_total_storage: 1_000_000,
            max_per_site: 100_000,
            max_update_size: 100_000,
            keep_versions: 5,
            keep_days: 365,
            min_update_interval: 0,
            unpin_on_unfollow: true,
            nip05: Nip05Mode::Off,
        }
    }

    fn site_event(pubkey: PublicKey, cid: &str, size: Option<u64>, created_at: u64) -> SiteEvent {
        SiteEvent {
            pubkey,
            d: "example.com".to_string(),
            cid: cid.to_string(),
            url: None,
            size,
            created_at,
        }
    }

    #[tokio::test]
    async fn pin_failure_leaves_state_and_old_pins_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let config = test_config(default_policy());
        let kubo = FakeKubo::default();
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);

        let key = state::site_key(&pubkey.to_hex(), "example.com");
        let mut state = State::default();
        state.apply_pin(
            &key,
            VersionRecord {
                cid: "bafy-old".to_string(),
                size: 10,
                created_at: 100,
                pinned_at: 100,
            },
        );
        kubo.0.lock().unwrap().pinned.insert("bafy-old".to_string());
        kubo.0
            .lock()
            .unwrap()
            .fail_pin_add
            .insert("bafy-new".to_string());

        let ev = site_event(pubkey, "bafy-new", Some(20), 200);
        apply_site_event(
            &config,
            &kubo,
            &FakeNip05::default(),
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        assert_eq!(state.site_bytes(&key), 10);
        assert!(
            state
                .sites
                .get(&key)
                .unwrap()
                .iter()
                .any(|v| v.cid == "bafy-old")
        );
        assert!(kubo.0.lock().unwrap().pinned.contains("bafy-old"));
        assert!(!kubo.0.lock().unwrap().pinned.contains("bafy-new"));
        assert!(!state_path.exists());
    }

    #[tokio::test]
    async fn event_from_non_target_pubkey_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let config = test_config(default_policy());
        let kubo = FakeKubo::default();
        let author = Keys::generate().public_key();
        let targets: HashSet<PublicKey> = HashSet::new();

        let mut state = State::default();
        let ev = site_event(author, "bafy-intruder", Some(20), 200);
        apply_site_event(
            &config,
            &kubo,
            &FakeNip05::default(),
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        assert!(state.sites.is_empty());
        assert!(!kubo.0.lock().unwrap().pinned.contains("bafy-intruder"));
    }

    #[tokio::test]
    async fn stat_failure_after_pin_unpins_and_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let config = test_config(default_policy());
        let kubo = FakeKubo::default();
        kubo.0
            .lock()
            .unwrap()
            .fail_stat
            .insert("bafy-sizeless".to_string());
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);

        let mut state = State::default();
        let ev = site_event(pubkey, "bafy-sizeless", None, 200);
        apply_site_event(
            &config,
            &kubo,
            &FakeNip05::default(),
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        assert!(state.sites.is_empty());
        assert!(!kubo.0.lock().unwrap().pinned.contains("bafy-sizeless"));
    }

    #[tokio::test]
    async fn sizeless_event_over_max_per_site_after_stat_is_unpinned_and_not_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let mut policy = default_policy();
        policy.max_per_site = 50;
        let config = test_config(policy);
        let kubo = FakeKubo::default();
        kubo.0
            .lock()
            .unwrap()
            .stat_sizes
            .insert("bafy-big".to_string(), 1_000);
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);

        let mut state = State::default();
        let ev = site_event(pubkey, "bafy-big", None, 200);
        apply_site_event(
            &config,
            &kubo,
            &FakeNip05::default(),
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        let key = state::site_key(&pubkey.to_hex(), "example.com");
        assert!(!state.sites.contains_key(&key));
        assert!(!kubo.0.lock().unwrap().pinned.contains("bafy-big"));
    }

    #[tokio::test]
    async fn sizeless_event_within_limits_evicts_oldest_per_keep_versions() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let mut policy = default_policy();
        policy.keep_versions = 1;
        let config = test_config(policy);
        let kubo = FakeKubo::default();
        kubo.0
            .lock()
            .unwrap()
            .stat_sizes
            .insert("bafy-new".to_string(), 20);
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);

        let key = state::site_key(&pubkey.to_hex(), "example.com");
        let mut state = State::default();
        state.apply_pin(
            &key,
            VersionRecord {
                cid: "bafy-old".to_string(),
                size: 10,
                created_at: 100,
                pinned_at: 100,
            },
        );
        kubo.0.lock().unwrap().pinned.insert("bafy-old".to_string());

        let ev = site_event(pubkey, "bafy-new", None, 200);
        apply_site_event(
            &config,
            &kubo,
            &FakeNip05::default(),
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        assert_eq!(state.site_bytes(&key), 20);
        assert!(
            state
                .sites
                .get(&key)
                .unwrap()
                .iter()
                .any(|v| v.cid == "bafy-new")
        );
        assert!(!kubo.0.lock().unwrap().pinned.contains("bafy-old"));
        assert!(kubo.0.lock().unwrap().pinned.contains("bafy-new"));
    }

    fn nip05_policy(mode: Nip05Mode) -> PolicyConfig {
        PolicyConfig {
            nip05: mode,
            ..default_policy()
        }
    }

    #[tokio::test]
    async fn nip05_warn_mode_pins_despite_mismatch_and_records_result() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let config = test_config(nip05_policy(Nip05Mode::Warn));
        let kubo = FakeKubo::default();
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);
        let nip05 = FakeNip05::default();
        nip05.set(
            "example.com",
            &pubkey.to_hex(),
            VerificationResult::Mismatch,
        );

        let mut state = State::default();
        let ev = site_event(pubkey, "bafy-new", Some(20), 200);
        apply_site_event(
            &config,
            &kubo,
            &nip05,
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        let key = state::site_key(&pubkey.to_hex(), "example.com");
        assert!(kubo.0.lock().unwrap().pinned.contains("bafy-new"));
        assert_eq!(state.site_bytes(&key), 20);
        assert_eq!(state.verifications.get(&key).unwrap().status, "mismatch");
    }

    #[tokio::test]
    async fn nip05_require_mode_skips_pin_when_not_verified() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let config = test_config(nip05_policy(Nip05Mode::Require));
        let kubo = FakeKubo::default();
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);
        let nip05 = FakeNip05::default();
        nip05.set(
            "example.com",
            &pubkey.to_hex(),
            VerificationResult::NotApplicable,
        );

        let mut state = State::default();
        let ev = site_event(pubkey, "bafy-new", Some(20), 200);
        apply_site_event(
            &config,
            &kubo,
            &nip05,
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        let key = state::site_key(&pubkey.to_hex(), "example.com");
        assert!(!kubo.0.lock().unwrap().pinned.contains("bafy-new"));
        assert!(!state.sites.contains_key(&key));
        assert_eq!(
            state.verifications.get(&key).unwrap().status,
            "not_applicable"
        );
    }

    #[tokio::test]
    async fn nip05_require_mode_pins_when_verified() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let config = test_config(nip05_policy(Nip05Mode::Require));
        let kubo = FakeKubo::default();
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);
        let nip05 = FakeNip05::default();
        nip05.set(
            "example.com",
            &pubkey.to_hex(),
            VerificationResult::Verified,
        );

        let mut state = State::default();
        let ev = site_event(pubkey, "bafy-new", Some(20), 200);
        apply_site_event(
            &config,
            &kubo,
            &nip05,
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        let key = state::site_key(&pubkey.to_hex(), "example.com");
        assert!(kubo.0.lock().unwrap().pinned.contains("bafy-new"));
        assert_eq!(state.verifications.get(&key).unwrap().status, "verified");
    }

    #[tokio::test]
    async fn nip05_off_mode_never_calls_verifier() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let config = test_config(nip05_policy(Nip05Mode::Off));
        let kubo = FakeKubo::default();
        let pubkey = Keys::generate().public_key();
        let targets = HashSet::from([pubkey]);
        let nip05 = FakeNip05::default();

        let mut state = State::default();
        let ev = site_event(pubkey, "bafy-new", Some(20), 200);
        apply_site_event(
            &config,
            &kubo,
            &nip05,
            &mut state,
            &state_path,
            &ev,
            &targets,
        )
        .await;

        let key = state::site_key(&pubkey.to_hex(), "example.com");
        assert!(kubo.0.lock().unwrap().pinned.contains("bafy-new"));
        assert!(!state.verifications.contains_key(&key));
    }
}
