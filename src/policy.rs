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

    // A `keep_versions` of 0 would otherwise evict the newest version, the one `decide` just accepted.
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
mod tests;
