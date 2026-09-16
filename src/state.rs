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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub sites: BTreeMap<SiteKey, Vec<VersionRecord>>,
    #[serde(default)]
    pub verifications: BTreeMap<SiteKey, Verification>,
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

    pub fn all_pinned_cids(&self) -> impl Iterator<Item = &str> {
        self.sites
            .values()
            .flat_map(|versions| versions.iter())
            .map(|v| v.cid.as_str())
    }

    pub fn apply_pin(&mut self, key: &SiteKey, record: VersionRecord) {
        self.sites.entry(key.clone()).or_default().push(record);
    }

    pub fn apply_unpins(&mut self, key: &SiteKey, cids: &[String]) {
        if let Some(versions) = self.sites.get_mut(key) {
            versions.retain(|v| !cids.contains(&v.cid));
            if versions.is_empty() {
                self.sites.remove(key);
            }
        }
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
    async fn loading_state_without_verifications_key_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"sites":{}}"#).unwrap();
        let state = State::load(&path).await.unwrap();
        assert!(state.verifications.is_empty());
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
            },
        );
        state.apply_pin(
            &key,
            VersionRecord {
                cid: "cid2".into(),
                size: 20,
                created_at: 2,
                pinned_at: 2,
            },
        );
        state.apply_unpins(&key, &["cid1".to_string()]);
        assert_eq!(state.site_bytes(&key), 20);
        state.apply_unpins(&key, &["cid2".to_string()]);
        assert!(!state.sites.contains_key(&key));
    }
}
