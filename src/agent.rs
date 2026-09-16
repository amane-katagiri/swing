use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;
use futures_util::StreamExt;
use nostr_sdk::prelude::*;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::config::{Config, Nip05Mode};
use crate::ipfs::{FetchLimits, Fetched, IpfsClient, KuboPins};
use crate::nip05::{self, HttpNip05Verifier, Nip05Verify};
use crate::nostr::{self, RelayClient, SiteEvent};
use crate::policy::{self, CandidateEvent, Decision, Usage, VersionInfo};
use crate::state::{self, SiteKey, State, Verification, VersionRecord};

const NIP05_ERROR_CACHE_TTL: u64 = 900;

fn now_secs() -> u64 {
    Timestamp::now().as_secs()
}

pub async fn run(config: Config) -> Result<()> {
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    info!(relays = ?relay.relays(), "connected to relays");

    let ipfs = IpfsClient::new(config.ipfs.api.clone());
    let state_path = config.agent.state_dir.join("state.json");
    let state = State::load(&state_path).await?;
    info!(path = %state_path.display(), sites = state.sites.len(), "loaded state");

    reconcile_with_kubo(&ipfs, &state).await;

    let site_event_kind = config.nostr.site_event_kind;
    let mut poll_timer = tokio::time::interval(config.agent.poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let agent = Arc::new(Agent::new(
        config,
        ipfs,
        HttpNip05Verifier::public_only(),
        state,
        state_path,
    ));
    let mut tasks = JoinSet::new();
    let mut notifications = relay.notifications();

    loop {
        tokio::select! {
            maybe_note = notifications.next() => {
                match maybe_note {
                    Some(ClientNotification::Event { event, subscription_id, .. }) => {
                        if subscription_id.as_str() == nostr::SITE_SUBSCRIPTION_ID
                            && event.kind == Kind::Custom(site_event_kind)
                        {
                            match nostr::parse_site_event(&event, site_event_kind) {
                                Ok(ev) => agent.submit(ev, &mut tasks),
                                Err(e) => warn!(error = %e, "skipping invalid site event"),
                            }
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
                refresh_follow_set(&relay, &agent, &mut tasks).await;
            }
            Some(joined) = tasks.join_next() => {
                if let Err(e) = joined {
                    error!(error = %e, "site task panicked");
                }
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

async fn refresh_follow_set<C, N>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N>>,
    tasks: &mut JoinSet<()>,
) where
    C: KuboPins + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
{
    let config = &agent.config;
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
    let removed = agent.replace_targets(new_targets.clone());
    if config.policy.unpin_on_unfollow {
        for pk in removed {
            agent.unfollow(pk).await;
        }
    }

    let target_list: Vec<PublicKey> = new_targets.into_iter().collect();
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
            for ev in nostr::select_latest(&parsed).into_values() {
                agent.submit(ev, tasks);
            }
        }
        Err(e) => warn!(error = %e, "fetching historical site events failed"),
    }
}

fn decide(
    state: &State,
    key: &SiteKey,
    pubkey_hex: &str,
    ev: &SiteEvent,
    size: Option<u64>,
    config: &Config,
) -> Decision {
    let existing: Vec<VersionInfo> = state
        .sites
        .get(key)
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
    let site = state.site_bytes(key);
    let usage = Usage {
        other_sites: state.total_bytes() - site,
        other_sites_of_account: state.account_bytes(pubkey_hex) - site,
    };
    let candidate = CandidateEvent {
        cid: ev.cid.clone(),
        size,
        created_at: ev.created_at,
    };
    policy::decide(&existing, usage, &candidate, &config.policy, now_secs())
}

struct Queued {
    running_created_at: u64,
    next: Option<SiteEvent>,
}

struct Agent<C, N> {
    config: Config,
    ipfs: C,
    nip05: N,
    state: tokio::sync::Mutex<State>,
    state_path: PathBuf,
    targets: RwLock<HashSet<PublicKey>>,
    queue: Mutex<HashMap<SiteKey, Queued>>,
    permits: Semaphore,
}

impl<C, N> Agent<C, N>
where
    C: KuboPins + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
{
    fn submit(self: &Arc<Self>, ev: SiteEvent, tasks: &mut JoinSet<()>) {
        let key = state::site_key(&ev.pubkey.to_hex(), &ev.d);
        {
            let mut queue = self.queue.lock().unwrap();
            if let Some(queued) = queue.get_mut(&key) {
                let newest = queued
                    .next
                    .as_ref()
                    .map_or(queued.running_created_at, |n| n.created_at);
                if ev.created_at > newest {
                    queued.next = Some(ev);
                }
                return;
            }
            queue.insert(
                key.clone(),
                Queued {
                    running_created_at: ev.created_at,
                    next: None,
                },
            );
        }
        let agent = Arc::clone(self);
        tasks.spawn(async move { agent.drain(key, ev).await });
    }

    async fn drain(&self, key: SiteKey, first: SiteEvent) {
        let mut ev = first;
        loop {
            {
                let _permit = self
                    .permits
                    .acquire()
                    .await
                    .expect("the semaphore is never closed");
                self.apply_site_event(&ev).await;
            }
            let next = {
                let mut queue = self.queue.lock().unwrap();
                let next = queue.get_mut(&key).and_then(|q| q.next.take());
                match &next {
                    Some(n) => {
                        if let Some(q) = queue.get_mut(&key) {
                            q.running_created_at = n.created_at;
                        }
                    }
                    None => {
                        queue.remove(&key);
                    }
                }
                next
            };
            match next {
                Some(n) => ev = n,
                None => return,
            }
        }
    }
}

impl<C: KuboPins, N: Nip05Verify> Agent<C, N> {
    fn new(config: Config, ipfs: C, nip05: N, state: State, state_path: PathBuf) -> Self {
        let permits = Semaphore::new(config.agent.concurrency);
        Self {
            config,
            ipfs,
            nip05,
            state: tokio::sync::Mutex::new(state),
            state_path,
            targets: RwLock::new(HashSet::new()),
            queue: Mutex::new(HashMap::new()),
            permits,
        }
    }

    fn is_target(&self, pubkey: &PublicKey) -> bool {
        self.targets.read().unwrap().contains(pubkey)
    }

    fn replace_targets(&self, new_targets: HashSet<PublicKey>) -> Vec<PublicKey> {
        let mut targets = self.targets.write().unwrap();
        let removed = targets.difference(&new_targets).copied().collect();
        *targets = new_targets;
        removed
    }

    async fn save(&self, state: &State, after: &str) {
        if let Err(e) = state.save(&self.state_path).await {
            error!(error = %e, "saving state after {after} failed");
        }
    }

    async fn release_cid(&self, state: &State, cid: &str) {
        if state.references_cid(cid) {
            debug!(cid = %cid, "still referenced by another site; keeping the pin");
            return;
        }
        match self.ipfs.pin_rm(cid).await {
            Ok(()) => info!(cid = %cid, "unpinned"),
            Err(e) => warn!(cid = %cid, error = %e, "pin_rm failed"),
        }
    }

    async fn unfollow(&self, pubkey: PublicKey) {
        let prefix = format!("{}:", pubkey.to_hex());
        let mut state = self.state.lock().await;
        let keys: HashSet<SiteKey> = state
            .sites
            .keys()
            .chain(state.verifications.keys())
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in keys {
            for v in state.remove_site(&key) {
                self.release_cid(&state, &v.cid).await;
            }
            self.save(&state, "unfollow").await;
            info!(site_key = %key, "unfollowed and unpinned");
        }
    }

    async fn nip05_verified(&self, key: &SiteKey, ev: &SiteEvent, pubkey_hex: &str) -> bool {
        let now = now_secs();
        let ttl = self.config.policy.nip05_cache_ttl;
        let cached = {
            let state = self.state.lock().await;
            state.verifications.get(key).and_then(|v| {
                let ttl = if v.status == nip05::STATE_ERROR {
                    ttl.min(NIP05_ERROR_CACHE_TTL)
                } else {
                    ttl
                };
                (now < v.checked_at.saturating_add(ttl)).then(|| v.status == nip05::STATE_VERIFIED)
            })
        };
        if let Some(verified) = cached {
            return verified;
        }

        let result = self.nip05.verify(&ev.d, pubkey_hex).await;
        if !result.is_verified() {
            warn!(
                site = %ev.d,
                pubkey = %pubkey_hex,
                status = result.as_state_str(),
                "nip05 verification did not pass"
            );
        }
        let mut state = self.state.lock().await;
        state.set_verification(
            key,
            Verification {
                status: result.as_state_str().to_string(),
                detail: result.detail(),
                checked_at: now_secs(),
            },
        );
        self.save(&state, "nip05 verification").await;
        result.is_verified()
    }

    async fn apply_site_event(&self, ev: &SiteEvent) {
        let pubkey_hex = ev.pubkey.to_hex();
        if !self.is_target(&ev.pubkey) {
            warn!(
                site = %ev.d,
                pubkey = %pubkey_hex,
                "ignoring site event from pubkey not in current follow set"
            );
            return;
        }
        let key = state::site_key(&pubkey_hex, &ev.d);

        let precheck = {
            let state = self.state.lock().await;
            decide(&state, &key, &pubkey_hex, ev, ev.size, &self.config)
        };
        if precheck.pin.is_none() {
            info!(site = %ev.d, pubkey = %pubkey_hex, reason = %precheck.reason, "skip");
            return;
        }

        let nip05_mode = self.config.policy.nip05;
        if nip05_mode != Nip05Mode::Off {
            let verified = self.nip05_verified(&key, ev, &pubkey_hex).await;
            if nip05_mode == Nip05Mode::Require && !verified {
                return;
            }
        }

        let limits = FetchLimits {
            max_bytes: policy::fetch_limit(&self.config.policy),
            total: self.config.agent.pin_timeout,
            idle: self.config.agent.fetch_idle_timeout,
        };
        match self.ipfs.fetch_dag(&ev.cid, limits).await {
            Ok(Fetched::Complete) => {}
            Ok(Fetched::TooLarge) => {
                warn!(cid = %ev.cid, site = %ev.d, limit = limits.max_bytes, "content exceeds the fetch limit; aborted");
                return;
            }
            Err(e) => {
                warn!(cid = %ev.cid, site = %ev.d, error = %e, "fetching content failed; will retry on next poll");
                return;
            }
        }

        let mut state = self.state.lock().await;
        if !self.is_target(&ev.pubkey) {
            info!(site = %ev.d, pubkey = %pubkey_hex, "author left the follow set during fetch; not pinning");
            return;
        }
        if let Err(e) = self
            .ipfs
            .pin_add_local(&ev.cid, self.config.agent.pin_timeout)
            .await
        {
            error!(cid = %ev.cid, error = %e, "pin_add failed");
            return;
        }
        let size = match self.ipfs.dag_size_local(&ev.cid).await {
            Ok(size) => size,
            Err(e) => {
                warn!(cid = %ev.cid, error = %e, "dag/stat failed after pin; unpinning and retrying later");
                self.release_cid(&state, &ev.cid).await;
                return;
            }
        };
        if let Some(declared) = ev.size
            && declared < size
        {
            warn!(cid = %ev.cid, site = %ev.d, declared, actual = size, "size tag understates the content");
        }

        let decision = decide(&state, &key, &pubkey_hex, ev, Some(size), &self.config);
        let Some(cid) = decision.pin else {
            warn!(cid = %ev.cid, site = %ev.d, size, reason = %decision.reason, "rejected after fetch; unpinning");
            self.release_cid(&state, &ev.cid).await;
            return;
        };
        state.apply_pin(
            &key,
            VersionRecord {
                cid: cid.clone(),
                size,
                created_at: ev.created_at,
                pinned_at: now_secs(),
            },
        );
        state.apply_unpins(&key, &decision.unpin);
        for old in &decision.unpin {
            self.release_cid(&state, old).await;
        }
        self.save(&state, "pin").await;
        info!(cid = %cid, site = %ev.d, pubkey = %pubkey_hex, size, "pinned");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::config::{AgentConfig, IpfsConfig, NostrConfig, PolicyConfig};
    use crate::nip05::VerificationResult;

    #[derive(Default)]
    struct FakeKuboState {
        pinned: HashSet<String>,
        fetched: Vec<String>,
        fail_fetch: HashSet<String>,
        fail_stat: HashSet<String>,
        sizes: HashMap<String, u64>,
    }

    #[derive(Default)]
    struct FakeKubo {
        s: Mutex<FakeKuboState>,
        gate: tokio::sync::RwLock<()>,
        entered_fetch: tokio::sync::Notify,
    }

    impl FakeKubo {
        fn with(f: impl FnOnce(&mut FakeKuboState)) -> Self {
            let kubo = Self::default();
            f(&mut kubo.s.lock().unwrap());
            kubo
        }
    }

    impl KuboPins for FakeKubo {
        async fn fetch_dag(&self, cid: &str, limits: FetchLimits) -> Result<Fetched> {
            self.entered_fetch.notify_one();
            let _open = self.gate.read().await;
            let mut s = self.s.lock().unwrap();
            s.fetched.push(cid.to_string());
            if s.fail_fetch.contains(cid) {
                anyhow::bail!("simulated fetch failure");
            }
            if s.sizes.get(cid).copied().unwrap_or(0) > limits.max_bytes {
                return Ok(Fetched::TooLarge);
            }
            Ok(Fetched::Complete)
        }

        async fn pin_add_local(&self, cid: &str, _timeout: Duration) -> Result<()> {
            self.s.lock().unwrap().pinned.insert(cid.to_string());
            Ok(())
        }

        async fn dag_size_local(&self, cid: &str) -> Result<u64> {
            let s = self.s.lock().unwrap();
            if s.fail_stat.contains(cid) {
                anyhow::bail!("simulated dag/stat failure");
            }
            Ok(s.sizes.get(cid).copied().unwrap_or(0))
        }

        async fn pin_rm(&self, cid: &str) -> Result<()> {
            self.s.lock().unwrap().pinned.remove(cid);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeNip05 {
        results: Mutex<HashMap<String, VerificationResult>>,
        calls: Mutex<usize>,
    }

    impl FakeNip05 {
        fn set(&self, d: &str, pubkey_hex: &str, result: VerificationResult) {
            self.results
                .lock()
                .unwrap()
                .insert(format!("{d}:{pubkey_hex}"), result);
        }

        fn calls(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    impl Nip05Verify for FakeNip05 {
        async fn verify(&self, d: &str, pubkey_hex: &str) -> VerificationResult {
            *self.calls.lock().unwrap() += 1;
            let key = format!("{d}:{pubkey_hex}");
            self.results
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
                fetch_idle_timeout: Duration::from_secs(10),
                concurrency: 2,
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
            max_per_account: 1_000_000,
            max_update_size: 100_000,
            keep_versions: 5,
            keep_days: 365,
            min_update_interval: 0,
            unpin_on_unfollow: true,
            nip05: Nip05Mode::Off,
            nip05_cache_ttl: 86_400,
        }
    }

    fn nip05_policy(mode: Nip05Mode) -> PolicyConfig {
        PolicyConfig {
            nip05: mode,
            ..default_policy()
        }
    }

    struct Fixture {
        agent: Arc<Agent<FakeKubo, FakeNip05>>,
        pubkey: PublicKey,
        state_path: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(policy: PolicyConfig, kubo: FakeKubo) -> Self {
            Self::with_nip05(policy, kubo, FakeNip05::default())
        }

        fn with_nip05(policy: PolicyConfig, kubo: FakeKubo, nip05: FakeNip05) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state_path = dir.path().join("state.json");
            let pubkey = Keys::generate().public_key();
            let agent = Agent::new(
                test_config(policy),
                kubo,
                nip05,
                State::default(),
                state_path.clone(),
            );
            agent.replace_targets(HashSet::from([pubkey]));
            Self {
                agent: Arc::new(agent),
                pubkey,
                state_path,
                _dir: dir,
            }
        }

        fn key(&self, d: &str) -> SiteKey {
            state::site_key(&self.pubkey.to_hex(), d)
        }

        fn event(&self, d: &str, cid: &str, size: Option<u64>, created_at: u64) -> SiteEvent {
            SiteEvent {
                pubkey: self.pubkey,
                d: d.to_string(),
                cid: cid.to_string(),
                url: None,
                size,
                created_at,
            }
        }

        async fn seed(&self, d: &str, cid: &str, size: u64) {
            self.agent.state.lock().await.apply_pin(
                &self.key(d),
                VersionRecord {
                    cid: cid.to_string(),
                    size,
                    created_at: 100,
                    pinned_at: 100,
                },
            );
            self.kubo().pinned.insert(cid.to_string());
        }

        async fn apply(&self, ev: SiteEvent) {
            self.agent.apply_site_event(&ev).await;
        }

        fn kubo(&self) -> std::sync::MutexGuard<'_, FakeKuboState> {
            self.agent.ipfs.s.lock().unwrap()
        }

        async fn site_bytes(&self, d: &str) -> u64 {
            self.agent.state.lock().await.site_bytes(&self.key(d))
        }

        async fn verification(&self, d: &str) -> Option<String> {
            self.agent
                .state
                .lock()
                .await
                .verifications
                .get(&self.key(d))
                .map(|v| v.status.clone())
        }
    }

    const D: &str = "example.com";

    #[tokio::test]
    async fn fetch_failure_leaves_state_and_old_pins_untouched() {
        let kubo = FakeKubo::with(|s| {
            s.fail_fetch.insert("bafy-new".into());
        });
        let fx = Fixture::new(default_policy(), kubo);
        fx.seed(D, "bafy-old", 10).await;

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert_eq!(fx.site_bytes(D).await, 10);
        assert!(fx.kubo().pinned.contains("bafy-old"));
        assert!(!fx.kubo().pinned.contains("bafy-new"));
        assert!(!fx.state_path.exists());
    }

    #[tokio::test]
    async fn event_from_non_target_pubkey_is_ignored() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.agent.replace_targets(HashSet::new());

        fx.apply(fx.event(D, "bafy-intruder", Some(20), 200)).await;

        assert!(fx.agent.state.lock().await.sites.is_empty());
        assert!(fx.kubo().fetched.is_empty());
    }

    #[tokio::test]
    async fn stat_failure_after_pin_unpins_and_records_nothing() {
        let kubo = FakeKubo::with(|s| {
            s.fail_stat.insert("bafy-new".into());
        });
        let fx = Fixture::new(default_policy(), kubo);

        fx.apply(fx.event(D, "bafy-new", None, 200)).await;

        assert!(fx.agent.state.lock().await.sites.is_empty());
        assert!(!fx.kubo().pinned.contains("bafy-new"));
    }

    #[tokio::test]
    async fn oversized_content_is_aborted_during_fetch_even_with_a_small_size_tag() {
        let mut policy = default_policy();
        policy.max_per_site = 50;
        let kubo = FakeKubo::with(|s| {
            s.sizes.insert("bafy-liar".into(), 1_000);
        });
        let fx = Fixture::new(policy, kubo);

        fx.apply(fx.event(D, "bafy-liar", Some(20), 200)).await;

        assert_eq!(fx.kubo().fetched, vec!["bafy-liar".to_string()]);
        assert!(!fx.kubo().pinned.contains("bafy-liar"));
        assert!(!fx.agent.state.lock().await.sites.contains_key(&fx.key(D)));
    }

    #[tokio::test]
    async fn actual_size_is_recorded_and_rechecked_instead_of_the_size_tag() {
        let mut policy = default_policy();
        policy.max_total_storage = 100;
        let kubo = FakeKubo::with(|s| {
            s.sizes.insert("bafy-liar".into(), 80);
            s.sizes.insert("bafy-honest".into(), 30);
        });
        let fx = Fixture::new(policy, kubo);
        fx.seed("other.example", "bafy-other", 40).await;

        fx.apply(fx.event(D, "bafy-liar", Some(1), 200)).await;
        assert!(!fx.kubo().pinned.contains("bafy-liar"));
        assert_eq!(fx.site_bytes(D).await, 0);

        fx.apply(fx.event(D, "bafy-honest", Some(1), 300)).await;
        assert!(fx.kubo().pinned.contains("bafy-honest"));
        assert_eq!(fx.site_bytes(D).await, 30);
    }

    #[tokio::test]
    async fn declared_size_over_limit_is_skipped_before_fetching() {
        let mut policy = default_policy();
        policy.max_update_size = 50;
        let fx = Fixture::new(policy, FakeKubo::default());

        fx.apply(fx.event(D, "bafy-big", Some(1_000), 200)).await;

        assert!(fx.kubo().fetched.is_empty());
    }

    #[tokio::test]
    async fn sizeless_event_within_limits_evicts_oldest_per_keep_versions() {
        let mut policy = default_policy();
        policy.keep_versions = 1;
        let kubo = FakeKubo::with(|s| {
            s.sizes.insert("bafy-new".into(), 20);
        });
        let fx = Fixture::new(policy, kubo);
        fx.seed(D, "bafy-old", 10).await;

        fx.apply(fx.event(D, "bafy-new", None, 200)).await;

        assert_eq!(fx.site_bytes(D).await, 20);
        assert!(!fx.kubo().pinned.contains("bafy-old"));
        assert!(fx.kubo().pinned.contains("bafy-new"));
        assert!(fx.state_path.exists());
    }

    #[tokio::test]
    async fn max_per_account_limits_the_sum_of_an_accounts_sites() {
        let mut policy = default_policy();
        policy.max_per_account = 100;
        let kubo = FakeKubo::with(|s| {
            s.sizes.insert("bafy-b".into(), 60);
            s.sizes.insert("bafy-c".into(), 40);
        });
        let fx = Fixture::new(policy, kubo);
        fx.seed("a.example", "bafy-a", 60).await;

        fx.apply(fx.event("b.example", "bafy-b", None, 200)).await;
        assert!(!fx.kubo().pinned.contains("bafy-b"));

        fx.apply(fx.event("c.example", "bafy-c", None, 200)).await;
        assert!(fx.kubo().pinned.contains("bafy-c"));
        assert_eq!(fx.site_bytes("c.example").await, 40);
    }

    #[tokio::test]
    async fn rejected_cid_shared_with_another_site_stays_pinned() {
        let mut policy = default_policy();
        policy.max_per_account = 150;
        let kubo = FakeKubo::with(|s| {
            s.sizes.insert("bafy-shared".into(), 100);
        });
        let fx = Fixture::new(policy, kubo);
        fx.seed("a.example", "bafy-shared", 100).await;

        fx.apply(fx.event("b.example", "bafy-shared", None, 200))
            .await;

        assert!(fx.kubo().pinned.contains("bafy-shared"));
        assert_eq!(fx.site_bytes("a.example").await, 100);
        assert_eq!(fx.site_bytes("b.example").await, 0);
    }

    #[tokio::test]
    async fn unfollow_during_fetch_does_not_pin() {
        let kubo = FakeKubo::with(|s| {
            s.sizes.insert("bafy-new".into(), 10);
        });
        let fx = Fixture::new(default_policy(), kubo);
        let gate = fx.agent.ipfs.gate.write().await;

        let agent = Arc::clone(&fx.agent);
        let ev = fx.event(D, "bafy-new", None, 200);
        let task = tokio::spawn(async move { agent.apply_site_event(&ev).await });
        fx.agent.ipfs.entered_fetch.notified().await;
        fx.agent.replace_targets(HashSet::new());
        fx.agent.unfollow(fx.pubkey).await;
        drop(gate);
        task.await.unwrap();

        assert!(!fx.kubo().pinned.contains("bafy-new"));
        assert!(fx.agent.state.lock().await.sites.is_empty());
    }

    #[tokio::test]
    async fn submit_coalesces_events_for_a_busy_site_to_the_newest() {
        let kubo = FakeKubo::with(|s| {
            for cid in ["bafy-1", "bafy-2", "bafy-3", "bafy-other"] {
                s.sizes.insert(cid.into(), 10);
            }
        });
        let fx = Fixture::new(default_policy(), kubo);
        let gate = fx.agent.ipfs.gate.write().await;

        let mut tasks = JoinSet::new();
        fx.agent
            .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
        fx.agent
            .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
        fx.agent
            .submit(fx.event(D, "bafy-3", None, 300), &mut tasks);
        fx.agent
            .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
        fx.agent.submit(
            fx.event("other.example", "bafy-other", None, 100),
            &mut tasks,
        );
        assert_eq!(tasks.len(), 2);
        drop(gate);
        while let Some(joined) = tasks.join_next().await {
            joined.unwrap();
        }

        let mut fetched = fx.kubo().fetched.clone();
        fetched.sort();
        assert_eq!(fetched, vec!["bafy-1", "bafy-3", "bafy-other"]);
        assert!(fx.kubo().pinned.contains("bafy-3"));
        assert!(fx.agent.queue.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn submit_after_completion_runs_again() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let mut tasks = JoinSet::new();
        fx.agent
            .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
        while tasks.join_next().await.is_some() {}
        fx.agent
            .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
        while tasks.join_next().await.is_some() {}
        assert_eq!(fx.kubo().fetched, vec!["bafy-1", "bafy-2"]);
    }

    #[tokio::test]
    async fn unfollow_removes_verifications_of_unpinned_sites() {
        let nip05 = FakeNip05::default();
        let kubo = FakeKubo::default();
        let fx = Fixture::with_nip05(nip05_policy(Nip05Mode::Require), kubo, nip05);
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Mismatch);
        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;
        assert_eq!(fx.verification(D).await.as_deref(), Some("mismatch"));

        fx.agent.unfollow(fx.pubkey).await;
        assert_eq!(fx.verification(D).await, None);
    }

    #[tokio::test]
    async fn nip05_warn_mode_pins_despite_mismatch_and_records_result() {
        let fx = Fixture::new(
            nip05_policy(Nip05Mode::Warn),
            FakeKubo::with(|s| {
                s.sizes.insert("bafy-new".into(), 20);
            }),
        );
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Mismatch);

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().pinned.contains("bafy-new"));
        assert_eq!(fx.site_bytes(D).await, 20);
        assert_eq!(fx.verification(D).await.as_deref(), Some("mismatch"));
    }

    #[tokio::test]
    async fn nip05_require_mode_skips_fetch_when_not_verified() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Require), FakeKubo::default());
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::NotApplicable);

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().fetched.is_empty());
        assert!(!fx.agent.state.lock().await.sites.contains_key(&fx.key(D)));
        assert_eq!(fx.verification(D).await.as_deref(), Some("not_applicable"));
    }

    #[tokio::test]
    async fn nip05_require_mode_pins_when_verified() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Require), FakeKubo::default());
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Verified);

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().pinned.contains("bafy-new"));
        assert_eq!(fx.verification(D).await.as_deref(), Some("verified"));
    }

    #[tokio::test]
    async fn nip05_off_mode_never_calls_verifier() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Off), FakeKubo::default());

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().pinned.contains("bafy-new"));
        assert_eq!(fx.agent.nip05.calls(), 0);
        assert_eq!(fx.verification(D).await, None);
    }

    #[tokio::test]
    async fn nip05_result_is_cached_until_ttl_expires() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Require), FakeKubo::default());
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Verified);

        fx.apply(fx.event(D, "bafy-1", None, 200)).await;
        fx.apply(fx.event(D, "bafy-2", None, 300)).await;
        assert_eq!(fx.agent.nip05.calls(), 1);
        assert!(fx.kubo().pinned.contains("bafy-2"));

        fx.agent
            .state
            .lock()
            .await
            .verifications
            .get_mut(&fx.key(D))
            .unwrap()
            .checked_at -= 86_400;
        fx.apply(fx.event(D, "bafy-3", None, 400)).await;
        assert_eq!(fx.agent.nip05.calls(), 2);
    }

    #[tokio::test]
    async fn nip05_errors_are_cached_for_a_shorter_time() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Require), FakeKubo::default());
        fx.agent.nip05.set(
            D,
            &fx.pubkey.to_hex(),
            VerificationResult::Error("timeout".into()),
        );

        fx.apply(fx.event(D, "bafy-1", None, 200)).await;
        fx.apply(fx.event(D, "bafy-1", None, 200)).await;
        assert_eq!(fx.agent.nip05.calls(), 1);

        fx.agent
            .state
            .lock()
            .await
            .verifications
            .get_mut(&fx.key(D))
            .unwrap()
            .checked_at -= NIP05_ERROR_CACHE_TTL;
        fx.apply(fx.event(D, "bafy-1", None, 200)).await;
        assert_eq!(fx.agent.nip05.calls(), 2);
        assert!(fx.kubo().fetched.is_empty());
    }

    #[tokio::test]
    async fn skipped_events_do_not_trigger_nip05() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Require), FakeKubo::default());
        fx.seed(D, "bafy-1", 10).await;

        fx.apply(fx.event(D, "bafy-1", None, 100)).await;

        assert_eq!(fx.agent.nip05.calls(), 0);
    }
}
