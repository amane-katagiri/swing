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
const REJECTED_PER_ACCOUNT: usize = 50;
const ATTEMPTS_PER_ACCOUNT: usize = 50;

#[derive(Default)]
struct SiteAttempts {
    rejected: Vec<(String, u64)>,
    tried_at: Option<u64>,
}

// Stored versions only throttle updates that succeeded, so failed or rejected attempts are throttled here.
#[derive(Default)]
pub(super) struct Attempts {
    sites: std::collections::BTreeMap<SiteKey, SiteAttempts>,
    next: u64,
}

impl Attempts {
    fn is_rejected(&self, key: &str, cid: &str) -> bool {
        self.sites
            .get(key)
            .is_some_and(|s| s.rejected.iter().any(|(c, _)| c == cid))
    }

    fn try_attempt(
        &mut self,
        key: &SiteKey,
        stored: bool,
        now: u64,
        window: u64,
        per_account: usize,
    ) -> Result<(), &'static str> {
        let recent = |at: &u64| now.saturating_sub(*at) < window;
        let entry = self.sites.get(key);
        if entry.and_then(|e| e.tried_at.as_ref()).is_some_and(recent) {
            return Err("fetch_attempt_interval");
        }
        let Some((account, _)) = state::split_site_key(key) else {
            return Ok(());
        };
        let account = account.to_string();
        let updates_stored = stored && entry.is_none_or(|e| e.rejected.is_empty());
        let recent_attempts = state::account_entries(&self.sites, &account)
            .filter(|(k, e)| *k != key && e.tried_at.as_ref().is_some_and(recent))
            .count();
        if !updates_stored && recent_attempts >= per_account.clamp(1, ATTEMPTS_PER_ACCOUNT) {
            return Err("fetch_attempts_per_account");
        }
        self.sites.entry(key.clone()).or_default().tried_at = Some(now);
        let stale: Vec<SiteKey> = state::account_entries(&self.sites, &account)
            .filter(|(_, e)| e.rejected.is_empty() && !e.tried_at.as_ref().is_some_and(recent))
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            self.sites.remove(&k);
        }
        Ok(())
    }

    fn clear(&mut self, key: &str) {
        self.sites.remove(key);
    }

    pub(super) fn retain_accounts(&mut self, keep: impl Fn(&str) -> bool) {
        self.sites
            .retain(|key, _| state::split_site_key(key).is_none_or(|(account, _)| keep(account)));
    }

    fn reject(&mut self, key: &SiteKey, cid: String) {
        self.next += 1;
        let seq = self.next;
        let entry = self.sites.entry(key.clone()).or_default();
        if !entry.rejected.iter().any(|(c, _)| *c == cid) {
            entry.rejected.push((cid, seq));
        }
        let Some((account, _)) = state::split_site_key(key) else {
            return;
        };
        let account = account.to_string();
        loop {
            let rejected: Vec<(&SiteKey, usize, u64)> =
                state::account_entries(&self.sites, &account)
                    .flat_map(|(k, e)| {
                        e.rejected
                            .iter()
                            .enumerate()
                            .map(move |(i, (_, seq))| (k, i, *seq))
                    })
                    .collect();
            if rejected.len() <= REJECTED_PER_ACCOUNT {
                return;
            }
            let Some((k, i, _)) = rejected.into_iter().min_by_key(|(_, _, seq)| *seq) else {
                return;
            };
            let k = k.clone();
            if let Some(e) = self.sites.get_mut(&k) {
                e.rejected.remove(i);
            }
        }
    }
}

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

fn usage(state: &State, key: &SiteKey, pubkey_hex: &str) -> Usage {
    let site = state.site_bytes(key);
    Usage {
        other_sites: state.total_bytes() - site,
        other_sites_of_account: state.account_bytes(pubkey_hex) - site,
        other_site_count_of_account: state.account_site_count(pubkey_hex)
            - usize::from(state.sites.contains_key(key)),
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
    let existing = version_infos(state, key);
    let usage = usage(state, key, pubkey_hex);
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
        let key = state::site_key(&pubkey_hex, &ev.d);
        let Some(max_bytes) = self.worth_fetching(&key, ev, &pubkey_hex).await else {
            return false;
        };
        self.fetch_directory(&key, ev, max_bytes).await
            && self.store_fetched(&key, ev, &pubkey_hex).await
    }

    async fn worth_fetching(&self, key: &SiteKey, ev: &SiteEvent, pubkey_hex: &str) -> Option<u64> {
        if !self.is_target(&ev.pubkey) {
            warn!(
                site = %ev.d,
                pubkey = %pubkey_hex,
                "ignoring site event from pubkey not in current follow set"
            );
            return None;
        }
        if self.attempts.lock().unwrap().is_rejected(key, &ev.cid) {
            debug!(cid = %ev.cid, site = %ev.d, "skip: this cid was already rejected after fetch");
            return None;
        }

        let (precheck, stored, budget) = {
            let state = self.state.lock().await;
            (
                decide(&state, key, pubkey_hex, ev, ev.size, &self.config),
                state.sites.contains_key(key),
                policy::fetch_budget(&self.config.policy, usage(&state, key, pubkey_hex)),
            )
        };
        if precheck.store.is_none() {
            info!(site = %ev.d, pubkey = %pubkey_hex, reason = %precheck.reason, "skip");
            return None;
        }

        let policy = &self.config.policy;
        if let Err(reason) = self.attempts.lock().unwrap().try_attempt(
            key,
            stored,
            now_secs(),
            policy.min_update_interval,
            policy.max_sites_per_account,
        ) {
            debug!(site = %ev.d, pubkey = %pubkey_hex, reason, "skip");
            return None;
        }
        if policy.nip05 != CheckMode::Off {
            let verified = self.nip05_verified(key, ev, pubkey_hex).await;
            if policy.nip05 == CheckMode::Require && !verified {
                return None;
            }
        }
        Some(budget)
    }

    async fn fetch_directory(&self, key: &SiteKey, ev: &SiteEvent, max_bytes: u64) -> bool {
        match self.ipfs.is_directory(&ev.cid).await {
            Ok(true) => {}
            Ok(false) => {
                warn!(cid = %ev.cid, site = %ev.d, reason = "not_a_directory", "cid is not a UnixFS directory; not fetching");
                self.reject(key, ev);
                return false;
            }
            Err(e) => {
                warn!(cid = %ev.cid, site = %ev.d, error = %e, "checking whether cid is a directory failed; will retry after min_update_interval");
                return false;
            }
        }
        let limits = FetchLimits {
            max_bytes,
            total: self.config.agent.fetch_timeout,
            idle: self.config.agent.fetch_idle_timeout,
        };
        match self.ipfs.fetch_dag(&ev.cid, limits).await {
            Ok(Fetched::Complete) => true,
            Ok(Fetched::TooLarge) => {
                warn!(cid = %ev.cid, site = %ev.d, limit = limits.max_bytes, "content exceeds the fetch limit or the space left for it; aborted");
                self.reject(key, ev);
                false
            }
            Err(e) => {
                warn!(cid = %ev.cid, site = %ev.d, error = %e, "fetching content failed; will retry after min_update_interval");
                false
            }
        }
    }

    async fn store_fetched(&self, key: &SiteKey, ev: &SiteEvent, pubkey_hex: &str) -> bool {
        let path = self.layout.agent_version(pubkey_hex, &ev.d, ev.created_at);
        let _storing = {
            let _state = self.state.lock().await;
            if !self.is_target(&ev.pubkey) {
                info!(site = %ev.d, pubkey = %pubkey_hex, "author left the follow set during fetch; not storing");
                return false;
            }
            // Registered under the state lock so that a sweep either has already removed what it listed or sees this path.
            Storing::new(&self.storing, path.clone())
        };
        if let Err(e) = self.ipfs.mfs_put(&ev.cid, &path).await {
            error!(cid = %ev.cid, path = %path, error = %e, "storing into MFS failed");
            return false;
        }
        let size = self.ipfs.dag_size_local(&[ev.cid.as_str()]).await;
        let mut state = self.state.lock().await;
        let size = match size {
            Ok(size) => size,
            Err(e) => {
                drop(state);
                warn!(cid = %ev.cid, error = %e, "content is incomplete after fetch; will retry after min_update_interval");
                self.remove_path(&path).await;
                return false;
            }
        };
        if !self.is_target(&ev.pubkey) {
            drop(state);
            info!(site = %ev.d, pubkey = %pubkey_hex, "author left the follow set during fetch; not storing");
            self.remove_path(&path).await;
            return false;
        }
        if let Some(declared) = ev.size
            && declared < size
        {
            warn!(cid = %ev.cid, site = %ev.d, declared, actual = size, "size tag understates the content");
        }

        let decision = decide(&state, key, pubkey_hex, ev, Some(size), &self.config);
        let Some(cid) = decision.store else {
            drop(state);
            warn!(cid = %ev.cid, site = %ev.d, size, reason = %decision.reason, "rejected after fetch");
            self.remove_path(&path).await;
            self.reject(key, ev);
            return false;
        };
        state.apply_store(
            key,
            VersionRecord {
                cid: cid.clone(),
                size,
                created_at: ev.created_at,
                stored_at: now_secs(),
            },
        );
        self.attempts.lock().unwrap().clear(key);
        let evicted = state.remove_versions(key, &decision.evict);
        self.save(&state, "store").await;
        drop(state);
        self.remove_versions(key, &evicted).await;
        info!(cid = %cid, site = %ev.d, pubkey = %pubkey_hex, size, "stored");
        true
    }

    fn reject(&self, key: &SiteKey, ev: &SiteEvent) {
        self.attempts.lock().unwrap().reject(key, ev.cid.clone());
    }
}

#[cfg(test)]
mod tests;
