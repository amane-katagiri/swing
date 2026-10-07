use crate::config::PolicyConfig;
use crate::nostr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub cid: String,
    pub size: u64,
    pub created_at: u64,
    pub stored_at: u64,
}

#[derive(Debug, Clone)]
pub struct CandidateEvent {
    pub cid: String,
    pub size: Option<u64>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub other_sites: u64,
    pub other_sites_of_account: u64,
    pub other_site_count_of_account: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Decision {
    pub store: Option<String>,
    pub evict: Vec<String>,
    pub reason: String,
}

impl Decision {
    fn skip(reason: impl Into<String>) -> Self {
        Self {
            store: None,
            evict: Vec::new(),
            reason: reason.into(),
        }
    }
}

fn total_size(versions: &[VersionInfo]) -> u64 {
    versions.iter().map(|v| v.size).sum()
}

fn evict_oldest_until(
    versions: &mut Vec<VersionInfo>,
    fits: impl Fn(&[VersionInfo]) -> bool,
) -> Vec<String> {
    let mut removed = Vec::new();
    while versions.len() > 1 && !fits(versions) {
        removed.push(versions.remove(0).cid);
    }
    removed
}

fn evict(versions: &mut Vec<VersionInfo>, cfg: &PolicyConfig, now: u64) -> Vec<String> {
    let mut evicted = evict_oldest_until(versions, |vs| total_size(vs) <= cfg.max_per_site);

    // A `keep_versions` of 0 would otherwise evict the newest version, which
    // is the one just accepted in `decide`.
    let keep_versions = cfg.keep_versions.max(1);
    evicted.extend(evict_oldest_until(versions, |vs| vs.len() <= keep_versions));

    if cfg.keep_days > 0 {
        let cutoff = now.saturating_sub(cfg.keep_days.saturating_mul(86_400));
        let newest_created_at = versions.iter().map(|v| v.created_at).max().unwrap_or(0);
        let (expired, kept): (Vec<VersionInfo>, Vec<VersionInfo>) = std::mem::take(versions)
            .into_iter()
            .partition(|v| v.created_at < cutoff && v.created_at != newest_created_at);
        evicted.extend(expired.into_iter().map(|v| v.cid));
        *versions = kept;
    }
    evicted
}

pub fn retention_evictions(
    existing_versions: &[VersionInfo],
    cfg: &PolicyConfig,
    now: u64,
) -> Vec<String> {
    let mut versions = existing_versions.to_vec();
    versions.sort_by_key(|v| v.created_at);
    evict(&mut versions, cfg, now)
}

pub fn fetch_limit(cfg: &PolicyConfig) -> u64 {
    cfg.max_update_size
        .min(cfg.max_per_site)
        .min(cfg.max_per_account)
}

pub fn fetch_budget(cfg: &PolicyConfig, usage: Usage) -> u64 {
    fetch_limit(cfg)
        .min(
            cfg.max_per_account
                .saturating_sub(usage.other_sites_of_account),
        )
        .min(cfg.max_total_storage.saturating_sub(usage.other_sites))
}

pub fn decide(
    existing_versions: &[VersionInfo],
    usage: Usage,
    candidate: &CandidateEvent,
    cfg: &PolicyConfig,
    now: u64,
) -> Decision {
    if !nostr::plausible_at(candidate.created_at, now) {
        return Decision::skip("future_created_at");
    }

    if existing_versions.iter().any(|v| v.cid == candidate.cid) {
        return Decision::skip("duplicate_cid");
    }

    if existing_versions.is_empty()
        && usage.other_site_count_of_account >= cfg.max_sites_per_account
    {
        return Decision::skip("max_sites_per_account");
    }

    let last_created_at = existing_versions.iter().map(|v| v.created_at).max();

    if let Some(last) = last_created_at
        && candidate.created_at <= last
    {
        return Decision::skip("stale");
    }

    if let Some(last) = existing_versions.iter().map(|v| v.stored_at).max()
        && now.saturating_sub(last) < cfg.min_update_interval
    {
        return Decision::skip("min_update_interval");
    }

    if let Some(size) = candidate.size
        && size > cfg.max_update_size
    {
        return Decision::skip("max_update_size");
    }

    let new_size = candidate.size.unwrap_or(0);
    let mut versions_after: Vec<VersionInfo> = existing_versions.to_vec();
    versions_after.push(VersionInfo {
        cid: candidate.cid.clone(),
        size: new_size,
        created_at: candidate.created_at,
        stored_at: now,
    });
    versions_after.sort_by_key(|v| v.created_at);

    if new_size > cfg.max_per_site {
        return Decision::skip("max_per_site_exceeded_alone");
    }
    let evicted = evict(&mut versions_after, cfg, now);

    let site_after = total_size(&versions_after);
    if usage.other_sites_of_account + site_after > cfg.max_per_account {
        return Decision::skip("max_per_account");
    }
    if usage.other_sites + site_after > cfg.max_total_storage {
        return Decision::skip("max_total_storage");
    }

    Decision {
        store: Some(candidate.cid.clone()),
        evict: evicted,
        reason: "accepted".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PolicyConfig {
        PolicyConfig {
            max_total_storage: 1_000_000,
            max_per_site: 300,
            max_per_account: 1_000_000,
            max_sites_per_account: 10,
            max_update_size: 250,
            keep_versions: 5,
            keep_days: 365,
            min_update_interval: 600,
            remove_on_unfollow: true,
            nip05: crate::config::CheckMode::Off,
            nip05_cache_ttl: 0,
        }
    }

    fn total_usage(other_sites: u64) -> Usage {
        Usage {
            other_sites,
            ..Usage::default()
        }
    }

    fn v(cid: &str, size: u64, created_at: u64) -> VersionInfo {
        v_at(cid, size, created_at, created_at)
    }

    fn v_at(cid: &str, size: u64, created_at: u64, stored_at: u64) -> VersionInfo {
        VersionInfo {
            cid: cid.to_string(),
            size,
            created_at,
            stored_at,
        }
    }

    fn c(cid: &str, size: Option<u64>, created_at: u64) -> CandidateEvent {
        CandidateEvent {
            cid: cid.to_string(),
            size,
            created_at,
        }
    }

    #[test]
    fn duplicate_cid_is_skipped() {
        let existing = vec![v("bafy1", 100, 1000)];
        let cand = c("bafy1", Some(100), 2000);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 3000);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "duplicate_cid");
        assert!(d.evict.is_empty());
    }

    #[test]
    fn min_update_interval_skips_too_soon_update() {
        let existing = vec![v("bafy1", 100, 1000)];
        let cand = c("bafy2", Some(100), 1500);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 1000 + 599);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "min_update_interval");
    }

    #[test]
    fn min_update_interval_allows_update_at_exact_boundary() {
        let existing = vec![v("bafy1", 100, 1000)];
        let cand = c("bafy2", Some(100), 1500);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 1000 + 600);
        assert_eq!(d.store, Some("bafy2".to_string()));
    }

    #[test]
    fn min_update_interval_counts_from_stored_at_not_created_at() {
        let existing = vec![v_at("bafy1", 100, 1000, 5000)];
        let cand = c("bafy2", Some(100), 4000);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 5100);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "min_update_interval");
    }

    #[test]
    fn min_update_interval_accepts_the_same_event_once_the_wait_has_passed() {
        let existing = vec![v("bafy1", 100, 1000)];
        let cand = c("bafy2", Some(100), 1010);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 1500);
        assert_eq!(d.reason, "min_update_interval");

        let d = decide(&existing, Usage::default(), &cand, &cfg(), 1600);
        assert_eq!(d.store, Some("bafy2".to_string()));
    }

    #[test]
    fn a_forged_created_at_does_not_shorten_the_wait() {
        let existing = vec![v("bafy1", 100, 1000)];
        let cand = c("bafy2", Some(100), 1000 + 10 * 600);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 1100);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "future_created_at");
    }

    #[test]
    fn far_future_created_at_is_rejected_on_a_fresh_site() {
        let d = decide(
            &[],
            Usage::default(),
            &c("bafy1", Some(10), 2000),
            &cfg(),
            1000,
        );
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "future_created_at");
    }

    #[test]
    fn created_at_within_the_skew_tolerance_is_accepted() {
        let cand = c("bafy1", Some(10), 1000 + nostr::MAX_FUTURE_SKEW);
        let d = decide(&[], Usage::default(), &cand, &cfg(), 1000);
        assert_eq!(d.store, Some("bafy1".to_string()));
    }

    #[test]
    fn max_update_size_skips_oversized_event() {
        let existing: Vec<VersionInfo> = vec![];
        let cand = c("bafy1", Some(251), 1000);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 2000);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "max_update_size");
    }

    #[test]
    fn max_update_size_none_size_is_not_checked_here() {
        let existing: Vec<VersionInfo> = vec![];
        let cand = c("bafy1", None, 1000);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 2000);
        assert_eq!(d.store, Some("bafy1".to_string()));
    }

    #[test]
    fn max_per_site_evicts_oldest_versions_to_fit() {
        let mut c1 = cfg();
        c1.max_update_size = 10_000;
        let existing = vec![v("old1", 150, 1000), v("old2", 100, 1700)];
        let cand = c("new1", Some(100), 3000);
        let d = decide(&existing, Usage::default(), &cand, &c1, 4000);
        assert_eq!(d.store, Some("new1".to_string()));
        assert_eq!(d.evict, vec!["old1".to_string()]);
    }

    #[test]
    fn max_per_site_skips_when_new_alone_exceeds_limit() {
        let mut c1 = cfg();
        c1.max_update_size = 10_000;
        let existing: Vec<VersionInfo> = vec![];
        let cand = c("new1", Some(301), 1000);
        let d = decide(&existing, Usage::default(), &cand, &c1, 2000);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "max_per_site_exceeded_alone");
        assert!(d.evict.is_empty());
    }

    #[test]
    fn max_total_storage_skips_without_touching_other_sites() {
        let existing: Vec<VersionInfo> = vec![];
        let mut c1 = cfg();
        c1.max_total_storage = 500;
        let cand = c("new1", Some(200), 1000);
        let d = decide(&existing, total_usage(400), &cand, &c1, 2000);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "max_total_storage");
        assert!(d.evict.is_empty());
    }

    #[test]
    fn keep_versions_evicts_oldest_beyond_limit() {
        let mut c1 = cfg();
        c1.keep_versions = 2;
        c1.max_per_site = 10_000;
        let existing = vec![v("v1", 10, 1000), v("v2", 10, 1700)];
        let cand = c("v3", Some(10), 2400);
        let d = decide(&existing, Usage::default(), &cand, &c1, 3000);
        assert_eq!(d.store, Some("v3".to_string()));
        assert_eq!(d.evict, vec!["v1".to_string()]);
    }

    #[test]
    fn keep_versions_zero_still_keeps_the_new_version() {
        let mut c1 = cfg();
        c1.keep_versions = 0;
        c1.max_per_site = 10_000;
        let existing = vec![v("v1", 10, 1000)];
        let cand = c("v2", Some(10), 1700);
        let d = decide(&existing, Usage::default(), &cand, &c1, 3000);
        assert_eq!(d.store, Some("v2".to_string()));
        assert_eq!(d.evict, vec!["v1".to_string()]);
    }

    #[test]
    fn keep_days_evicts_old_versions_but_keeps_latest() {
        let mut c1 = cfg();
        c1.max_per_site = 10_000;
        c1.keep_versions = 100;
        c1.keep_days = 10;
        let day = 86_400u64;
        let now = 100 * day;
        let existing = vec![v("ancient", 10, day), v("recent_old", 10, now - 20 * day)];
        let cand = c("new", Some(10), now - 1);
        let d = decide(&existing, Usage::default(), &cand, &c1, now);
        assert_eq!(d.store, Some("new".to_string()));
        assert!(d.evict.contains(&"ancient".to_string()));
        assert!(d.evict.contains(&"recent_old".to_string()));
    }

    #[test]
    fn keep_days_never_evicts_the_newest_version_even_if_old() {
        let mut c1 = cfg();
        c1.max_per_site = 10_000;
        c1.keep_versions = 100;
        c1.keep_days = 10;
        let day = 86_400u64;
        let now = 100 * day;
        let existing: Vec<VersionInfo> = vec![];
        let cand = c("new_but_old_timestamp", Some(10), day);
        let d = decide(&existing, Usage::default(), &cand, &c1, now);
        assert_eq!(d.store, Some("new_but_old_timestamp".to_string()));
        assert!(d.evict.is_empty());
    }

    #[test]
    fn first_ever_version_is_accepted() {
        let existing: Vec<VersionInfo> = vec![];
        let cand = c("bafy1", Some(50), 1000);
        let d = decide(&existing, Usage::default(), &cand, &cfg(), 1000);
        assert_eq!(d.store, Some("bafy1".to_string()));
        assert!(d.evict.is_empty());
        assert_eq!(d.reason, "accepted");
    }

    #[test]
    fn stale_event_is_rejected_even_with_zero_min_update_interval() {
        let mut c1 = cfg();
        c1.min_update_interval = 0;
        let existing = vec![v("bafy1", 100, 2000)];
        let same_ts = c("bafy2", Some(100), 2000);
        let d = decide(&existing, Usage::default(), &same_ts, &c1, 3000);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "stale");

        let older_ts = c("bafy3", Some(100), 1000);
        let d = decide(&existing, Usage::default(), &older_ts, &c1, 3000);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "stale");
    }

    #[test]
    fn max_total_storage_uses_post_eviction_total() {
        let mut c1 = cfg();
        c1.keep_versions = 3;
        c1.max_per_site = 10_000;
        c1.max_update_size = 10_000;
        c1.max_total_storage = 350;
        c1.min_update_interval = 0;
        let existing = vec![v("v1", 100, 1000), v("v2", 100, 1100), v("v3", 100, 1200)];
        let cand = c("v4", Some(100), 1300);
        let d = decide(&existing, Usage::default(), &cand, &c1, 2000);
        assert_eq!(d.store, Some("v4".to_string()));
        assert_eq!(d.evict, vec!["v1".to_string()]);
    }

    #[test]
    fn max_per_account_counts_other_sites_of_the_same_account() {
        let mut c1 = cfg();
        c1.max_per_account = 500;
        let cand = c("v1", Some(200), 1000);
        let usage = Usage {
            other_sites: 400,
            other_sites_of_account: 400,
            ..Usage::default()
        };
        let d = decide(&[], usage, &cand, &c1, 2000);
        assert_eq!(d.store, None);
        assert_eq!(d.reason, "max_per_account");

        let usage = Usage {
            other_sites: 400,
            other_sites_of_account: 300,
            ..Usage::default()
        };
        let d = decide(&[], usage, &cand, &c1, 2000);
        assert_eq!(d.store, Some("v1".to_string()));
    }

    #[test]
    fn max_per_account_uses_post_eviction_site_size() {
        let mut c1 = cfg();
        c1.max_per_account = 450;
        c1.keep_versions = 1;
        c1.min_update_interval = 0;
        let existing = vec![v("v1", 200, 1000)];
        let cand = c("v2", Some(200), 1100);
        let usage = Usage {
            other_sites: 250,
            other_sites_of_account: 250,
            ..Usage::default()
        };
        let d = decide(&existing, usage, &cand, &c1, 2000);
        assert_eq!(d.store, Some("v2".to_string()));
        assert_eq!(d.evict, vec!["v1".to_string()]);
    }

    #[test]
    fn max_sites_per_account_blocks_only_new_sites() {
        let mut c1 = cfg();
        c1.max_sites_per_account = 2;
        c1.min_update_interval = 0;
        let usage = Usage {
            other_site_count_of_account: 2,
            ..Usage::default()
        };
        let d = decide(&[], usage, &c("new", Some(10), 1000), &c1, 2000);
        assert_eq!(d.reason, "max_sites_per_account");

        let existing = vec![v("v1", 10, 1000)];
        let d = decide(&existing, usage, &c("v2", Some(10), 1100), &c1, 2000);
        assert_eq!(d.store, Some("v2".to_string()));

        let usage = Usage {
            other_site_count_of_account: 1,
            ..Usage::default()
        };
        let d = decide(&[], usage, &c("new", Some(10), 1000), &c1, 2000);
        assert_eq!(d.store, Some("new".to_string()));
    }

    #[test]
    fn fetch_limit_is_the_smallest_per_candidate_cap() {
        let mut c1 = cfg();
        assert_eq!(fetch_limit(&c1), 250);
        c1.max_per_account = 100;
        assert_eq!(fetch_limit(&c1), 100);
        c1.max_per_site = 50;
        assert_eq!(fetch_limit(&c1), 50);
    }

    #[test]
    fn fetch_budget_leaves_out_what_the_account_and_node_already_hold() {
        let mut c1 = cfg();
        c1.max_per_account = 1_000;
        c1.max_total_storage = 2_000;
        assert_eq!(fetch_budget(&c1, Usage::default()), 250);
        let usage = |account, total| Usage {
            other_sites_of_account: account,
            other_sites: total,
            ..Usage::default()
        };
        assert_eq!(fetch_budget(&c1, usage(900, 900)), 100);
        assert_eq!(fetch_budget(&c1, usage(0, 1_950)), 50);
        assert_eq!(fetch_budget(&c1, usage(1_200, 1_200)), 0);
        assert_eq!(fetch_budget(&c1, usage(0, 3_000)), 0);
    }

    #[test]
    fn retention_evicts_idle_sites_without_a_new_version() {
        let mut c1 = cfg();
        c1.keep_versions = 2;
        c1.keep_days = 1;
        let now = 10 * 86_400;
        let existing = vec![
            v("v3", 10, now - 3 * 86_400),
            v("v1", 10, now - 5 * 86_400),
            v("v2", 10, now - 4 * 86_400),
        ];
        assert_eq!(
            retention_evictions(&existing, &c1, now),
            vec!["v1".to_string(), "v2".to_string()]
        );
    }

    #[test]
    fn retention_keeps_the_newest_version_and_honours_a_lowered_limit() {
        let mut c1 = cfg();
        c1.keep_versions = 1;
        c1.max_per_site = 15;
        let existing = vec![v("v1", 10, 1000), v("v2", 10, 2000)];
        assert_eq!(
            retention_evictions(&existing, &c1, 3000),
            vec!["v1".to_string()]
        );
        assert!(retention_evictions(&[v("v1", 100, 1000)], &c1, 3000).is_empty());
        assert!(retention_evictions(&[], &c1, 3000).is_empty());
    }

    #[test]
    fn keep_days_does_not_overflow() {
        let mut c1 = cfg();
        c1.keep_days = u64::MAX;
        let existing = vec![v("v1", 10, 1000), v("v2", 10, 2000)];
        assert!(retention_evictions(&existing, &c1, 3000).is_empty());
    }
}
