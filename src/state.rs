use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionRecord {
    pub cid: String,
    pub size: u64,
    pub created_at: u64,
    pub pinned_at: u64,
    pub preexisting_pin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Verification {
    pub status: String,
    pub detail: Option<String>,
    pub checked_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Releasing {
    pub preexisting_pin: bool,
    pub since: u64,
}

pub type SiteKey = String;

pub fn site_key(pubkey_hex: &str, d: &str) -> SiteKey {
    format!("{pubkey_hex}:{d}")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    pub sites: BTreeMap<SiteKey, Vec<VersionRecord>>,
    pub verifications: BTreeMap<SiteKey, Verification>,
    pub releasing: BTreeMap<String, Releasing>,
}

impl State {
    fn load_sync(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading state file {}", path.display()))?;
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let state: State = serde_json::from_str(&text)
            .with_context(|| format!("parsing state file {}", path.display()))?;
        Ok(state)
    }

    fn save_sync(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating state dir {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).context("serializing state")?;
        let tmp: PathBuf = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }

    pub async fn load(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || Self::load_sync(&path))
            .await
            .context("state load task panicked")?
    }

    pub async fn save(&self, path: &Path) -> Result<()> {
        let path = path.to_path_buf();
        let state = self.clone();
        tokio::task::spawn_blocking(move || state.save_sync(&path))
            .await
            .context("state save task panicked")?
    }

    pub fn total_bytes(&self) -> u64 {
        self.sites
            .values()
            .flat_map(|versions| versions.iter())
            .map(|v| v.size)
            .sum()
    }

    pub fn site_bytes(&self, key: &str) -> u64 {
        self.sites
            .get(key)
            .map(|versions| versions.iter().map(|v| v.size).sum())
            .unwrap_or(0)
    }

    pub fn account_bytes(&self, pubkey_hex: &str) -> u64 {
        let prefix = format!("{pubkey_hex}:");
        self.sites
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .flat_map(|(_, versions)| versions.iter())
            .map(|v| v.size)
            .sum()
    }

    pub fn account_site_count(&self, pubkey_hex: &str) -> usize {
        let prefix = format!("{pubkey_hex}:");
        self.sites
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .count()
    }

    pub fn references_cid(&self, cid: &str) -> bool {
        self.all_pinned_cids().any(|c| c == cid)
    }

    pub fn preexisting_pin(&self, cid: &str) -> Option<bool> {
        let mut flags = self
            .sites
            .values()
            .flat_map(|versions| versions.iter())
            .filter(|v| v.cid == cid)
            .map(|v| v.preexisting_pin)
            .chain(self.releasing.get(cid).map(|r| r.preexisting_pin))
            .peekable();
        flags.peek()?;
        Some(flags.any(|preexisting_pin| preexisting_pin))
    }

    pub fn mark_releasing(&mut self, cid: &str, preexisting_pin: bool, now: u64) {
        self.releasing
            .entry(cid.to_string())
            .and_modify(|r| r.preexisting_pin |= preexisting_pin)
            .or_insert(Releasing {
                preexisting_pin,
                since: now,
            });
    }

    pub fn prune_unpinned_verifications(&mut self, pubkey_hex: &str, keep: usize) {
        let prefix = format!("{pubkey_hex}:");
        let mut unpinned: Vec<(u64, SiteKey)> = self
            .verifications
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, _)| !self.sites.contains_key(*k))
            .map(|(k, v)| (v.checked_at, k.clone()))
            .collect();
        if unpinned.len() <= keep {
            return;
        }
        unpinned.sort_by(|a, b| b.cmp(a));
        for (_, key) in unpinned.into_iter().skip(keep) {
            self.verifications.remove(&key);
        }
    }

    pub fn all_pinned_cids(&self) -> impl Iterator<Item = &str> {
        self.sites
            .values()
            .flat_map(|versions| versions.iter())
            .map(|v| v.cid.as_str())
    }

    pub fn apply_pin(&mut self, key: &SiteKey, record: VersionRecord) {
        self.sites.entry(key.clone()).or_default().push(record);
    }

    pub fn apply_unpins(&mut self, key: &SiteKey, cids: &[String]) -> Vec<VersionRecord> {
        let Some(versions) = self.sites.get_mut(key) else {
            return Vec::new();
        };
        let (removed, kept) = std::mem::take(versions)
            .into_iter()
            .partition(|v| cids.contains(&v.cid));
        *versions = kept;
        if versions.is_empty() {
            self.sites.remove(key);
        }
        removed
    }

    pub fn remove_site(&mut self, key: &SiteKey) -> Vec<VersionRecord> {
        self.verifications.remove(key);
        self.sites.remove(key).unwrap_or_default()
    }

    pub fn set_verification(&mut self, key: &SiteKey, verification: Verification) {
        self.verifications.insert(key.clone(), verification);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = State::default();
        state.apply_pin(
            &site_key("abc", "example.com"),
            VersionRecord {
                cid: "bafyone".into(),
                size: 100,
                created_at: 1,
                pinned_at: 2,
                preexisting_pin: false,
            },
        );
        state.save(&path).await.unwrap();
        let loaded = State::load(&path).await.unwrap();
        assert_eq!(loaded.total_bytes(), 100);
        assert_eq!(loaded.site_bytes(&site_key("abc", "example.com")), 100);
    }

    #[tokio::test]
    async fn load_missing_file_is_empty_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let state = State::load(&path).await.unwrap();
        assert_eq!(state.total_bytes(), 0);
    }

    #[tokio::test]
    async fn loading_state_without_verifications_key_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"sites":{}}"#).unwrap();
        assert!(State::load(&path).await.is_err());
    }

    #[test]
    fn account_bytes_sums_only_that_accounts_sites() {
        let mut state = State::default();
        let record = |cid: &str, size| VersionRecord {
            cid: cid.into(),
            size,
            created_at: 1,
            pinned_at: 1,
            preexisting_pin: false,
        };
        state.apply_pin(&site_key("aa", "one.example"), record("c1", 10));
        state.apply_pin(&site_key("aa", "two.example"), record("c2", 20));
        state.apply_pin(&site_key("aab", "x.example"), record("c3", 40));
        state.apply_pin(&site_key("ab", "x.example"), record("c4", 80));
        assert_eq!(state.account_bytes("aa"), 30);
        assert_eq!(state.account_bytes("ab"), 80);
        assert_eq!(state.account_bytes("zz"), 0);
        assert!(state.references_cid("c3"));
        assert!(!state.references_cid("c5"));
    }

    #[test]
    fn releasing_entries_count_for_preexisting_pin_and_stay_sticky() {
        let mut state = State::default();
        state.mark_releasing("c1", false, 10);
        assert_eq!(state.preexisting_pin("c1"), Some(false));
        state.mark_releasing("c1", true, 20);
        state.mark_releasing("c1", false, 30);
        assert_eq!(
            state.releasing.get("c1"),
            Some(&Releasing {
                preexisting_pin: true,
                since: 10
            })
        );
        assert_eq!(state.preexisting_pin("c1"), Some(true));
        assert!(!state.references_cid("c1"));
    }

    #[test]
    fn preexisting_pin_is_known_only_for_referenced_cids() {
        let mut state = State::default();
        let record = |cid: &str, preexisting_pin| VersionRecord {
            cid: cid.into(),
            size: 1,
            created_at: 1,
            pinned_at: 1,
            preexisting_pin,
        };
        state.apply_pin(&site_key("aa", "one.example"), record("c1", false));
        state.apply_pin(&site_key("bb", "two.example"), record("c1", true));
        state.apply_pin(&site_key("bb", "two.example"), record("c2", false));
        assert_eq!(state.preexisting_pin("c1"), Some(true));
        assert_eq!(state.preexisting_pin("c2"), Some(false));
        assert_eq!(state.preexisting_pin("c3"), None);
        assert_eq!(state.account_site_count("bb"), 1);
        assert_eq!(state.account_site_count("b"), 0);
    }

    #[test]
    fn prune_unpinned_verifications_keeps_pinned_and_newest() {
        let mut state = State::default();
        let verification = |checked_at| Verification {
            status: "mismatch".into(),
            detail: None,
            checked_at,
        };
        state.apply_pin(
            &site_key("aa", "pinned.example"),
            VersionRecord {
                cid: "c".into(),
                size: 1,
                created_at: 1,
                pinned_at: 1,
                preexisting_pin: false,
            },
        );
        state.set_verification(&site_key("aa", "pinned.example"), verification(1));
        for (d, t) in [("old.example", 2), ("mid.example", 3), ("new.example", 4)] {
            state.set_verification(&site_key("aa", d), verification(t));
        }
        state.set_verification(&site_key("ab", "other.example"), verification(1));

        state.prune_unpinned_verifications("aa", 2);

        let keys: Vec<&str> = state.verifications.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "aa:mid.example",
                "aa:new.example",
                "aa:pinned.example",
                "ab:other.example"
            ]
        );
    }

    #[test]
    fn set_verification_overwrites_previous_entry() {
        let mut state = State::default();
        let key = site_key("abc", "example.com");
        state.set_verification(
            &key,
            Verification {
                status: "mismatch".into(),
                detail: None,
                checked_at: 1,
            },
        );
        state.set_verification(
            &key,
            Verification {
                status: "verified".into(),
                detail: None,
                checked_at: 2,
            },
        );
        assert_eq!(state.verifications.get(&key).unwrap().status, "verified");
        assert_eq!(state.verifications.len(), 1);
    }

    #[test]
    fn remove_site_also_removes_verification() {
        let mut state = State::default();
        let key = site_key("abc", "example.com");
        state.apply_pin(
            &key,
            VersionRecord {
                cid: "cid1".into(),
                size: 10,
                created_at: 1,
                pinned_at: 1,
                preexisting_pin: false,
            },
        );
        state.set_verification(
            &key,
            Verification {
                status: "verified".into(),
                detail: None,
                checked_at: 1,
            },
        );
        state.remove_site(&key);
        assert!(state.verifications.is_empty());
    }

    #[test]
    fn apply_unpins_removes_matching_versions_and_empty_site() {
        let mut state = State::default();
        let key = site_key("abc", "example.com");
        state.apply_pin(
            &key,
            VersionRecord {
                cid: "cid1".into(),
                size: 10,
                created_at: 1,
                pinned_at: 1,
                preexisting_pin: false,
            },
        );
        state.apply_pin(
            &key,
            VersionRecord {
                cid: "cid2".into(),
                size: 20,
                created_at: 2,
                pinned_at: 2,
                preexisting_pin: false,
            },
        );
        state.apply_unpins(&key, &["cid1".to_string()]);
        assert_eq!(state.site_bytes(&key), 20);
        state.apply_unpins(&key, &["cid2".to_string()]);
        assert!(!state.sites.contains_key(&key));
    }
}
