use std::sync::Arc;

use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::config::{CheckMode, Config};
use crate::ipfs::{FetchLimits, Fetched, KuboStore};
use crate::nip05::{self, Nip05Verify};
use crate::nostr::{ReportRelay, SiteEvent};
use crate::policy::{self, CandidateEvent, Decision, Usage, VersionInfo};
use crate::state::{self, SiteKey, State, Verification, VersionRecord};

use super::{Agent, now_secs};

const NIP05_ERROR_CACHE_TTL: u64 = 900;

struct Storing<'a> {
    paths: &'a std::sync::Mutex<std::collections::HashSet<String>>,
    path: String,
}

impl<'a> Storing<'a> {
    fn new(paths: &'a std::sync::Mutex<std::collections::HashSet<String>>, path: String) -> Self {
        paths.lock().unwrap().insert(path.clone());
        Self { paths, path }
    }
}

impl Drop for Storing<'_> {
    fn drop(&mut self) {
        self.paths.lock().unwrap().remove(&self.path);
    }
}

pub(super) struct Queued {
    running_created_at: u64,
    next: Option<SiteEvent>,
}

pub(super) fn version_infos(state: &State, key: &SiteKey) -> Vec<VersionInfo> {
    state
        .sites
        .get(key)
        .map(|vs| {
            vs.iter()
                .map(|v| VersionInfo {
                    cid: v.cid.clone(),
                    size: v.size,
                    created_at: v.created_at,
                    stored_at: v.stored_at,
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

impl<C, N, R> Agent<C, N, R>
where
    C: KuboStore + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + Send + Sync + 'static,
{
    pub(super) fn submit(self: &Arc<Self>, ev: SiteEvent, tasks: &mut JoinSet<()>) {
        let pubkey_hex = ev.pubkey.to_hex();
        if !crate::nostr::plausible_at(ev.created_at, now_secs()) {
            warn!(
                site = %ev.d,
                pubkey = %pubkey_hex,
                created_at = ev.created_at,
                reason = "future_created_at",
                "dropping site event before queueing"
            );
            return;
        }
        if !self.is_target(&ev.pubkey) {
            warn!(
                site = %ev.d,
                pubkey = %pubkey_hex,
                "ignoring site event from pubkey not in current follow set"
            );
            return;
        }
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
            let active = state::account_entries(&*queue, &pubkey_hex).count();
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
            let stored = {
                let _permit = self
                    .permits
                    .acquire()
                    .await
                    .expect("the semaphore is never closed");
                self.apply_site_event(&ev).await
            };
            if stored {
                self.sync_reports().await;
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

impl<C: KuboStore, N: Nip05Verify, R: ReportRelay> Agent<C, N, R> {
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

    pub(super) async fn apply_site_event(&self, ev: &SiteEvent) -> bool {
        let pubkey_hex = ev.pubkey.to_hex();
        if !self.is_target(&ev.pubkey) {
            warn!(
                site = %ev.d,
                pubkey = %pubkey_hex,
                "ignoring site event from pubkey not in current follow set"
            );
            return false;
        }
        let key = state::site_key(&pubkey_hex, &ev.d);
        if self.rejected.lock().unwrap().get(&key) == Some(&ev.cid) {
            debug!(cid = %ev.cid, site = %ev.d, "skip: this cid was already rejected after fetch");
            return false;
        }

        let precheck = {
            let state = self.state.lock().await;
            decide(&state, &key, &pubkey_hex, ev, ev.size, &self.config)
        };
        if precheck.store.is_none() {
            info!(site = %ev.d, pubkey = %pubkey_hex, reason = %precheck.reason, "skip");
            return false;
        }

        let nip05_mode = self.config.policy.nip05;
        if nip05_mode != CheckMode::Off {
            let verified = self.nip05_verified(&key, ev, &pubkey_hex).await;
            if nip05_mode == CheckMode::Require && !verified {
                return false;
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
                self.reject(&key, ev);
                return false;
            }
            Err(e) => {
                warn!(cid = %ev.cid, site = %ev.d, error = %e, "fetching content failed; will retry on next poll");
                return false;
            }
        }
        match self.ipfs.is_directory(&ev.cid).await {
            Ok(true) => {}
            Ok(false) => {
                warn!(cid = %ev.cid, site = %ev.d, reason = "not_a_directory", "cid is not a UnixFS directory; not storing");
                self.reject(&key, ev);
                return false;
            }
            Err(e) => {
                warn!(cid = %ev.cid, site = %ev.d, error = %e, "checking whether cid is a directory failed; will retry on next poll");
                return false;
            }
        }

        let path = self.layout.agent_version(&pubkey_hex, &ev.d, ev.created_at);
        let _storing = Storing::new(&self.storing, path.clone());
        {
            let _state = self.state.lock().await;
            if !self.is_target(&ev.pubkey) {
                info!(site = %ev.d, pubkey = %pubkey_hex, "author left the follow set during fetch; not storing");
                return false;
            }
            if let Err(e) = self.ipfs.mfs_put(&ev.cid, &path).await {
                error!(cid = %ev.cid, path = %path, error = %e, "storing into MFS failed");
                return false;
            }
        }
        let size = self.ipfs.dag_size_local(&[ev.cid.as_str()]).await;
        let mut state = self.state.lock().await;
        let size = match size {
            Ok(size) => size,
            Err(e) => {
                warn!(cid = %ev.cid, error = %e, "content is incomplete after fetch; will retry on next poll");
                self.remove_path(&path).await;
                return false;
            }
        };
        if !self.is_target(&ev.pubkey) {
            info!(site = %ev.d, pubkey = %pubkey_hex, "author left the follow set during fetch; not storing");
            self.remove_path(&path).await;
            return false;
        }
        if let Some(declared) = ev.size
            && declared < size
        {
            warn!(cid = %ev.cid, site = %ev.d, declared, actual = size, "size tag understates the content");
        }

        let decision = decide(&state, &key, &pubkey_hex, ev, Some(size), &self.config);
        let Some(cid) = decision.store else {
            warn!(cid = %ev.cid, site = %ev.d, size, reason = %decision.reason, "rejected after fetch");
            self.remove_path(&path).await;
            self.reject(&key, ev);
            return false;
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
        self.rejected.lock().unwrap().remove(&key);
        let evicted = state.remove_versions(&key, &decision.evict);
        self.save(&state, "store").await;
        self.remove_versions(&key, &evicted).await;
        info!(cid = %cid, site = %ev.d, pubkey = %pubkey_hex, size, "stored");
        true
    }

    fn reject(&self, key: &SiteKey, ev: &SiteEvent) {
        self.rejected
            .lock()
            .unwrap()
            .insert(key.clone(), ev.cid.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use nostr_sdk::prelude::Keys;
    use std::collections::HashSet;

    use crate::nip05::VerificationResult;

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
    async fn a_cid_that_is_a_file_is_fetched_but_not_stored() {
        let kubo = FakeKubo::with(|s| {
            s.files.insert("bafy-file".into());
        });
        let fx = Fixture::new(default_policy(), kubo);

        fx.apply(fx.event(D, "bafy-file", Some(20), 200)).await;

        assert_eq!(fx.kubo().fetched, vec!["bafy-file".to_string()]);
        assert!(fx.kubo().paths().is_empty());
        assert!(fx.cids(D).await.is_empty());
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
    async fn submit_drops_events_from_pubkeys_outside_the_follow_set_before_queueing() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let mut flood = fx.event(D, "bafy-intruder", None, 200);
        flood.pubkey = Keys::generate().public_key();

        let mut tasks = JoinSet::new();
        for i in 0..50 {
            let mut ev = flood.clone();
            ev.d = format!("intruder-{i}.example");
            fx.agent.submit(ev, &mut tasks);
        }

        assert_eq!(tasks.len(), 0);
        assert!(fx.agent.queue.lock().unwrap().is_empty());
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
    async fn a_cid_rejected_after_fetch_is_not_fetched_again_until_it_changes() {
        let mut policy = default_policy();
        policy.max_per_site = 50;
        let kubo = sized(&[("bafy-big", 1_000), ("bafy-small", 10)]);
        kubo.s.lock().unwrap().files.insert("bafy-file".into());
        let fx = Fixture::new(policy, kubo);

        fx.apply(fx.event(D, "bafy-big", None, 200)).await;
        fx.apply(fx.event(D, "bafy-big", None, 200)).await;
        fx.apply(fx.event("f.example", "bafy-file", None, 200))
            .await;
        fx.apply(fx.event("f.example", "bafy-file", None, 200))
            .await;
        assert_eq!(fx.kubo().fetched, vec!["bafy-big", "bafy-file"]);

        fx.apply(fx.event(D, "bafy-small", None, 300)).await;
        assert!(fx.kubo().stores("bafy-small"));
    }

    #[tokio::test]
    async fn a_failed_fetch_is_retried() {
        let kubo = FakeKubo::with(|s| {
            s.fail_fetch.insert("bafy-new".into());
        });
        let fx = Fixture::new(default_policy(), kubo);

        fx.apply(fx.event(D, "bafy-new", None, 200)).await;
        fx.kubo().fail_fetch.clear();
        fx.apply(fx.event(D, "bafy-new", None, 200)).await;

        assert_eq!(fx.kubo().fetched, vec!["bafy-new", "bafy-new"]);
        assert!(fx.kubo().stores("bafy-new"));
    }

    #[tokio::test]
    async fn the_sweep_leaves_a_version_that_is_being_stored() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        let path = fx.path(D, 200);
        fx.kubo().mfs.insert(path.clone(), "bafy-new".into());

        {
            let _storing = Storing::new(&fx.agent.storing, path.clone());
            fx.agent.sweep().await;
            assert_eq!(fx.kubo().paths(), vec![path.clone()]);
        }
        fx.agent.sweep().await;
        assert!(fx.kubo().paths().is_empty());
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
    async fn submit_drops_a_future_event_without_displacing_a_pending_one() {
        let fx = Fixture::new(default_policy(), sized(&[("bafy-1", 10), ("bafy-2", 10)]));
        let gate = fx.agent.ipfs.gate.write().await;

        let mut tasks = JoinSet::new();
        fx.agent
            .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
        let forged_future = now_secs() + crate::nostr::MAX_FUTURE_SKEW + 1;
        fx.agent
            .submit(fx.event(D, "bafy-forged", None, forged_future), &mut tasks);
        fx.agent
            .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
        assert_eq!(tasks.len(), 1);
        drop(gate);
        while let Some(joined) = tasks.join_next().await {
            joined.unwrap();
        }

        assert_eq!(fx.kubo().fetched, vec!["bafy-1", "bafy-2"]);
        assert!(fx.kubo().stores("bafy-2"));
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
        let fx = Fixture::new(nip05_policy(CheckMode::Warn), sized(&[("bafy-new", 20)]));
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Mismatch);

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().stores("bafy-new"));
        assert_eq!(fx.verification(D).await.as_deref(), Some("mismatch"));
    }

    #[tokio::test]
    async fn nip05_require_mode_skips_fetch_when_not_verified() {
        let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
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
        let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
        fx.agent
            .nip05
            .set(D, &fx.pubkey.to_hex(), VerificationResult::Verified);

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().stores("bafy-new"));
        assert_eq!(fx.verification(D).await.as_deref(), Some("verified"));
    }

    #[tokio::test]
    async fn nip05_off_mode_never_calls_verifier() {
        let fx = Fixture::new(nip05_policy(CheckMode::Off), FakeKubo::default());

        fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

        assert!(fx.kubo().stores("bafy-new"));
        assert_eq!(fx.agent.nip05.calls(), 0);
        assert_eq!(fx.verification(D).await, None);
    }

    #[tokio::test]
    async fn nip05_result_is_cached_until_ttl_expires() {
        let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
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
        let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
        fx.agent.nip05.set(
            D,
            &fx.pubkey.to_hex(),
            VerificationResult::Error("timeout".into(), nip05::ErrorCategory::Timeout),
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
        let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
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
        let mut policy = nip05_policy(CheckMode::Require);
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
}
