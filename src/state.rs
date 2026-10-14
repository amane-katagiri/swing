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
mod tests {
    use super::*;
    use crate::test_support::version;

    #[test]
    fn latest_stored_at_is_the_newest_across_all_sites() {
        let mut state = State::default();
        assert_eq!(state.latest_stored_at(), None);
        let record = |cid: &str, stored_at| VersionRecord {
            stored_at,
            ..version(cid, 1, 1)
        };
        state.apply_store(&site_key("aa", "x.example"), record("one", 30));
        state.apply_store(&site_key("aa", "x.example"), record("two", 10));
        state.apply_store(&site_key("bb", "y.example"), record("three", 20));
        assert_eq!(state.latest_stored_at(), Some(30));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn saved_state_and_new_state_dir_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("state");
        let path = state_dir.join("state.json");
        State::default().save(&path).await.unwrap();
        assert_eq!(crate::test_support::mode(&state_dir), 0o700);
        assert_eq!(crate::test_support::mode(&path), 0o600);
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

    #[tokio::test]
    async fn state_saved_before_events_and_withdrawals_were_kept_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"sites":{"aa:x.example":[{"cid":"c","size":1,"created_at":1,"stored_at":2}]},"verifications":{}}"#,
        )
        .unwrap();
        let state = State::load(&path).await.unwrap();
        assert_eq!(state.latest_version("aa:x.example").unwrap().event, None);
        assert!(state.withdrawn.is_empty());
        state.save(&path).await.unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("withdrawn")
        );
    }

    fn signed(created_at: u64) -> Event {
        use nostr_sdk::prelude::*;
        EventBuilder::new(Kind::TextNote, "")
            .custom_created_at(Timestamp::from_secs(created_at))
            .finalize(&Keys::generate())
            .unwrap()
    }

    fn record(cid: &str, created_at: u64, event: Option<Event>) -> VersionRecord {
        VersionRecord {
            event,
            ..version(cid, 1, created_at)
        }
    }

    #[test]
    fn listed_site_events_are_the_matching_events_of_the_latest_versions() {
        use crate::test_support::{CID_A, CID_B, keys, make_site_event};
        let author = keys();
        let other = keys();
        let hex = author.public_key().to_hex();
        let event = |d: &str, cid: &str, at: u64| make_site_event(&author, 35980, d, cid, at);
        let mut state = State::default();
        let kept = event("kept.example", CID_A, 20);
        state.apply_store(&site_key(&hex, "kept.example"), record(CID_B, 10, None));
        state.apply_store(
            &site_key(&hex, "kept.example"),
            record(CID_A, 20, Some(kept.clone())),
        );
        state.apply_store(
            &site_key(&hex, "other-cid.example"),
            record(CID_B, 20, Some(event("other-cid.example", CID_A, 20))),
        );
        state.apply_store(
            &site_key(&hex, "other-d.example"),
            record(CID_A, 20, Some(event("kept.example", CID_A, 20))),
        );
        state.apply_store(&site_key(&hex, "no-event.example"), record(CID_A, 20, None));
        let withdrawn = event("withdrawn.example", CID_A, 20);
        state.apply_store(
            &site_key(&hex, "withdrawn.example"),
            record(CID_A, 20, Some(withdrawn.clone())),
        );
        state.record_withdrawal(
            &site_key(&hex, "withdrawn.example"),
            Withdrawn {
                events: BTreeSet::new(),
                through: Some(20),
            },
        );
        let other_hex = other.public_key().to_hex();
        state.apply_store(
            &site_key(&other_hex, "x.example"),
            record(
                CID_A,
                20,
                Some(make_site_event(&other, 35980, "x.example", CID_A, 20)),
            ),
        );

        assert_eq!(
            state.listed_site_events(35980, &[author.public_key()]),
            vec![kept, withdrawn]
        );
        assert!(
            state
                .listed_site_events(35981, &[author.public_key()])
                .is_empty()
        );
    }

    #[tokio::test]
    async fn only_the_latest_version_keeps_its_event() {
        let key = site_key("aa", "x.example");
        let mut state = State::default();
        state.apply_store(&key, record("one", 10, Some(signed(10))));
        state.apply_store(&key, record("two", 20, Some(signed(20))));
        state.apply_store(&key, record("older", 5, Some(signed(5))));
        let events: Vec<(u64, bool)> = state.sites[&key]
            .iter()
            .map(|v| (v.created_at, v.event.is_some()))
            .collect();
        assert_eq!(events, vec![(10, false), (20, true), (5, false)]);
        assert_eq!(state.latest_version(&key).unwrap().cid, "two");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        state.save(&path).await.unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("\"event\"").count(), 1);
        let loaded = State::load(&path).await.unwrap();
        assert_eq!(loaded.sites[&key], state.sites[&key]);
    }

    #[test]
    fn a_missing_event_is_filled_only_into_the_latest_version_with_its_cid() {
        let key = site_key("aa", "x.example");
        let mut state = State::default();
        state.apply_store(&key, record("old", 10, None));
        state.apply_store(&key, record("new", 20, None));

        assert!(!state.fill_latest_event(&key, "old", &signed(10)));
        assert!(!state.fill_latest_event(&key, "new", &signed(19)));
        assert!(!state.fill_latest_event(&key, "other", &signed(20)));
        assert!(!state.fill_latest_event("aa:none.example", "new", &signed(20)));
        let event = signed(20);
        assert!(state.fill_latest_event(&key, "new", &event));
        assert!(!state.fill_latest_event(&key, "new", &event));
        assert!(!state.fill_latest_event(&key, "new", &signed(19)));
        assert_eq!(state.latest_version(&key).unwrap().event, Some(event));
        assert_eq!(state.sites[&key][0].event, None);
    }

    fn with_bad_signature(event: &Event) -> Event {
        let mut forged = event.clone();
        forged.sig = signed(event.created_at.as_secs()).sig;
        forged
    }

    #[test]
    fn an_event_with_a_bad_signature_is_not_filled() {
        let key = site_key("aa", "x.example");
        let good = signed(20);
        let mut state = State::default();
        state.apply_store(&key, record("new", 20, None));

        assert!(!state.fill_latest_event(&key, "new", &with_bad_signature(&good)));
        assert_eq!(state.latest_version(&key).unwrap().event, None);
    }

    #[test]
    fn a_newer_version_of_another_cid_is_recorded_as_superseding_the_latest() {
        let key = site_key("aa", "x.example");
        let mut state = State::default();
        state.apply_store(&key, record("old", 10, None));
        assert!(!state.note_newer_version(&key, "other", &signed(10)));
        assert!(!state.note_newer_version(&key, "other", &with_bad_signature(&signed(20))));
        assert!(state.note_newer_version(&key, "other", &signed(20)));
        assert_eq!(state.withdrawn[&key], through(19));
        assert!(!state.note_newer_version("aa:none.example", "other", &signed(20)));

        let a = signed(30);
        let b = signed(30);
        let (newer, held) = if a.id < b.id { (a, b) } else { (b, a) };
        let mut state = State::default();
        state.apply_store(&key, record("held", 30, Some(held.clone())));
        assert!(!state.note_newer_version(&key, "other", &signed(29)));
        assert!(state.note_newer_version(&key, "other", &newer));
        assert_eq!(state.withdrawn[&key], by_id(&held.id.to_hex()));

        let later = signed(40);
        assert!(state.note_newer_version(&key, "held", &later));
        assert_eq!(state.latest_version(&key).unwrap().event, Some(later));
    }

    #[test]
    fn a_later_signing_of_the_latest_cid_replaces_its_event_but_keeps_its_time() {
        let key = site_key("aa", "x.example");
        let mut state = State::default();
        state.apply_store(&key, record("new", 20, Some(signed(20))));
        state.record_withdrawal(&key, by_id("e1"));
        state.record_withdrawal(&key, through(15));
        let later = signed(30);

        assert!(!state.fill_latest_event(&key, "other", &later));
        assert!(!state.fill_latest_event(&key, "new", &with_bad_signature(&later)));
        assert!(state.fill_latest_event(&key, "new", &later));
        let latest = state.latest_version(&key).unwrap();
        assert_eq!(latest.event, Some(later.clone()));
        assert_eq!(latest.created_at, 20);
        assert_eq!(latest.announced_at(), 30);
        assert_eq!(state.withdrawn[&key], through(15));
        assert!(!state.fill_latest_event(&key, "new", &signed(25)));

        let unsigned = site_key("aa", "y.example");
        state.apply_store(&unsigned, record("old-format", 20, None));
        assert!(state.fill_latest_event(&unsigned, "old-format", &later));
        assert_eq!(state.sites[&unsigned][0].created_at, 20);
    }

    #[test]
    fn a_signing_covered_by_a_withdrawal_is_not_kept() {
        let key = site_key("aa", "x.example");
        let held = signed(20);
        let mut state = State::default();
        state.apply_store(&key, record("new", 20, Some(held.clone())));
        state.record_withdrawal(&key, through(30));

        assert!(!state.fill_latest_event(&key, "new", &signed(30)));
        assert_eq!(state.latest_version(&key).unwrap().event, Some(held));
        assert!(state.fill_latest_event(&key, "new", &signed(31)));
    }

    #[test]
    fn a_withdrawal_covers_its_events_and_versions_up_to_the_address_deletion() {
        let withdrawn = Withdrawn {
            events: BTreeSet::from(["aa".to_string()]),
            through: Some(200),
        };
        assert!(withdrawn.covers("aa", 900));
        assert!(withdrawn.covers("bb", 200));
        assert!(!withdrawn.covers("bb", 201));
        assert!(!Withdrawn::default().covers("aa", 1));
    }

    fn by_id(event_id_hex: &str) -> Withdrawn {
        Withdrawn {
            events: BTreeSet::from([event_id_hex.to_string()]),
            through: None,
        }
    }

    fn through(at: u64) -> Withdrawn {
        Withdrawn {
            events: BTreeSet::new(),
            through: Some(at),
        }
    }

    #[test]
    fn a_newer_version_drops_the_withdrawn_ids_but_keeps_the_range() {
        let key = site_key("aa", "x.example");
        let mut state = State::default();
        state.apply_store(&key, record("one", 10, None));
        state.record_withdrawal(&key, by_id("e1"));
        state.apply_store(&key, record("older", 5, None));
        state.apply_store(&key, record("same", 10, None));
        assert_eq!(state.withdrawn[&key], by_id("e1"));
        state.apply_store(&key, record("two", 20, None));
        assert!(state.withdrawn.is_empty());

        state.record_withdrawal(&key, by_id("e2"));
        state.record_withdrawal(&key, through(15));
        state.apply_store(&key, record("three", 30, None));
        assert_eq!(state.withdrawn[&key], through(15));
    }

    #[tokio::test]
    async fn withdrawals_are_kept_for_stored_sites_and_dropped_with_them() {
        let key = site_key("aa", "x.example");
        let mut state = State::default();
        assert!(!state.record_withdrawal(&key, by_id("e1")));
        state.apply_store(&key, record("one", 10, None));
        assert!(state.record_withdrawal(&key, by_id("e1")));
        assert!(!state.record_withdrawal(&key, by_id("e1")));
        assert!(state.record_withdrawal(&key, through(20)));
        assert!(!state.record_withdrawal(&key, through(15)));
        assert_eq!(
            state.withdrawn[&key],
            Withdrawn {
                events: BTreeSet::from(["e1".to_string()]),
                through: Some(20),
            }
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        state.save(&path).await.unwrap();
        let loaded = State::load(&path).await.unwrap();
        assert_eq!(loaded.withdrawn, state.withdrawn);

        state.remove_versions(&key, &["one".to_string()]);
        assert!(state.withdrawn.is_empty());
        state.apply_store(&key, record("two", 30, None));
        state.record_withdrawal(&key, through(30));
        state.remove_account("aa");
        assert!(state.withdrawn.is_empty());
    }

    #[test]
    fn site_key_round_trips_even_when_d_contains_colons() {
        let key = site_key("ab", "a:b");
        assert_eq!(split_site_key(&key), Some(("ab", "a:b")));
    }

    #[test]
    fn accounts_and_remove_account_cover_sites_and_verifications() {
        let mut state = State::default();
        state.apply_store(&site_key("aa", "one.example"), version("c", 1, 1));
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
        let record = |cid: &str, size| version(cid, size, 1);
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
        state.apply_store(&site_key("aa", "stored.example"), version("c", 1, 1));
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
    fn remove_site_also_removes_verification() {
        let mut state = State::default();
        let key = site_key("abc", "example.com");
        state.apply_store(&key, version("cid1", 10, 1));
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
        state.apply_store(&key, version("cid1", 10, 1));
        state.apply_store(&key, version("cid2", 20, 2));
        let cids = |removed: Vec<VersionRecord>| -> Vec<String> {
            removed.into_iter().map(|v| v.cid).collect()
        };
        assert_eq!(
            cids(state.remove_versions(&key, &["cid1".to_string(), "missing".to_string()])),
            vec!["cid1".to_string()]
        );
        assert_eq!(state.site_bytes(&key), 20);
        assert_eq!(
            cids(state.remove_versions(&key, &["cid2".to_string()])),
            vec!["cid2".to_string()]
        );
        assert!(!state.sites.contains_key(&key));
        assert!(
            state
                .remove_versions(&key, &["cid2".to_string()])
                .is_empty()
        );
    }
}
