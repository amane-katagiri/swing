use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nostr_sdk::prelude::Event;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionRecord {
    pub cid: String,
    pub size: u64,
    pub created_at: u64,
    pub stored_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Verification {
    pub status: String,
    pub detail: Option<String>,
    pub checked_at: u64,
}

pub type SiteKey = String;

pub fn site_key(pubkey_hex: &str, d: &str) -> SiteKey {
    format!("{pubkey_hex}:{d}")
}

pub fn split_site_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    pub sites: BTreeMap<SiteKey, Vec<VersionRecord>>,
    pub verifications: BTreeMap<SiteKey, Verification>,
    pub follow_set: Option<Event>,
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

    fn write_sync(path: &Path, text: &str) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating state dir {}", parent.display()))?;
        }
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
        let text = serde_json::to_string_pretty(self).context("serializing state")?;
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || Self::write_sync(&path, &text))
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

    pub fn accounts(&self) -> BTreeSet<String> {
        self.sites
            .keys()
            .chain(self.verifications.keys())
            .filter_map(|key| split_site_key(key).map(|(pubkey_hex, _)| pubkey_hex.to_string()))
            .collect()
    }

    pub fn remove_account(&mut self, pubkey_hex: &str) -> Vec<SiteKey> {
        let prefix = format!("{pubkey_hex}:");
        let keys: BTreeSet<SiteKey> = self
            .sites
            .keys()
            .chain(self.verifications.keys())
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in &keys {
            self.remove_site(key);
        }
        keys.into_iter().collect()
    }

    pub fn account_site_count(&self, pubkey_hex: &str) -> usize {
        let prefix = format!("{pubkey_hex}:");
        self.sites
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .count()
    }

    pub fn prune_unstored_verifications(&mut self, pubkey_hex: &str, keep: usize) {
        let prefix = format!("{pubkey_hex}:");
        let mut unstored: Vec<(u64, SiteKey)> = self
            .verifications
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter(|(k, _)| !self.sites.contains_key(*k))
            .map(|(k, v)| (v.checked_at, k.clone()))
            .collect();
        if unstored.len() <= keep {
            return;
        }
        unstored.sort_by(|a, b| b.cmp(a));
        for (_, key) in unstored.into_iter().skip(keep) {
            self.verifications.remove(&key);
        }
    }

    pub fn apply_store(&mut self, key: &SiteKey, record: VersionRecord) {
        self.sites.entry(key.clone()).or_default().push(record);
    }

    pub fn remove_versions(&mut self, key: &SiteKey, cids: &[String]) -> Vec<VersionRecord> {
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

    fn remove_site(&mut self, key: &SiteKey) -> Vec<VersionRecord> {
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
        state.apply_store(
            &site_key("abc", "example.com"),
            VersionRecord {
                cid: "bafyone".into(),
                size: 100,
                created_at: 1,
                stored_at: 2,
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
    fn site_key_round_trips_even_when_d_contains_colons() {
        let key = site_key("ab", "a:b");
        assert_eq!(split_site_key(&key), Some(("ab", "a:b")));
    }

    #[test]
    fn accounts_and_remove_account_cover_sites_and_verifications() {
        let mut state = State::default();
        state.apply_store(
            &site_key("aa", "one.example"),
            VersionRecord {
                cid: "c".into(),
                size: 1,
                created_at: 1,
                stored_at: 1,
            },
        );
        let verification = Verification {
            status: "mismatch".into(),
            detail: None,
            checked_at: 1,
        };
        state.set_verification(&site_key("aa", "two.example"), verification.clone());
        state.set_verification(&site_key("bb", "x.example"), verification);
        assert_eq!(
            state.accounts().into_iter().collect::<Vec<_>>(),
            vec!["aa", "bb"]
        );

        let removed = state.remove_account("aa");
        assert_eq!(removed, vec!["aa:one.example", "aa:two.example"]);
        assert_eq!(state.accounts().into_iter().collect::<Vec<_>>(), vec!["bb"]);
        assert!(state.sites.is_empty());
    }

    #[test]
    fn account_bytes_sums_only_that_accounts_sites() {
        let mut state = State::default();
        let record = |cid: &str, size| VersionRecord {
            cid: cid.into(),
            size,
            created_at: 1,
            stored_at: 1,
        };
        state.apply_store(&site_key("aa", "one.example"), record("c1", 10));
        state.apply_store(&site_key("aa", "two.example"), record("c2", 20));
        state.apply_store(&site_key("aab", "x.example"), record("c3", 40));
        state.apply_store(&site_key("ab", "x.example"), record("c4", 80));
        assert_eq!(state.account_bytes("aa"), 30);
        assert_eq!(state.account_bytes("ab"), 80);
        assert_eq!(state.account_bytes("zz"), 0);
        assert_eq!(state.account_site_count("aa"), 2);
        assert_eq!(state.account_site_count("a"), 0);
    }

    #[test]
    fn prune_unstored_verifications_keeps_stored_and_newest() {
        let mut state = State::default();
        let verification = |checked_at| Verification {
            status: "mismatch".into(),
            detail: None,
            checked_at,
        };
        state.apply_store(
            &site_key("aa", "stored.example"),
            VersionRecord {
                cid: "c".into(),
                size: 1,
                created_at: 1,
                stored_at: 1,
            },
        );
        state.set_verification(&site_key("aa", "stored.example"), verification(1));
        for (d, t) in [("old.example", 2), ("mid.example", 3), ("new.example", 4)] {
            state.set_verification(&site_key("aa", d), verification(t));
        }
        state.set_verification(&site_key("ab", "other.example"), verification(1));

        state.prune_unstored_verifications("aa", 2);

        let keys: Vec<&str> = state.verifications.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "aa:mid.example",
                "aa:new.example",
                "aa:stored.example",
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
        state.apply_store(
            &key,
            VersionRecord {
                cid: "cid1".into(),
                size: 10,
                created_at: 1,
                stored_at: 1,
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
    fn remove_versions_removes_matching_versions_and_empty_site() {
        let mut state = State::default();
        let key = site_key("abc", "example.com");
        state.apply_store(
            &key,
            VersionRecord {
                cid: "cid1".into(),
                size: 10,
                created_at: 1,
                stored_at: 1,
            },
        );
        state.apply_store(
            &key,
            VersionRecord {
                cid: "cid2".into(),
                size: 20,
                created_at: 2,
                stored_at: 2,
            },
        );
        state.remove_versions(&key, &["cid1".to_string()]);
        assert_eq!(state.site_bytes(&key), 20);
        state.remove_versions(&key, &["cid2".to_string()]);
        assert!(!state.sites.contains_key(&key));
    }
}
