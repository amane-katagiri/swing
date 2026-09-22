use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;

use anyhow::{Result, bail};
use nostr_sdk::prelude::PublicKey;

use crate::config::Config;
use crate::ipfs::{IpfsClient, KuboStore, MfsEntry};
use crate::mfs::MfsLayout;
use crate::state::{self, State};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionHealth {
    Ok,
    Missing,
    Mismatch(String),
    Incomplete(String),
    CheckFailed(String),
}

impl VersionHealth {
    pub fn is_broken(&self) -> bool {
        matches!(
            self,
            Self::Missing | Self::Mismatch(_) | Self::Incomplete(_)
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::Mismatch(_) => "cid mismatch",
            Self::Incomplete(_) => "incomplete",
            Self::CheckFailed(_) => "check failed",
        }
    }
}

impl fmt::Display for VersionHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => f.write_str("ok"),
            Self::Missing => f.write_str("MFS entry is missing"),
            Self::Mismatch(cid) => write!(f, "MFS entry points to {cid}"),
            Self::Incomplete(e) => write!(f, "content is incomplete: {e}"),
            Self::CheckFailed(e) => write!(f, "checking MFS failed: {e}"),
        }
    }
}

async fn check_placement<C: KuboStore>(ipfs: &C, path: &str, cid: &str) -> Option<VersionHealth> {
    match ipfs.mfs_stat_cid(path).await {
        Ok(Some(found)) if found == cid => None,
        Ok(Some(found)) => Some(VersionHealth::Mismatch(found)),
        Ok(None) => Some(VersionHealth::Missing),
        Err(e) => Some(VersionHealth::CheckFailed(format!("{e:#}"))),
    }
}

async fn check_blocks<C: KuboStore>(ipfs: &C, cid: &str) -> VersionHealth {
    match ipfs.dag_size_local(&[cid]).await {
        Ok(_) => VersionHealth::Ok,
        Err(e) => VersionHealth::Incomplete(format!("{e:#}")),
    }
}

pub async fn check_version<C: KuboStore>(ipfs: &C, path: &str, cid: &str) -> VersionHealth {
    match check_placement(ipfs, path, cid).await {
        Some(problem) => problem,
        None => check_blocks(ipfs, cid).await,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteHealth {
    pub versions: Vec<VersionHealth>,
    pub actual_size: Option<u64>,
}

// One dag/stat over every version of a site walks blocks the versions share
// only once, and fails if any block is missing, so the per-version walk is
// only needed to tell which version is broken.
pub async fn check_site<C: KuboStore>(ipfs: &C, versions: &[(&str, &str)]) -> SiteHealth {
    let mut health = Vec::with_capacity(versions.len());
    for (path, cid) in versions {
        health.push(
            check_placement(ipfs, path, cid)
                .await
                .unwrap_or(VersionHealth::Ok),
        );
    }

    let placed = complete_cids(versions, &health);
    if let Ok(size) = ipfs.dag_size_local(&placed).await {
        return SiteHealth {
            versions: health,
            actual_size: Some(size),
        };
    }

    for (i, h) in health.iter_mut().enumerate() {
        if *h == VersionHealth::Ok {
            *h = check_blocks(ipfs, versions[i].1).await;
        }
    }
    let complete = complete_cids(versions, &health);
    let actual_size = ipfs.dag_size_local(&complete).await.ok();
    SiteHealth {
        versions: health,
        actual_size,
    }
}

fn complete_cids<'a>(versions: &[(&str, &'a str)], health: &[VersionHealth]) -> Vec<&'a str> {
    versions
        .iter()
        .zip(health)
        .filter(|(_, h)| **h == VersionHealth::Ok)
        .map(|((_, cid), _)| *cid)
        .collect()
}

pub fn version_path(layout: &MfsLayout, key: &str, created_at: u64) -> Option<String> {
    let (pubkey_hex, d) = state::split_site_key(key)?;
    Some(layout.agent_version(pubkey_hex, d, created_at))
}

#[derive(Debug, Default, Clone)]
pub struct Garbage {
    pub paths: Vec<String>,
    pub unlisted: Vec<(String, String)>,
}

async fn list_dir<C: KuboStore>(
    ipfs: &C,
    path: &str,
    garbage: &mut Garbage,
) -> Option<Vec<MfsEntry>> {
    match ipfs.mfs_list(path).await {
        Ok(entries) => Some(entries),
        Err(e) => {
            garbage.unlisted.push((path.to_string(), format!("{e:#}")));
            None
        }
    }
}

// Everything under the agent root belongs to SWING, so any entry the state
// does not reference is a leftover of a failed or interrupted step.
pub async fn find_garbage<C: KuboStore>(ipfs: &C, layout: &MfsLayout, state: &State) -> Garbage {
    let expected: HashSet<String> = state
        .sites
        .iter()
        .flat_map(|(key, versions)| {
            versions
                .iter()
                .filter_map(|v| version_path(layout, key, v.created_at))
        })
        .collect();
    let mut garbage = Garbage::default();
    let root = layout.agent_root();
    let Some(accounts) = list_dir(ipfs, &root, &mut garbage).await else {
        return garbage;
    };
    for account in accounts {
        let account_path = format!("{root}/{}", account.name);
        if !account.is_dir {
            garbage.paths.push(account_path);
            continue;
        }
        let Some(sites) = list_dir(ipfs, &account_path, &mut garbage).await else {
            continue;
        };
        let mut account_garbage = Vec::new();
        let mut account_kept = false;
        for site in sites {
            let site_path = format!("{account_path}/{}", site.name);
            if !site.is_dir {
                account_garbage.push(site_path);
                continue;
            }
            let Some(versions) = list_dir(ipfs, &site_path, &mut garbage).await else {
                account_kept = true;
                continue;
            };
            let (kept, stale): (Vec<String>, Vec<String>) = versions
                .into_iter()
                .map(|v| format!("{site_path}/{}", v.name))
                .partition(|path| expected.contains(path));
            if kept.is_empty() {
                account_garbage.push(site_path);
            } else {
                account_kept = true;
                account_garbage.extend(stale);
            }
        }
        if account_kept {
            garbage.paths.extend(account_garbage);
        } else {
            garbage.paths.push(account_path);
        }
    }
    garbage
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionStatus {
    pub pubkey: PublicKey,
    pub d: String,
    pub path: String,
    pub cid: String,
    pub size: u64,
    pub created_at: u64,
    pub health: VersionHealth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusLine {
    Version(VersionStatus),
    InvalidKey { key: String, cid: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteSize {
    pub pubkey: PublicKey,
    pub d: String,
    pub path: String,
    pub actual: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct StatusReport {
    pub state_path: PathBuf,
    pub lines: Vec<StatusLine>,
    pub sites: Vec<SiteSize>,
    pub garbage: Garbage,
    pub problems: usize,
}

impl StatusReport {
    pub fn versions(&self) -> impl Iterator<Item = &VersionStatus> {
        self.lines.iter().filter_map(|line| match line {
            StatusLine::Version(v) => Some(v),
            StatusLine::InvalidKey { .. } => None,
        })
    }

    pub fn actual_bytes(&self) -> Option<u64> {
        self.sites.iter().map(|s| s.actual).sum()
    }
}

pub async fn collect_status<C: KuboStore>(ipfs: &C, config: &Config) -> Result<StatusReport> {
    let state_path = config.agent.state_dir.join("state.json");
    let state = State::load(&state_path).await?;
    let layout = MfsLayout::new(config.ipfs.mfs_root.clone());

    let mut problems = 0usize;
    let mut lines = Vec::new();
    let mut sites = Vec::new();
    for (key, versions) in &state.sites {
        let parsed = state::split_site_key(key).and_then(|(pubkey_hex, d)| {
            PublicKey::from_hex(pubkey_hex)
                .ok()
                .map(|pubkey| (pubkey_hex, pubkey, d))
        });
        let Some((pubkey_hex, pubkey, d)) = parsed else {
            for v in versions {
                lines.push(StatusLine::InvalidKey {
                    key: key.clone(),
                    cid: v.cid.clone(),
                });
                problems += 1;
            }
            continue;
        };
        let paths: Vec<String> = versions
            .iter()
            .map(|v| layout.agent_version(pubkey_hex, d, v.created_at))
            .collect();
        let entries: Vec<(&str, &str)> = paths
            .iter()
            .zip(versions)
            .map(|(path, v)| (path.as_str(), v.cid.as_str()))
            .collect();
        let site = check_site(ipfs, &entries).await;
        for ((v, path), health) in versions.iter().zip(&paths).zip(site.versions) {
            if health != VersionHealth::Ok {
                problems += 1;
            }
            lines.push(StatusLine::Version(VersionStatus {
                pubkey,
                d: d.to_string(),
                path: path.clone(),
                cid: v.cid.clone(),
                size: v.size,
                created_at: v.created_at,
                health,
            }));
        }
        sites.push(SiteSize {
            pubkey,
            d: d.to_string(),
            path: layout.agent_site(pubkey_hex, d),
            actual: site.actual_size,
        });
    }

    let garbage = find_garbage(ipfs, &layout, &state).await;
    problems += garbage.paths.len() + garbage.unlisted.len();

    Ok(StatusReport {
        state_path,
        lines,
        sites,
        garbage,
        problems,
    })
}

fn bytes_or_unknown(size: Option<u64>) -> String {
    size.map_or_else(|| "unknown".to_string(), |n| n.to_string())
}

fn print_status(report: &StatusReport) {
    println!("Stored versions ({}):", report.state_path.display());
    if report.lines.is_empty() {
        println!("  (none)");
    }
    for line in &report.lines {
        match line {
            StatusLine::InvalidKey { key, cid } => {
                println!("  {key} cid={cid} [invalid site key]");
            }
            StatusLine::Version(v) => {
                let detail = match &v.health {
                    VersionHealth::Ok => String::new(),
                    other => format!(": {other}"),
                };
                println!(
                    "  {} cid={} size={} [{}]{detail}",
                    v.path,
                    v.cid,
                    v.size,
                    v.health.label()
                );
            }
        }
    }

    println!();
    println!("Actual size (blocks the versions share are counted once):");
    if report.sites.is_empty() {
        println!("  (none)");
    }
    for site in &report.sites {
        println!("  {} {}", site.path, bytes_or_unknown(site.actual));
    }
    if !report.sites.is_empty() {
        println!("  total {}", bytes_or_unknown(report.actual_bytes()));
    }

    println!();
    println!("Not in state (the agent removes them on its next sweep):");
    if report.garbage.paths.is_empty() && report.garbage.unlisted.is_empty() {
        println!("  (none)");
    }
    for path in &report.garbage.paths {
        println!("  {path}");
    }
    for (path, error) in &report.garbage.unlisted {
        println!("  {path} [list failed]: {error}");
    }
}

pub async fn status(config: &Config) -> Result<()> {
    let ipfs = IpfsClient::new(config.ipfs.api.clone());
    let report = collect_status(&ipfs, config).await?;
    print_status(&report);
    if report.problems > 0 {
        bail!("{} problem(s) found", report.problems);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use crate::ipfs::{FetchLimits, Fetched};
    use crate::state::VersionRecord;

    use super::*;

    #[derive(Default)]
    struct FakeKubo {
        mfs: BTreeMap<String, String>,
        incomplete: HashSet<String>,
        fail_list: HashSet<String>,
        fail_stat: HashSet<String>,
        unions: BTreeMap<String, u64>,
        dag_stats: Mutex<Vec<String>>,
    }

    impl KuboStore for FakeKubo {
        async fn fetch_dag(&self, _cid: &str, _limits: FetchLimits) -> Result<Fetched> {
            unreachable!()
        }

        async fn dag_size_local(&self, cids: &[&str]) -> Result<u64> {
            let key = cids.join(",");
            self.dag_stats.lock().unwrap().push(key.clone());
            for cid in cids {
                if self.incomplete.contains(*cid) {
                    bail!("block not found");
                }
            }
            Ok(self.unions.get(&key).copied().unwrap_or(cids.len() as u64))
        }

        async fn mfs_put(&self, _cid: &str, _path: &str) -> Result<()> {
            unreachable!()
        }

        async fn mfs_remove(&self, _path: &str) -> Result<()> {
            unreachable!()
        }

        async fn mfs_list(&self, path: &str) -> Result<Vec<MfsEntry>> {
            if self.fail_list.contains(path) {
                bail!("simulated files/ls failure");
            }
            let prefix = format!("{path}/");
            let mut entries: BTreeMap<String, MfsEntry> = BTreeMap::new();
            for (p, cid) in &self.mfs {
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
                        cid: cid.clone(),
                    },
                );
            }
            Ok(entries.into_values().collect())
        }

        async fn mfs_stat_cid(&self, path: &str) -> Result<Option<String>> {
            if self.fail_stat.contains(path) {
                bail!("simulated files/stat failure");
            }
            Ok(self.mfs.get(path).cloned())
        }
    }

    const PK: &str = "ab";

    fn layout() -> MfsLayout {
        MfsLayout::new("/swing")
    }

    fn state_with(entries: &[(&str, u64)]) -> State {
        let mut state = State::default();
        for (d, created_at) in entries {
            state.apply_store(
                &state::site_key(PK, d),
                VersionRecord {
                    cid: format!("bafy-{d}-{created_at}"),
                    size: 1,
                    created_at: *created_at,
                    stored_at: *created_at,
                },
            );
        }
        state
    }

    #[tokio::test]
    async fn check_version_classifies_each_problem() {
        let l = layout();
        let path = |d| l.agent_version(PK, d, 1);
        let mut kubo = FakeKubo::default();
        kubo.mfs.insert(path("ok"), "bafy-ok".into());
        kubo.mfs.insert(path("moved"), "bafy-other".into());
        kubo.mfs.insert(path("broken"), "bafy-broken".into());
        kubo.incomplete.insert("bafy-broken".into());
        kubo.fail_stat.insert(path("flaky"));

        assert_eq!(
            check_version(&kubo, &path("ok"), "bafy-ok").await,
            VersionHealth::Ok
        );
        assert_eq!(
            check_version(&kubo, &path("gone"), "bafy-gone").await,
            VersionHealth::Missing
        );
        assert_eq!(
            check_version(&kubo, &path("moved"), "bafy-moved").await,
            VersionHealth::Mismatch("bafy-other".into())
        );
        let broken = check_version(&kubo, &path("broken"), "bafy-broken").await;
        assert!(matches!(broken, VersionHealth::Incomplete(_)));
        assert!(broken.is_broken());
        let flaky = check_version(&kubo, &path("flaky"), "bafy-flaky").await;
        assert!(matches!(flaky, VersionHealth::CheckFailed(_)));
        assert!(!flaky.is_broken());
    }

    #[tokio::test]
    async fn check_site_measures_every_version_in_one_dag_stat() {
        let l = layout();
        let mut kubo = FakeKubo::default();
        for created_at in [1u64, 2] {
            kubo.mfs.insert(
                l.agent_version(PK, "a.example", created_at),
                format!("bafy-{created_at}"),
            );
        }
        kubo.unions.insert("bafy-1,bafy-2".into(), 150);
        let paths: Vec<String> = [1u64, 2]
            .iter()
            .map(|c| l.agent_version(PK, "a.example", *c))
            .collect();
        let entries = vec![(paths[0].as_str(), "bafy-1"), (paths[1].as_str(), "bafy-2")];

        let site = check_site(&kubo, &entries).await;

        assert_eq!(site.versions, vec![VersionHealth::Ok, VersionHealth::Ok]);
        assert_eq!(site.actual_size, Some(150));
        assert_eq!(*kubo.dag_stats.lock().unwrap(), vec!["bafy-1,bafy-2"]);
    }

    #[tokio::test]
    async fn check_site_finds_the_broken_version_and_sizes_the_rest() {
        let l = layout();
        let mut kubo = FakeKubo::default();
        for created_at in [1u64, 2, 3] {
            kubo.mfs.insert(
                l.agent_version(PK, "a.example", created_at),
                format!("bafy-{created_at}"),
            );
        }
        kubo.mfs
            .remove(&l.agent_version(PK, "a.example", 3))
            .unwrap();
        kubo.incomplete.insert("bafy-2".into());
        kubo.unions.insert("bafy-1".into(), 40);
        let paths: Vec<String> = [1u64, 2, 3]
            .iter()
            .map(|c| l.agent_version(PK, "a.example", *c))
            .collect();
        let entries = vec![
            (paths[0].as_str(), "bafy-1"),
            (paths[1].as_str(), "bafy-2"),
            (paths[2].as_str(), "bafy-3"),
        ];

        let site = check_site(&kubo, &entries).await;

        assert_eq!(site.versions[0], VersionHealth::Ok);
        assert!(matches!(site.versions[1], VersionHealth::Incomplete(_)));
        assert_eq!(site.versions[2], VersionHealth::Missing);
        assert_eq!(site.actual_size, Some(40));
        assert_eq!(
            *kubo.dag_stats.lock().unwrap(),
            vec!["bafy-1,bafy-2", "bafy-1", "bafy-2", "bafy-1"]
        );
    }

    #[tokio::test]
    async fn find_garbage_reports_the_highest_unreferenced_path() {
        let l = layout();
        let state = state_with(&[("kept.example", 2)]);
        let mut kubo = FakeKubo::default();
        let account = l.agent_account(PK);
        for path in [
            l.agent_version(PK, "kept.example", 2),
            l.agent_version(PK, "kept.example", 1),
            l.agent_version(PK, "gone.example", 1),
            l.agent_version(PK, "gone.example", 2),
            format!("{account}/stray-file"),
            "/swing/agent/cd/a.example/1".to_string(),
            "/swing/agent/loose".to_string(),
            l.publish_version(PK, "kept.example", 1),
        ] {
            kubo.mfs.insert(path, "bafy".into());
        }

        let garbage = find_garbage(&kubo, &l, &state).await;

        assert_eq!(
            garbage.paths,
            vec![
                l.agent_site(PK, "gone.example"),
                l.agent_version(PK, "kept.example", 1),
                format!("{account}/stray-file"),
                "/swing/agent/cd".to_string(),
                "/swing/agent/loose".to_string(),
            ]
        );
        assert!(garbage.unlisted.is_empty());
    }

    #[tokio::test]
    async fn find_garbage_keeps_directories_it_could_not_list() {
        let l = layout();
        let state = State::default();
        let mut kubo = FakeKubo::default();
        kubo.mfs
            .insert(l.agent_version(PK, "a.example", 1), "bafy".into());
        kubo.fail_list.insert(l.agent_site(PK, "a.example"));

        let garbage = find_garbage(&kubo, &l, &state).await;

        assert!(garbage.paths.is_empty());
        assert_eq!(garbage.unlisted.len(), 1);
        assert_eq!(garbage.unlisted[0].0, l.agent_site(PK, "a.example"));
    }
}
