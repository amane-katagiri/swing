mod anchors;
mod follow;
mod lifecycle;
mod replicas;
mod resend;
mod store;
#[cfg(test)]
mod test_support;

pub use lifecycle::run_until;

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use nostr_sdk::prelude::*;
use tokio::sync::Semaphore;
use tokio::task::{AbortHandle, JoinSet};
use tracing::{debug, info, warn};

use crate::activity::Activity;
use crate::config::Config;
use crate::health;
use crate::ipfs::{KuboNode, KuboStore};
use crate::mfs::MfsLayout;
use crate::nip05::Nip05Verify;
use crate::nostr::{AnchorRelay, RelayClient, ReportRelay};
use crate::policy;
use crate::state::{SiteKey, State, VersionRecord};

use anchors::AnchorAgentState;
use replicas::ReportBook;
use store::Queued;

fn now_secs() -> u64 {
    Timestamp::now().as_secs()
}

async fn poll_once<C, N, R>(
    relay: &Arc<RelayClient>,
    agent: &Arc<Agent<C, N, R>>,
    tasks: &mut JoinSet<()>,
) where
    C: KuboStore + KuboNode + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + AnchorRelay + Send + Sync + 'static,
{
    agent.sweep().await;
    let targets = follow::refresh_follow_set(relay, agent, tasks).await;
    agent.start_report_round(relay, targets, tasks);
    agent.start_anchor_refresh(tasks);
}

impl<C, N, R> Agent<C, N, R>
where
    C: KuboStore + KuboNode + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + AnchorRelay + Send + Sync + 'static,
{
    // The resend runs in the report round's task so its fetches never overlap the reports'.
    fn start_report_round(
        self: &Arc<Self>,
        relay: &Arc<RelayClient>,
        targets: Option<Vec<PublicKey>>,
        tasks: &mut JoinSet<()>,
    ) {
        let resend = targets.map(|targets| {
            let agent = Arc::clone(self);
            let relay = Arc::clone(relay);
            async move { resend::resend_held_sites(&relay, &agent, &targets).await }
        });
        self.begin_report_round(tokio::time::Instant::now(), resend, tasks);
    }

    fn begin_report_round(
        self: &Arc<Self>,
        now: tokio::time::Instant,
        resend: Option<impl Future<Output = ()> + Send + 'static>,
        tasks: &mut JoinSet<()>,
    ) -> bool {
        let mut round = self.report_round.lock().unwrap();
        if round.as_ref().is_some_and(|r| !r.is_finished()) {
            debug!("the previous replica report round is still running; skipping this one");
            return false;
        }
        let resend = resend.filter(|_| self.resend.lock().unwrap().take_due(now));
        let agent = Arc::clone(self);
        *round = Some(tasks.spawn(async move {
            agent.sync_reports_in_round().await;
            agent.record_replica_reports().await;
            if let Some(resend) = resend {
                resend.await;
            }
        }));
        true
    }

    fn start_anchor_check(self: &Arc<Self>, tasks: &mut JoinSet<()>) {
        let mut check = self.anchors.check.lock().unwrap();
        if check.as_ref().is_some_and(|c| !c.is_finished()) {
            return;
        }
        let agent = Arc::clone(self);
        *check = Some(tasks.spawn(async move { agent.check_anchor_ad().await }));
    }

    fn start_anchor_refresh(self: &Arc<Self>, tasks: &mut JoinSet<()>) -> bool {
        let mut refresh = self.anchors.refresh.lock().unwrap();
        if refresh.as_ref().is_some_and(|r| !r.is_finished()) {
            debug!("the previous anchor refresh is still running; skipping this one");
            return false;
        }
        let agent = Arc::clone(self);
        *refresh = Some(tasks.spawn(async move {
            let targets: Vec<PublicKey> = agent.targets.read().unwrap().iter().copied().collect();
            agent.refresh_anchors(&targets).await;
        }));
        true
    }
}

struct Agent<C, N, R> {
    config: Config,
    ipfs: C,
    nip05: N,
    reporter: R,
    own: PublicKey,
    reports: tokio::sync::Mutex<ReportBook>,
    activity: Arc<Activity>,
    layout: MfsLayout,
    state: tokio::sync::Mutex<State>,
    state_path: PathBuf,
    targets: RwLock<HashSet<PublicKey>>,
    queue: Mutex<BTreeMap<SiteKey, Queued>>,
    permits: Semaphore,
    storing: Mutex<HashSet<String>>,
    attempts: Mutex<store::Attempts>,
    report_round: Mutex<Option<AbortHandle>>,
    resend: Mutex<resend::Schedule>,
    anchors: AnchorAgentState,
}

impl<C: KuboStore, N: Nip05Verify, R: ReportRelay> Agent<C, N, R> {
    fn new(
        config: Config,
        ipfs: C,
        nip05: N,
        reporter: R,
        state: State,
        state_path: PathBuf,
        activity: Arc<Activity>,
    ) -> Self {
        let permits = Semaphore::new(config.agent.concurrency);
        let layout = MfsLayout::new(config.ipfs.mfs_root.clone());
        Self {
            layout,
            config,
            ipfs,
            nip05,
            own: reporter.public_key(),
            reporter,
            reports: tokio::sync::Mutex::new(ReportBook::default()),
            activity,
            state: tokio::sync::Mutex::new(state),
            state_path,
            targets: RwLock::new(HashSet::new()),
            queue: Mutex::new(BTreeMap::new()),
            permits,
            storing: Mutex::new(HashSet::new()),
            attempts: Mutex::default(),
            report_round: Mutex::new(None),
            resend: Mutex::default(),
            anchors: AnchorAgentState::default(),
        }
    }

    fn with_anchors(mut self, status: Arc<crate::anchor::AnchorStatus>) -> Self {
        self.anchors = AnchorAgentState::with_status(status);
        self
    }

    fn is_target(&self, pubkey: &PublicKey) -> bool {
        self.targets.read().unwrap().contains(pubkey)
    }

    fn replace_targets(&self, new_targets: HashSet<PublicKey>) {
        *self.targets.write().unwrap() = new_targets;
    }

    async fn save(&self, state: &State, after: &str) {
        if let Err(e) = state.save(&self.state_path).await {
            tracing::error!(error = %e, "saving state after {after} failed");
        }
    }

    fn version_path(&self, key: &str, created_at: u64) -> Option<String> {
        health::version_path(&self.layout, key, created_at)
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
        let snapshot = {
            let mut state = self.state.lock().await;
            let now = now_secs();
            let keys: Vec<SiteKey> = state.sites.keys().cloned().collect();
            let mut removed = Vec::new();
            for key in keys {
                let cids = policy::retention_evictions(
                    &store::version_infos(&state, &key),
                    &self.config.policy,
                    now,
                );
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
            state.clone()
        };
        let garbage = health::find_garbage(&self.ipfs, &self.layout, &snapshot).await;
        self.remove_garbage(&garbage).await;
    }

    async fn remove_garbage(&self, garbage: &health::Garbage) {
        for (path, error) in &garbage.unlisted {
            warn!(path = %path, error = %error, "listing MFS failed");
        }
        if garbage.paths.is_empty() {
            return;
        }
        let state = self.state.lock().await;
        let expected = health::expected_paths(&self.layout, &state);
        let storing = self.storing.lock().unwrap().clone();
        for path in &garbage.paths {
            if covers(&expected, path) || covers(&storing, path) {
                continue;
            }
            self.remove_path(path).await;
        }
    }

    async fn reconcile(&self) {
        let mut state = self.state.lock().await;
        let mut missing = Vec::new();
        for (key, versions) in &state.sites {
            let paths: Vec<String> = versions
                .iter()
                .filter_map(|v| self.version_path(key, v.created_at))
                .collect();
            if paths.len() != versions.len() {
                continue;
            }
            let entries: Vec<(&str, &str)> = paths
                .iter()
                .zip(versions)
                .map(|(path, v)| (path.as_str(), v.cid.as_str()))
                .collect();
            let site = health::check_site(&self.ipfs, &entries).await;
            for ((v, path), problem) in versions.iter().zip(&paths).zip(site.versions) {
                if problem.is_broken() {
                    warn!(site_key = %key, cid = %v.cid, problem = %problem, "forgetting the version so it is fetched again");
                    missing.push((key.clone(), v.cid.clone()));
                } else if let health::VersionHealth::CheckFailed(e) = &problem {
                    warn!(path = %path, error = %e, "checking MFS failed; keeping the version");
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

    // Compared against the state, not the previous follow set, so accounts dropped while stopped are removed too.
    async fn remove_unfollowed(&self) {
        let targets: HashSet<String> = self
            .targets
            .read()
            .unwrap()
            .iter()
            .map(|pk| pk.to_hex())
            .collect();
        self.attempts
            .lock()
            .unwrap()
            .retain_accounts(|pubkey_hex| targets.contains(pubkey_hex));
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
        let storing = self.storing.lock().unwrap().clone();
        for pubkey_hex in &unfollowed {
            let path = self.layout.agent_account(pubkey_hex);
            if covers(&storing, &path) {
                info!(path = %path, "a version is being stored under the unfollowed account; leaving its removal to the sweep");
                continue;
            }
            self.remove_path(&path).await;
        }
    }
}

fn covers(paths: &HashSet<String>, path: &str) -> bool {
    let prefix = format!("{path}/");
    paths.iter().any(|p| p == path || p.starts_with(&prefix))
}

#[cfg(test)]
mod tests;
