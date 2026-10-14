use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, PublicKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionRecord {
    pub cid: String,
    pub size: u64,
    pub created_at: u64,
    pub stored_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
}

impl VersionRecord {
    pub fn announced_at(&self) -> u64 {
        self.event.as_ref().map_or(self.created_at, |e| {
            e.created_at.as_secs().max(self.created_at)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Verification {
    pub status: String,
    pub detail: Option<String>,
    pub checked_at: u64,
}

pub type SiteKey = String;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Withdrawn {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub events: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through: Option<u64>,
}

impl Withdrawn {
    pub fn covers(&self, event_id_hex: &str, created_at: u64) -> bool {
        self.events.contains(event_id_hex) || self.through.is_some_and(|at| created_at <= at)
    }

    // A newer version in the same second only outranks the held one by id, so covering that second would also cover the newer version.
    pub fn superseded(held: &Event, newer: &Event) -> Self {
        let newer_at = newer.created_at.as_secs();
        if newer_at > held.created_at.as_secs() {
            Self {
                events: BTreeSet::new(),
                through: Some(newer_at - 1),
            }
        } else {
            Self {
                events: BTreeSet::from([held.id.to_hex()]),
                through: None,
            }
        }
    }
}

fn latest_index(versions: &[VersionRecord]) -> Option<usize> {
    versions
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| v.created_at)
        .map(|(i, _)| i)
}

pub fn site_key(pubkey_hex: &str, d: &str) -> SiteKey {
    format!("{pubkey_hex}:{d}")
}

pub fn split_site_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

pub fn account_entries<'a, V>(
    map: &'a BTreeMap<SiteKey, V>,
    pubkey_hex: &str,
) -> impl Iterator<Item = (&'a SiteKey, &'a V)> + use<'a, V> {
    let prefix = site_key(pubkey_hex, "");
    map.range(prefix.clone()..)
        .take_while(move |(k, _)| k.starts_with(&prefix))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    pub sites: BTreeMap<SiteKey, Vec<VersionRecord>>,
    pub verifications: BTreeMap<SiteKey, Verification>,
    pub follow_set: Option<Event>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub withdrawn: BTreeMap<SiteKey, Withdrawn>,
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
            crate::auth::create_private_dir_all(parent)
                .with_context(|| format!("creating state dir {}", parent.display()))?;
        }
        crate::auth::write_private_file(path, text)
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
        account_entries(&self.sites, pubkey_hex)
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
        let keys: BTreeSet<SiteKey> = account_entries(&self.sites, pubkey_hex)
            .map(|(k, _)| k)
            .chain(account_entries(&self.verifications, pubkey_hex).map(|(k, _)| k))
            .cloned()
            .collect();
        for key in &keys {
            self.remove_site(key);
        }
        keys.into_iter().collect()
    }

    pub fn account_site_count(&self, pubkey_hex: &str) -> usize {
        account_entries(&self.sites, pubkey_hex).count()
    }

    pub fn prune_unstored_verifications(&mut self, pubkey_hex: &str, keep: usize) {
        let mut unstored: Vec<(u64, SiteKey)> = account_entries(&self.verifications, pubkey_hex)
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
        let versions = self.sites.entry(key.clone()).or_default();
        let newer =
            latest_index(versions).is_none_or(|i| record.created_at > versions[i].created_at);
        versions.push(record);
        let latest = latest_index(versions);
        for (i, v) in versions.iter_mut().enumerate() {
            if Some(i) != latest {
                v.event = None;
            }
        }
        if newer {
            self.forget_withdrawn_ids(key);
        }
    }

    fn forget_withdrawn_ids(&mut self, key: &str) {
        if let Some(withdrawn) = self.withdrawn.get_mut(key) {
            withdrawn.events.clear();
            if withdrawn.through.is_none() {
                self.withdrawn.remove(key);
            }
        }
    }

    pub fn latest_version(&self, key: &str) -> Option<&VersionRecord> {
        let versions = self.sites.get(key)?;
        latest_index(versions).map(|i| &versions[i])
    }

    // The record's `created_at` names the version's MFS path, so a later signing of the same CID replaces only the event.
    pub fn fill_latest_event(&mut self, key: &str, cid: &str, event: &Event) -> bool {
        let Some(versions) = self.sites.get_mut(key) else {
            return false;
        };
        let Some(latest) = latest_index(versions).map(|i| &mut versions[i]) else {
            return false;
        };
        let created_at = event.created_at.as_secs();
        let newer = match &latest.event {
            Some(held) => crate::nostr::is_newer_replaceable(event, held),
            None => created_at >= latest.created_at,
        };
        if latest.cid != cid
            || !newer
            || self
                .withdrawn
                .get(key)
                .is_some_and(|w| w.covers(&event.id.to_hex(), created_at))
            || event.verify().is_err()
        {
            return false;
        }
        if latest.event.replace(event.clone()).is_some() {
            self.forget_withdrawn_ids(key);
        }
        true
    }

    pub fn note_newer_version(&mut self, key: &str, cid: &str, event: &Event) -> bool {
        let Some(latest) = self.latest_version(key) else {
            return false;
        };
        if latest.cid == cid {
            return self.fill_latest_event(key, cid, event);
        }
        let newer_at = event.created_at.as_secs();
        let withdrawn = match &latest.event {
            Some(held) if crate::nostr::is_newer_replaceable(event, held) => {
                Withdrawn::superseded(held, event)
            }
            None if newer_at > latest.created_at => Withdrawn {
                events: BTreeSet::new(),
                through: Some(newer_at - 1),
            },
            _ => return false,
        };
        if event.verify().is_err() {
            return false;
        }
        self.record_withdrawal(key, withdrawn)
    }

    pub fn listed_site_events(&self, site_event_kind: u16, authors: &[PublicKey]) -> Vec<Event> {
        authors
            .iter()
            .flat_map(|author| {
                account_entries(&self.sites, &author.to_hex())
                    .filter_map(move |(key, versions)| {
                        let (_, d) = split_site_key(key)?;
                        let record = &versions[latest_index(versions)?];
                        let event = record.event.as_ref()?;
                        let site =
                            crate::nostr::saved_site_event(event, author, d, site_event_kind)?;
                        (site.cid == record.cid && site.created_at >= record.created_at)
                            .then(|| event.clone())
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn latest_stored_at(&self) -> Option<u64> {
        self.sites.values().flatten().map(|v| v.stored_at).max()
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
            self.withdrawn.remove(key);
        }
        removed
    }

    fn remove_site(&mut self, key: &SiteKey) -> Vec<VersionRecord> {
        self.verifications.remove(key);
        self.withdrawn.remove(key);
        self.sites.remove(key).unwrap_or_default()
    }

    pub fn record_withdrawal(&mut self, key: &str, withdrawn: Withdrawn) -> bool {
        if !self.sites.contains_key(key) {
            return false;
        }
        let entry = self.withdrawn.entry(key.to_string()).or_default();
        let before = entry.clone();
        entry.events.extend(withdrawn.events);
        if let Some(at) = withdrawn.through {
            entry.through = Some(entry.through.map_or(at, |t| t.max(at)));
        }
        *entry != before
    }

    pub fn set_verification(&mut self, key: &SiteKey, verification: Verification) {
        self.verifications.insert(key.clone(), verification);
    }
}

#[cfg(test)]
mod tests;
