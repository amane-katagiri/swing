use std::collections::HashSet;
use std::fmt;

use anyhow::{Result, bail};

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

pub async fn check_version<C: KuboStore>(ipfs: &C, path: &str, cid: &str) -> VersionHealth {
    match ipfs.mfs_stat_cid(path).await {
        Ok(Some(found)) if found == cid => match ipfs.dag_size_local(cid).await {
            Ok(_) => VersionHealth::Ok,
            Err(e) => VersionHealth::Incomplete(format!("{e:#}")),
        },
        Ok(Some(found)) => VersionHealth::Mismatch(found),
        Ok(None) => VersionHealth::Missing,
        Err(e) => VersionHealth::CheckFailed(format!("{e:#}")),
    }
}

pub fn version_path(layout: &MfsLayout, key: &str, created_at: u64) -> Option<String> {
    let (pubkey_hex, d) = state::split_site_key(key)?;
    Some(layout.agent_version(pubkey_hex, d, created_at))
}

#[derive(Debug, Default)]
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

pub async fn status(config: &Config) -> Result<()> {
    let state_path = config.agent.state_dir.join("state.json");
    let state = State::load(&state_path).await?;
    let ipfs = IpfsClient::new(config.ipfs.api.clone());
    let layout = MfsLayout::new(config.ipfs.mfs_root.clone());

    let mut problems = 0usize;
    println!("Stored versions ({}):", state_path.display());
    if state.sites.is_empty() {
        println!("  (none)");
    }
    for (key, versions) in &state.sites {
        for v in versions {
            let Some(path) = version_path(&layout, key, v.created_at) else {
                println!("  {key} cid={} [invalid site key]", v.cid);
                problems += 1;
                continue;
            };
            let health = check_version(&ipfs, &path, &v.cid).await;
            let detail = match &health {
                VersionHealth::Ok => String::new(),
                other => format!(": {other}"),
            };
            println!(
                "  {path} cid={} size={} [{}]{detail}",
                v.cid,
                v.size,
                health.label()
            );
            if health != VersionHealth::Ok {
                problems += 1;
            }
        }
    }

    let garbage = find_garbage(&ipfs, &layout, &state).await;
    println!();
    println!("Not in state (the agent removes them on its next sweep):");
    if garbage.paths.is_empty() && garbage.unlisted.is_empty() {
        println!("  (none)");
    }
    for path in &garbage.paths {
        println!("  {path}");
    }
    for (path, error) in &garbage.unlisted {
        println!("  {path} [list failed]: {error}");
    }
    problems += garbage.paths.len() + garbage.unlisted.len();

    if problems > 0 {
        bail!("{problems} problem(s) found");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::ipfs::{FetchLimits, Fetched};
    use crate::state::VersionRecord;

    use super::*;

    #[derive(Default)]
    struct FakeKubo {
        mfs: BTreeMap<String, String>,
        incomplete: HashSet<String>,
        fail_list: HashSet<String>,
        fail_stat: HashSet<String>,
    }

    impl KuboStore for FakeKubo {
        async fn fetch_dag(&self, _cid: &str, _limits: FetchLimits) -> Result<Fetched> {
            unreachable!()
        }

        async fn dag_size_local(&self, cid: &str) -> Result<u64> {
            if self.incomplete.contains(cid) {
                bail!("block not found");
            }
            Ok(1)
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
