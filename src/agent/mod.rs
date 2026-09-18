mod follow;
mod lifecycle;
mod replicas;
mod store;
#[cfg(test)]
mod test_support;

pub use lifecycle::run;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use nostr_sdk::prelude::*;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{info, warn};

use crate::config::Config;
use crate::health;
use crate::ipfs::KuboStore;
use crate::mfs::MfsLayout;
use crate::nip05::Nip05Verify;
use crate::nostr::{RelayClient, ReportRelay};
use crate::policy;
use crate::state::{SiteKey, State, VersionRecord};

use replicas::ReportBook;
use store::Queued;

fn now_secs() -> u64 {
    Timestamp::now().as_secs()
}

async fn poll_once<C, N, R>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N, R>>,
    tasks: &mut JoinSet<()>,
) where
    C: KuboStore + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + Send + Sync + 'static,
{
    agent.sweep().await;
    follow::refresh_follow_set(relay, agent, tasks).await;
    agent.sync_reports().await;
}

struct Agent<C, N, R> {
    config: Config,
    ipfs: C,
    nip05: N,
    reporter: R,
    own: PublicKey,
    reports: tokio::sync::Mutex<ReportBook>,
    layout: MfsLayout,
    state: tokio::sync::Mutex<State>,
    state_path: PathBuf,
    targets: RwLock<HashSet<PublicKey>>,
    queue: Mutex<BTreeMap<SiteKey, Queued>>,
    permits: Semaphore,
}

impl<C: KuboStore, N: Nip05Verify, R: ReportRelay> Agent<C, N, R> {
    fn new(
        config: Config,
        ipfs: C,
        nip05: N,
        reporter: R,
        state: State,
        state_path: PathBuf,
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
        self.collect_garbage(&state).await;
    }

    async fn collect_garbage(&self, state: &State) {
        let garbage = health::find_garbage(&self.ipfs, &self.layout, state).await;
        for (path, error) in &garbage.unlisted {
            warn!(path = %path, error = %error, "listing MFS failed");
        }
        for path in &garbage.paths {
            self.remove_path(path).await;
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
                let problem = health::check_version(&self.ipfs, &path, &v.cid).await;
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
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::config::Nip05Mode;
    use crate::ipfs::IpfsClient;
    use crate::nip05::VerificationResult;
    use crate::nostr::SiteEvent;
    use crate::state::{self, VersionRecord};

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
            FakeRelay::default(),
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
            message: None,
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
