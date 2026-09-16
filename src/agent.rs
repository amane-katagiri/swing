use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;
use futures_util::StreamExt;
use nostr_sdk::prelude::*;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::config::{Config, Nip05Mode};
use crate::ipfs::{FetchLimits, Fetched, IpfsClient, KuboStore};
use crate::mfs::MfsLayout;
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
    agent.reconcile().await;
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
                agent.sweep().await;
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

async fn refresh_follow_set<C, N>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N>>,
    tasks: &mut JoinSet<()>,
) where
    C: KuboStore + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
{
    let config = &agent.config;
    let (fetched, fetch_succeeded) = match relay.fetch_follow_set(&config.nostr.mirror_set).await {
        Ok(event) => (event, true),
        Err(e) => {
            warn!(error = %e, "fetching follow set failed");
            (None, false)
        }
    };
    let own = relay.keys.public_key();
    let choice = {
        let mut state = agent.state.lock().await;
        let stored = state
            .follow_set
            .clone()
            .filter(|ev| nostr::is_follow_set_of(ev, &own, &config.nostr.mirror_set));
        let choice = nostr::choose_follow_set(fetched, fetch_succeeded, stored);
        if let Some(choice) = &choice
            && choice.save
        {
            state.follow_set = Some(choice.event.clone());
            agent.save(&state, "follow set update").await;
        }
        choice
    };
    let Some(choice) = choice else {
        warn!(mirror_set = %config.nostr.mirror_set, "no follow set found yet; will retry");
        return;
    };
    if choice.republish {
        warn!(event_id = %choice.event.id, "relays returned an older follow set or none; republishing the saved one");
        match relay.publish_to_relays(&choice.event).await {
            Ok(output) if output.success.is_empty() => {
                warn!("no relay accepted the republished follow set")
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "republishing the follow set failed"),
        }
    }
    let follow_event = choice.event;

    let new_targets: HashSet<PublicKey> = nostr::extract_follow_set_pubkeys(&follow_event)
        .into_iter()
        .collect();
    agent.replace_targets(new_targets.clone());
    if config.policy.remove_on_unfollow {
        agent.remove_unfollowed().await;
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
            let latest = nostr::select_latest(&parsed).into_values().collect();
            let selected = {
                let state = agent.state.lock().await;
                limit_sites_per_account(latest, &state, config.policy.max_sites_per_account)
            };
            for ev in selected {
                agent.submit(ev, tasks);
            }
        }
        Err(e) => warn!(error = %e, "fetching historical site events failed"),
    }
}

fn limit_sites_per_account(
    events: Vec<SiteEvent>,
    state: &State,
    max_sites: usize,
) -> Vec<SiteEvent> {
    let mut by_account: HashMap<PublicKey, Vec<(bool, SiteEvent)>> = HashMap::new();
    for ev in events {
        let stored = state
            .sites
            .contains_key(&state::site_key(&ev.pubkey.to_hex(), &ev.d));
        by_account.entry(ev.pubkey).or_default().push((stored, ev));
    }
    let mut selected = Vec::new();
    for mut events in by_account.into_values() {
        events.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.created_at.cmp(&a.1.created_at)));
        let stored = events.iter().filter(|(s, _)| *s).count();
        selected.extend(
            events
                .into_iter()
                .take(stored.max(max_sites))
                .map(|(_, ev)| ev),
        );
    }
    selected
}

fn version_infos(state: &State, key: &SiteKey) -> Vec<VersionInfo> {
    state
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
        .unwrap_or_default()
}

fn decide(
    state: &State,
    key: &SiteKey,
    pubkey_hex: &str,
    ev: &SiteEvent,
    size: Option<u64>,
    config: &Config,
) -> Decision {
    let existing = version_infos(state, key);
    let site = state.site_bytes(key);
    let usage = Usage {
        other_sites: state.total_bytes() - site,
        other_sites_of_account: state.account_bytes(pubkey_hex) - site,
        other_site_count_of_account: state.account_site_count(pubkey_hex)
            - usize::from(state.sites.contains_key(key)),
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
    layout: MfsLayout,
    state: tokio::sync::Mutex<State>,
    state_path: PathBuf,
    targets: RwLock<HashSet<PublicKey>>,
    queue: Mutex<BTreeMap<SiteKey, Queued>>,
    permits: Semaphore,
}

impl<C, N> Agent<C, N>
where
    C: KuboStore + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
{
    fn submit(self: &Arc<Self>, ev: SiteEvent, tasks: &mut JoinSet<()>) {
        let pubkey_hex = ev.pubkey.to_hex();
        let key = state::site_key(&pubkey_hex, &ev.d);
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
            let prefix = format!("{pubkey_hex}:");
            let active = queue
                .range(prefix.clone()..)
                .take_while(|(k, _)| k.starts_with(&prefix))
                .count();
            if active >= self.config.policy.max_sites_per_account {
                debug!(site_key = %key, "too many sites of this account in progress; dropping until next poll");
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

impl<C: KuboStore, N: Nip05Verify> Agent<C, N> {
    fn new(config: Config, ipfs: C, nip05: N, state: State, state_path: PathBuf) -> Self {
        let permits = Semaphore::new(config.agent.concurrency);
        let layout = MfsLayout::new(config.ipfs.mfs_root.clone());
        Self {
            layout,
            config,
            ipfs,
            nip05,
            state: tokio::sync::Mutex::new(state),
            state_path,
            targets: RwLock::new(HashSet::new()),
            queue: Mutex::new(BTreeMap::new()),
            permits,
        }
    }

    fn is_target(&self, pubkey: &PublicKey) -> bool {
        self.targets.read().unwrap().contains(pubkey)
    }

    fn replace_targets(&self, new_targets: HashSet<PublicKey>) {
        *self.targets.write().unwrap() = new_targets;
    }

    async fn save(&self, state: &State, after: &str) {
        if let Err(e) = state.save(&self.state_path).await {
            error!(error = %e, "saving state after {after} failed");
        }
    }

    fn version_path(&self, key: &str, created_at: u64) -> Option<String> {
        let (pubkey_hex, d) = state::split_site_key(key)?;
        Some(self.layout.agent_version(pubkey_hex, d, created_at))
    }

    async fn remove_path(&self, path: &str) {
        match self.ipfs.mfs_remove(path).await {
            Ok(()) => info!(path = %path, "removed from MFS"),
            Err(e) => {
                warn!(path = %path, error = %e, "removing from MFS failed; the next sweep retries")
            }
        }
    }

    async fn remove_versions(&self, key: &str, versions: &[VersionRecord]) {
        for v in versions {
            if let Some(path) = self.version_path(key, v.created_at) {
                self.remove_path(&path).await;
            }
        }
    }

    async fn sweep(&self) {
        let mut state = self.state.lock().await;
        let now = now_secs();
        let keys: Vec<SiteKey> = state.sites.keys().cloned().collect();
        let mut removed = Vec::new();
        for key in keys {
            let cids =
                policy::retention_evictions(&version_infos(&state, &key), &self.config.policy, now);
            if !cids.is_empty() {
                info!(site_key = %key, count = cids.len(), "evicting versions past retention");
                removed.push((key.clone(), state.remove_versions(&key, &cids)));
            }
        }
        if !removed.is_empty() {
            self.save(&state, "retention").await;
            for (key, versions) in &removed {
                self.remove_versions(key, versions).await;
            }
        }
        self.collect_garbage(&state).await;
    }

    // Everything under the agent root belongs to SWING, so any entry the
    // state does not reference is a leftover of a failed or interrupted step.
    async fn collect_garbage(&self, state: &State) {
        let expected: HashSet<String> = state
            .sites
            .iter()
            .flat_map(|(key, versions)| {
                versions
                    .iter()
                    .filter_map(|v| self.version_path(key, v.created_at))
            })
            .collect();
        let root = self.layout.agent_root();
        let Some(accounts) = self.list_dir(&root).await else {
            return;
        };
        for account in accounts {
            let account_path = format!("{root}/{}", account.name);
            if !account.is_dir {
                self.remove_path(&account_path).await;
                continue;
            }
            let Some(sites) = self.list_dir(&account_path).await else {
                continue;
            };
            let mut account_kept = false;
            for site in sites {
                let site_path = format!("{account_path}/{}", site.name);
                if !site.is_dir {
                    self.remove_path(&site_path).await;
                    continue;
                }
                let Some(versions) = self.list_dir(&site_path).await else {
                    account_kept = true;
                    continue;
                };
                let mut site_kept = false;
                for version in versions {
                    let path = format!("{site_path}/{}", version.name);
                    if expected.contains(&path) {
                        site_kept = true;
                    } else {
                        self.remove_path(&path).await;
                    }
                }
                if site_kept {
                    account_kept = true;
                } else {
                    self.remove_path(&site_path).await;
                }
            }
            if !account_kept {
                self.remove_path(&account_path).await;
            }
        }
    }

    async fn list_dir(&self, path: &str) -> Option<Vec<crate::ipfs::MfsEntry>> {
        match self.ipfs.mfs_list(path).await {
            Ok(entries) => Some(entries),
            Err(e) => {
                warn!(path = %path, error = %e, "listing MFS failed");
                None
            }
        }
    }

    async fn reconcile(&self) {
        let mut state = self.state.lock().await;
        let mut missing = Vec::new();
        for (key, versions) in &state.sites {
            for v in versions {
                let Some(path) = self.version_path(key, v.created_at) else {
                    continue;
                };
                let problem = match self.ipfs.mfs_stat_cid(&path).await {
                    Ok(Some(cid)) if cid == v.cid => match self.ipfs.dag_size_local(&v.cid).await {
                        Ok(_) => None,
                        Err(e) => Some(format!("content is incomplete: {e}")),
                    },
                    Ok(Some(cid)) => Some(format!("MFS entry points to {cid}")),
                    Ok(None) => Some("MFS entry is missing".to_string()),
                    Err(e) => {
                        warn!(path = %path, error = %e, "checking MFS failed; keeping the version");
                        None
                    }
                };
                if let Some(problem) = problem {
                    warn!(site_key = %key, cid = %v.cid, problem = %problem, "forgetting the version so it is fetched again");
                    missing.push((key.clone(), v.cid.clone()));
                }
            }
        }
        if missing.is_empty() {
            return;
        }
        for (key, cid) in &missing {
            state.remove_versions(key, std::slice::from_ref(cid));
        }
        self.save(&state, "reconciliation").await;
    }

    // Compared against the state rather than the previous follow set, so
    // accounts dropped while the agent was stopped are removed too.
    async fn remove_unfollowed(&self) {
        let targets: HashSet<String> = self
            .targets
            .read()
            .unwrap()
            .iter()
            .map(|pk| pk.to_hex())
            .collect();
        let mut state = self.state.lock().await;
        let unfollowed: Vec<String> = state
            .accounts()
            .into_iter()
            .filter(|pubkey_hex| !targets.contains(pubkey_hex))
            .collect();
        if unfollowed.is_empty() {
            return;
        }
        for pubkey_hex in &unfollowed {
            for key in state.remove_account(pubkey_hex) {
                info!(site_key = %key, "unfollowed");
            }
        }
        self.save(&state, "unfollow").await;
        for pubkey_hex in &unfollowed {
            self.remove_path(&self.layout.agent_account(pubkey_hex))
                .await;
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
        state.prune_unstored_verifications(pubkey_hex, self.config.policy.max_sites_per_account);
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
        if precheck.store.is_none() {
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
            total: self.config.agent.fetch_timeout,
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
            info!(site = %ev.d, pubkey = %pubkey_hex, "author left the follow set during fetch; not storing");
            return;
        }
        let path = self.layout.agent_version(&pubkey_hex, &ev.d, ev.created_at);
        if let Err(e) = self.ipfs.mfs_put(&ev.cid, &path).await {
            error!(cid = %ev.cid, path = %path, error = %e, "storing into MFS failed");
            return;
        }
        let size = match self.ipfs.dag_size_local(&ev.cid).await {
            Ok(size) => size,
            Err(e) => {
                warn!(cid = %ev.cid, error = %e, "content is incomplete after fetch; will retry on next poll");
                self.remove_path(&path).await;
                return;
            }
        };
        if let Some(declared) = ev.size
            && declared < size
        {
            warn!(cid = %ev.cid, site = %ev.d, declared, actual = size, "size tag understates the content");
        }

        let decision = decide(&state, &key, &pubkey_hex, ev, Some(size), &self.config);
        let Some(cid) = decision.store else {
            warn!(cid = %ev.cid, site = %ev.d, size, reason = %decision.reason, "rejected after fetch");
            self.remove_path(&path).await;
            return;
        };
        state.apply_store(
            &key,
            VersionRecord {
                cid: cid.clone(),
                size,
                created_at: ev.created_at,
                stored_at: now_secs(),
            },
        );
        let evicted = state.remove_versions(&key, &decision.evict);
        self.save(&state, "store").await;
        self.remove_versions(&key, &evicted).await;
        info!(cid = %cid, site = %ev.d, pubkey = %pubkey_hex, size, "stored");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::config::{AgentConfig, IpfsConfig, NostrConfig, PolicyConfig};
    use crate::ipfs::MfsEntry;
    use crate::nip05::VerificationResult;

    #[derive(Default)]
    struct FakeKuboState {
        mfs: BTreeMap<String, String>,
        put_calls: Vec<String>,
        fetched: Vec<String>,
        fail_fetch: HashSet<String>,
        fail_stat: HashSet<String>,
        fail_put: HashSet<String>,
        fail_remove: HashSet<String>,
        sizes: HashMap<String, u64>,
    }

    impl FakeKuboState {
        fn stores(&self, cid: &str) -> bool {
            self.mfs.values().any(|c| c == cid)
        }

        fn paths(&self) -> Vec<String> {
            self.mfs.keys().cloned().collect()
        }
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

    impl KuboStore for FakeKubo {
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

        async fn dag_size_local(&self, cid: &str) -> Result<u64> {
            let s = self.s.lock().unwrap();
            if s.fail_stat.contains(cid) {
                anyhow::bail!("simulated dag/stat failure");
            }
            Ok(s.sizes.get(cid).copied().unwrap_or(0))
        }

        async fn mfs_put(&self, cid: &str, path: &str) -> Result<()> {
            let mut s = self.s.lock().unwrap();
            s.put_calls.push(cid.to_string());
            if s.fail_put.contains(cid) {
                anyhow::bail!("simulated files/cp failure");
            }
            s.mfs.insert(path.to_string(), cid.to_string());
            Ok(())
        }

        async fn mfs_remove(&self, path: &str) -> Result<()> {
            let mut s = self.s.lock().unwrap();
            if s.fail_remove.contains(path) {
                anyhow::bail!("simulated files/rm failure");
            }
            let prefix = format!("{path}/");
            s.mfs.retain(|p, _| p != path && !p.starts_with(&prefix));
            Ok(())
        }

        async fn mfs_list(&self, path: &str) -> Result<Vec<MfsEntry>> {
            let s = self.s.lock().unwrap();
            let prefix = format!("{path}/");
            let mut entries: BTreeMap<String, MfsEntry> = BTreeMap::new();
            for (p, cid) in &s.mfs {
                let Some(rest) = p.strip_prefix(&prefix) else {
                    continue;
                };
                let (name, is_dir) = match rest.split_once('/') {
                    Some((name, _)) => (name, true),
                    None => (rest, false),
                };
                entries.insert(
                    name.to_string(),
                    MfsEntry {
                        name: name.to_string(),
                        is_dir,
                        cid: if is_dir { String::new() } else { cid.clone() },
                    },
                );
            }
            Ok(entries.into_values().collect())
        }

        async fn mfs_stat_cid(&self, path: &str) -> Result<Option<String>> {
            Ok(self.s.lock().unwrap().mfs.get(path).cloned())
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
                mfs_root: "/swing".to_string(),
            },
            policy,
            agent: AgentConfig {
                state_dir: std::path::PathBuf::from("./data"),
                poll_interval: Duration::from_secs(300),
                fetch_timeout: Duration::from_secs(60),
                fetch_idle_timeout: Duration::from_secs(10),
                concurrency: 2,
            },
            publish: crate::config::PublishConfig {
                nip05: Nip05Mode::Off,
                keep_versions: 5,
            },
        }
    }

    fn default_policy() -> PolicyConfig {
        PolicyConfig {
            max_total_storage: 1_000_000,
            max_per_site: 100_000,
            max_per_account: 1_000_000,
            max_sites_per_account: 10,
            max_update_size: 100_000,
            keep_versions: 5,
            keep_days: 365,
            min_update_interval: 0,
            remove_on_unfollow: true,
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
            let dir = tempfile::tempdir().unwrap();
            let state_path = dir.path().join("state.json");
            let pubkey = Keys::generate().public_key();
            let agent = Agent::new(
                test_config(policy),
                kubo,
                FakeNip05::default(),
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

        fn path(&self, d: &str, created_at: u64) -> String {
            self.agent
                .layout
                .agent_version(&self.pubkey.to_hex(), d, created_at)
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

        async fn seed(&self, d: &str, cid: &str, size: u64, created_at: u64) {
            self.agent.state.lock().await.apply_store(
                &self.key(d),
                VersionRecord {
                    cid: cid.to_string(),
                    size,
                    created_at,
                    stored_at: created_at,
                },
            );
            let path = self.path(d, created_at);
            let mut kubo = self.kubo();
            kubo.mfs.insert(path, cid.to_string());
            kubo.sizes.insert(cid.to_string(), size);
        }

        async fn apply(&self, ev: SiteEvent) {
            self.agent.apply_site_event(&ev).await;
        }

        fn kubo(&self) -> std::sync::MutexGuard<'_, FakeKuboState> {
            self.agent.ipfs.s.lock().unwrap()
        }

        async fn cids(&self, d: &str) -> Vec<String> {
            self.agent
                .state
                .lock()
                .await
                .sites
                .get(&self.key(d))
                .map(|vs| vs.iter().map(|v| v.cid.clone()).collect())
                .unwrap_or_default()
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

    fn sized(entries: &[(&str, u64)]) -> FakeKubo {
        FakeKubo::with(|s| {
            for (cid, size) in entries {
                s.sizes.insert(cid.to_string(), *size);
            }
        })
    }

    #[tokio::test]
    async fn stores_a_new_site_under_its_versioned_path() {
        let fx = Fixture::new(default_policy(), sized(&[("bafy-new", 20)]));

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert_eq!(fx.kubo().paths(), vec![fx.path(D, 200)]);
        assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
        assert_eq!(fx.site_bytes(D).await, 20);
        assert!(fx.state_path.exists());
    }

    #[tokio::test]
    async fn fetch_failure_leaves_state_and_old_versions_untouched() {
        let kubo = FakeKubo::with(|s| {
            s.fail_fetch.insert("bafy-new".into());
        });
        let fx = Fixture::new(default_policy(), kubo);
        fx.seed(D, "bafy-old", 10, 100).await;

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert_eq!(fx.cids(D).await, vec!["bafy-old"]);
        assert_eq!(fx.kubo().paths(), vec![fx.path(D, 100)]);
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
    async fn store_failure_records_nothing() {
        let kubo = FakeKubo::with(|s| {
            s.fail_put.insert("bafy-new".into());
        });
        let fx = Fixture::new(default_policy(), kubo);

        fx.apply(fx.event(D, "bafy-new", None, 200)).await;

        assert!(fx.cids(D).await.is_empty());
        assert!(fx.kubo().mfs.is_empty());
    }

    #[tokio::test]
    async fn incomplete_content_is_removed_and_not_recorded() {
        let kubo = FakeKubo::with(|s| {
            s.fail_stat.insert("bafy-new".into());
        });
        let fx = Fixture::new(default_policy(), kubo);

        fx.apply(fx.event(D, "bafy-new", None, 200)).await;

        assert!(fx.cids(D).await.is_empty());
        assert!(fx.kubo().mfs.is_empty());
    }

    #[tokio::test]
    async fn oversized_content_is_aborted_during_fetch_even_with_a_small_size_tag() {
        let mut policy = default_policy();
        policy.max_per_site = 50;
        let fx = Fixture::new(policy, sized(&[("bafy-liar", 1_000)]));

        fx.apply(fx.event(D, "bafy-liar", Some(20), 200)).await;

        assert_eq!(fx.kubo().fetched, vec!["bafy-liar".to_string()]);
        assert!(fx.kubo().put_calls.is_empty());
        assert!(fx.cids(D).await.is_empty());
    }

    #[tokio::test]
    async fn actual_size_is_recorded_and_rechecked_instead_of_the_size_tag() {
        let mut policy = default_policy();
        policy.max_total_storage = 100;
        let fx = Fixture::new(policy, sized(&[("bafy-liar", 80), ("bafy-honest", 30)]));
        fx.seed("other.example", "bafy-other", 40, 100).await;

        fx.apply(fx.event(D, "bafy-liar", Some(1), 200)).await;
        assert!(!fx.kubo().stores("bafy-liar"));
        assert_eq!(fx.site_bytes(D).await, 0);

        fx.apply(fx.event(D, "bafy-honest", Some(1), 300)).await;
        assert!(fx.kubo().stores("bafy-honest"));
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
    async fn new_version_evicts_the_oldest_per_keep_versions() {
        let mut policy = default_policy();
        policy.keep_versions = 1;
        let fx = Fixture::new(policy, sized(&[("bafy-new", 20)]));
        fx.seed(D, "bafy-old", 10, 100).await;

        fx.apply(fx.event(D, "bafy-new", None, 200)).await;

        assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
        assert_eq!(fx.kubo().paths(), vec![fx.path(D, 200)]);
        assert_eq!(fx.site_bytes(D).await, 20);
    }

    #[tokio::test]
    async fn max_per_account_limits_the_sum_of_an_accounts_sites() {
        let mut policy = default_policy();
        policy.max_per_account = 100;
        let fx = Fixture::new(policy, sized(&[("bafy-b", 60), ("bafy-c", 40)]));
        fx.seed("a.example", "bafy-a", 60, 100).await;

        fx.apply(fx.event("b.example", "bafy-b", None, 200)).await;
        assert!(!fx.kubo().stores("bafy-b"));

        fx.apply(fx.event("c.example", "bafy-c", None, 200)).await;
        assert!(fx.kubo().stores("bafy-c"));
        assert_eq!(fx.site_bytes("c.example").await, 40);
    }

    #[tokio::test]
    async fn sites_sharing_a_cid_are_stored_and_removed_independently() {
        let mut policy = default_policy();
        policy.keep_versions = 1;
        let fx = Fixture::new(policy, sized(&[("bafy-shared", 10), ("bafy-a2", 10)]));

        fx.apply(fx.event("a.example", "bafy-shared", None, 200))
            .await;
        fx.apply(fx.event("b.example", "bafy-shared", None, 200))
            .await;
        fx.apply(fx.event("a.example", "bafy-a2", None, 300)).await;

        assert_eq!(
            fx.kubo().paths(),
            vec![fx.path("a.example", 300), fx.path("b.example", 200)]
        );
        assert!(fx.kubo().stores("bafy-shared"));
    }

    #[tokio::test]
    async fn unfollow_during_fetch_does_not_store() {
        let fx = Fixture::new(default_policy(), sized(&[("bafy-new", 10)]));
        let gate = fx.agent.ipfs.gate.write().await;

        let agent = Arc::clone(&fx.agent);
        let ev = fx.event(D, "bafy-new", None, 200);
        let task = tokio::spawn(async move { agent.apply_site_event(&ev).await });
        fx.agent.ipfs.entered_fetch.notified().await;
        fx.agent.replace_targets(HashSet::new());
        fx.agent.remove_unfollowed().await;
        drop(gate);
        task.await.unwrap();

        assert!(fx.kubo().mfs.is_empty());
        assert!(fx.agent.state.lock().await.sites.is_empty());
    }

    #[tokio::test]
    async fn unfollow_removes_the_account_directory_and_its_verifications() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Warn), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        fx.seed("b.example", "bafy-b", 1, 100).await;
        fx.agent.nip05.set(
            "c.example",
            &fx.pubkey.to_hex(),
            VerificationResult::Mismatch,
        );
        fx.apply(fx.event("c.example", "bafy-c", None, 100)).await;
        let other = format!("{}/other", fx.agent.layout.agent_root());
        fx.kubo()
            .mfs
            .insert(format!("{other}/x/1"), "bafy-x".into());

        fx.agent.replace_targets(HashSet::new());
        fx.agent.remove_unfollowed().await;

        assert_eq!(fx.kubo().paths(), vec![format!("{other}/x/1")]);
        let state = fx.agent.state.lock().await;
        assert!(state.sites.is_empty());
        assert!(state.verifications.is_empty());
    }

    #[tokio::test]
    async fn accounts_dropped_while_stopped_are_removed_on_the_first_refresh() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let kept = Keys::generate().public_key();
        fx.seed(D, "bafy-gone", 1, 100).await;
        let kept_key = state::site_key(&kept.to_hex(), D);
        let kept_path = fx.agent.layout.agent_version(&kept.to_hex(), D, 100);
        fx.agent.state.lock().await.apply_store(
            &kept_key,
            VersionRecord {
                cid: "bafy-kept".into(),
                size: 1,
                created_at: 100,
                stored_at: 100,
            },
        );
        fx.kubo().mfs.insert(kept_path.clone(), "bafy-kept".into());

        fx.agent.replace_targets(HashSet::from([kept]));
        fx.agent.remove_unfollowed().await;

        let state = fx.agent.state.lock().await;
        assert_eq!(
            state.sites.keys().cloned().collect::<Vec<_>>(),
            vec![kept_key]
        );
        drop(state);
        assert_eq!(fx.kubo().paths(), vec![kept_path]);
        assert!(fx.state_path.exists());
    }

    #[tokio::test]
    async fn remove_unfollowed_without_changes_does_not_write_state() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        fx.agent.remove_unfollowed().await;
        assert!(!fx.state_path.exists());
        assert_eq!(fx.cids(D).await, vec!["bafy-a"]);
    }

    #[tokio::test]
    async fn submit_coalesces_events_for_a_busy_site_to_the_newest() {
        let fx = Fixture::new(
            default_policy(),
            sized(&[
                ("bafy-1", 10),
                ("bafy-2", 10),
                ("bafy-3", 10),
                ("bafy-other", 10),
            ]),
        );
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
        assert!(fx.kubo().stores("bafy-3"));
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
    async fn nip05_warn_mode_stores_despite_mismatch_and_records_result() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Warn), sized(&[("bafy-new", 20)]));
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Mismatch);

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().stores("bafy-new"));
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
        assert!(fx.cids(D).await.is_empty());
        assert_eq!(fx.verification(D).await.as_deref(), Some("not_applicable"));
    }

    #[tokio::test]
    async fn nip05_require_mode_stores_when_verified() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Require), FakeKubo::default());
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Verified);

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().stores("bafy-new"));
        assert_eq!(fx.verification(D).await.as_deref(), Some("verified"));
    }

    #[tokio::test]
    async fn nip05_off_mode_never_calls_verifier() {
        let fx = Fixture::new(nip05_policy(Nip05Mode::Off), FakeKubo::default());

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().stores("bafy-new"));
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
        assert!(fx.kubo().stores("bafy-2"));

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
        fx.seed(D, "bafy-1", 10, 100).await;

        fx.apply(fx.event(D, "bafy-1", None, 100)).await;

        assert_eq!(fx.agent.nip05.calls(), 0);
    }

    #[tokio::test]
    async fn max_sites_per_account_skips_new_sites_before_fetching() {
        let mut policy = default_policy();
        policy.max_sites_per_account = 2;
        let fx = Fixture::new(policy, FakeKubo::default());
        fx.seed("a.example", "bafy-a", 1, 100).await;
        fx.seed("b.example", "bafy-b", 1, 100).await;

        fx.apply(fx.event("c.example", "bafy-c", None, 200)).await;
        assert!(fx.kubo().fetched.is_empty());

        fx.apply(fx.event("a.example", "bafy-a2", None, 200)).await;
        assert_eq!(fx.kubo().fetched, vec!["bafy-a2"]);
    }

    fn stored_state(fx: &Fixture, ds: &[&str]) -> State {
        let mut state = State::default();
        for d in ds {
            state.apply_store(
                &fx.key(d),
                VersionRecord {
                    cid: "c".into(),
                    size: 1,
                    created_at: 1,
                    stored_at: 1,
                },
            );
        }
        state
    }

    #[test]
    fn limit_sites_per_account_prefers_stored_then_newest() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let state = stored_state(&fx, &["stored.example"]);
        let mut other_event = fx.event("x.example", "c", None, 1);
        other_event.pubkey = Keys::generate().public_key();
        let events = vec![
            fx.event("old.example", "c", None, 10),
            fx.event("stored.example", "c", None, 5),
            fx.event("new.example", "c", None, 30),
            fx.event("mid.example", "c", None, 20),
            other_event,
        ];

        let mut selected: Vec<String> = limit_sites_per_account(events, &state, 3)
            .into_iter()
            .map(|e| e.d)
            .collect();
        selected.sort();
        assert_eq!(
            selected,
            vec!["mid.example", "new.example", "stored.example", "x.example"]
        );
    }

    #[test]
    fn limit_sites_per_account_keeps_all_stored_sites_over_the_limit() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let state = stored_state(&fx, &["a.example", "b.example"]);
        let events = vec![
            fx.event("a.example", "c", None, 1),
            fx.event("b.example", "c", None, 1),
            fx.event("new.example", "c", None, 99),
        ];

        let mut selected: Vec<String> = limit_sites_per_account(events, &state, 1)
            .into_iter()
            .map(|e| e.d)
            .collect();
        selected.sort();
        assert_eq!(selected, vec!["a.example", "b.example"]);
    }

    #[tokio::test]
    async fn submit_caps_in_progress_sites_per_account() {
        let mut policy = default_policy();
        policy.max_sites_per_account = 2;
        let fx = Fixture::new(policy, FakeKubo::default());
        let gate = fx.agent.ipfs.gate.write().await;

        let mut tasks = JoinSet::new();
        for d in ["a.example", "b.example", "c.example"] {
            fx.agent.submit(fx.event(d, "bafy", None, 100), &mut tasks);
        }
        fx.agent
            .submit(fx.event("a.example", "bafy-a2", None, 200), &mut tasks);
        assert_eq!(tasks.len(), 2);
        drop(gate);
        while let Some(joined) = tasks.join_next().await {
            joined.unwrap();
        }
        let mut fetched = fx.kubo().fetched.clone();
        fetched.sort();
        assert_eq!(fetched, vec!["bafy", "bafy", "bafy-a2"]);
    }

    #[tokio::test]
    async fn verifications_of_unstored_sites_are_pruned_per_account() {
        let mut policy = nip05_policy(Nip05Mode::Require);
        policy.max_sites_per_account = 2;
        let fx = Fixture::new(policy, FakeKubo::default());
        for d in ["a.example", "b.example", "c.example"] {
            fx.agent
                .nip05
                .set(d, &fx.pubkey.to_hex(), VerificationResult::Mismatch);
            fx.apply(fx.event(d, "bafy", None, 200)).await;
        }
        assert_eq!(fx.agent.state.lock().await.verifications.len(), 2);
    }

    #[tokio::test]
    async fn failed_removal_is_cleaned_up_by_the_next_sweep() {
        let mut policy = default_policy();
        policy.keep_versions = 1;
        let fx = Fixture::new(policy, FakeKubo::default());
        fx.seed(D, "bafy-old", 10, 100).await;
        let old_path = fx.path(D, 100);
        fx.kubo().fail_remove.insert(old_path.clone());

        fx.apply(fx.event(D, "bafy-new", None, 200)).await;
        assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
        assert!(fx.kubo().mfs.contains_key(&old_path));

        fx.kubo().fail_remove.clear();
        fx.agent.sweep().await;
        assert_eq!(fx.kubo().paths(), vec![fx.path(D, 200)]);
    }

    #[tokio::test]
    async fn sweep_removes_entries_the_state_does_not_reference() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-kept", 1, 100).await;
        let root = fx.agent.layout.agent_root();
        let account = fx.agent.layout.agent_account(&fx.pubkey.to_hex());
        let publish = fx.agent.layout.publish_version(&fx.pubkey.to_hex(), D, 1);
        {
            let mut kubo = fx.kubo();
            kubo.mfs.insert(fx.path(D, 50), "bafy-stale".into());
            kubo.mfs
                .insert(fx.path("gone.example", 1), "bafy-gone".into());
            kubo.mfs
                .insert(format!("{account}/stray-file"), "bafy-f".into());
            kubo.mfs
                .insert(format!("{root}/unknown/a/1"), "bafy-u".into());
            kubo.mfs.insert(format!("{root}/loose"), "bafy-l".into());
            kubo.mfs.insert(publish.clone(), "bafy-p".into());
            kubo.mfs.insert("/manual/x".into(), "bafy-m".into());
        }

        fx.agent.sweep().await;

        let mut expected = vec![fx.path(D, 100), publish, "/manual/x".to_string()];
        expected.sort();
        assert_eq!(fx.kubo().paths(), expected);
    }

    #[tokio::test]
    async fn sweep_applies_retention_to_idle_sites() {
        let mut policy = default_policy();
        policy.keep_days = 1;
        let fx = Fixture::new(policy, FakeKubo::default());
        let now = now_secs();
        fx.seed(D, "bafy-old", 1, now - 3 * 86_400).await;
        fx.seed(D, "bafy-new", 1, now - 2 * 86_400).await;

        fx.agent.sweep().await;

        assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
        assert_eq!(fx.kubo().paths(), vec![fx.path(D, now - 2 * 86_400)]);
        assert!(fx.state_path.exists());
    }

    #[tokio::test]
    async fn sweep_without_retention_work_does_not_write_state() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        fx.agent.sweep().await;
        assert!(!fx.state_path.exists());
        assert_eq!(fx.kubo().paths(), vec![fx.path(D, 100)]);
    }

    #[tokio::test]
    async fn reconcile_forgets_missing_mismatched_and_incomplete_versions() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-kept", 1, 100).await;
        fx.seed("missing.example", "bafy-missing", 1, 100).await;
        fx.seed("moved.example", "bafy-moved", 1, 100).await;
        fx.seed("broken.example", "bafy-broken", 1, 100).await;
        {
            let mut kubo = fx.kubo();
            kubo.mfs.remove(&fx.path("missing.example", 100));
            kubo.mfs
                .insert(fx.path("moved.example", 100), "bafy-other".into());
            kubo.fail_stat.insert("bafy-broken".into());
        }

        fx.agent.reconcile().await;

        let state = fx.agent.state.lock().await;
        assert_eq!(
            state.sites.keys().cloned().collect::<Vec<_>>(),
            vec![fx.key(D)]
        );
        drop(state);
        assert!(fx.state_path.exists());

        fx.kubo().fail_stat.clear();
        fx.apply(fx.event("broken.example", "bafy-broken", None, 100))
            .await;
        assert_eq!(fx.cids("broken.example").await, vec!["bafy-broken"]);
    }

    #[tokio::test]
    async fn reconcile_without_problems_does_not_write_state() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-kept", 1, 100).await;
        fx.agent.reconcile().await;
        assert!(!fx.state_path.exists());
    }

    // Requires the local Kubo used by tests/kubo_integration.rs:
    //   cargo test --lib agent_stores_and_removes_through_real_kubo -- --ignored
    #[tokio::test]
    #[ignore]
    async fn agent_stores_and_removes_through_real_kubo() {
        let api = std::env::var("SWING_TEST_IPFS_API")
            .unwrap_or_else(|_| "http://127.0.0.1:15001".to_string());
        let kubo = IpfsClient::new(api);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = format!("/swing-test-agent-{nanos}");
        let site = tempfile::tempdir().unwrap();
        std::fs::write(site.path().join("index.html"), format!("{nanos}")).unwrap();
        let cid = kubo
            .add_dir(site.path(), &format!("{root}/origin"))
            .await
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let mut policy = default_policy();
        policy.keep_versions = 1;
        let mut config = test_config(policy);
        config.ipfs.mfs_root = root.clone();
        let agent = Agent::new(
            config,
            kubo,
            FakeNip05::default(),
            State::default(),
            dir.path().join("state.json"),
        );
        let pubkey = Keys::generate().public_key();
        agent.replace_targets(HashSet::from([pubkey]));
        let ev = SiteEvent {
            pubkey,
            d: "a/b.example".to_string(),
            cid: cid.clone(),
            url: None,
            size: None,
            created_at: 100,
        };

        agent.apply_site_event(&ev).await;
        let path = agent.layout.agent_version(&pubkey.to_hex(), &ev.d, 100);
        assert_eq!(
            agent.ipfs.mfs_stat_cid(&path).await.unwrap(),
            Some(cid.clone())
        );
        let size = agent
            .state
            .lock()
            .await
            .site_bytes(&state::site_key(&pubkey.to_hex(), &ev.d));
        assert!(size > 0);

        agent.reconcile().await;
        agent.sweep().await;
        assert!(agent.ipfs.mfs_stat_cid(&path).await.unwrap().is_some());

        agent
            .ipfs
            .mfs_put(
                &cid,
                &agent.layout.agent_version(&pubkey.to_hex(), "junk", 1),
            )
            .await
            .unwrap();
        agent.sweep().await;
        let sites = agent
            .ipfs
            .mfs_list(&agent.layout.agent_account(&pubkey.to_hex()))
            .await
            .unwrap();
        assert_eq!(sites.len(), 1);

        agent.replace_targets(HashSet::new());
        agent.remove_unfollowed().await;
        assert!(
            agent
                .ipfs
                .mfs_list(&agent.layout.agent_root())
                .await
                .unwrap()
                .is_empty()
        );
        agent.ipfs.mfs_remove(&root).await.unwrap();
    }
}
