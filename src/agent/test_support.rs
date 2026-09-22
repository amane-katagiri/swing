use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::config::{
    AgentConfig, Config, DashboardConfig, DashboardListen, IpfsConfig, Nip05Mode, NostrConfig,
    PolicyConfig, PublishConfig,
};
use crate::ipfs::{FetchLimits, Fetched, KuboStore, MfsEntry};
use crate::nip05::{Nip05Verify, VerificationResult};
use crate::nostr::ReportRelay;
use crate::state::{self, SiteKey, State, VersionRecord};

use super::Agent;

#[derive(Default)]
pub(super) struct FakeKuboState {
    pub(super) mfs: BTreeMap<String, String>,
    pub(super) put_calls: Vec<String>,
    pub(super) fetched: Vec<String>,
    pub(super) fail_fetch: HashSet<String>,
    pub(super) fail_stat: HashSet<String>,
    pub(super) fail_put: HashSet<String>,
    pub(super) fail_remove: HashSet<String>,
    pub(super) sizes: HashMap<String, u64>,
}

impl FakeKuboState {
    pub(super) fn stores(&self, cid: &str) -> bool {
        self.mfs.values().any(|c| c == cid)
    }

    pub(super) fn paths(&self) -> Vec<String> {
        self.mfs.keys().cloned().collect()
    }
}

#[derive(Default)]
pub(super) struct FakeKubo {
    pub(super) s: Mutex<FakeKuboState>,
    pub(super) gate: tokio::sync::RwLock<()>,
    pub(super) entered_fetch: tokio::sync::Notify,
}

impl FakeKubo {
    pub(super) fn with(f: impl FnOnce(&mut FakeKuboState)) -> Self {
        let kubo = Self::default();
        f(&mut kubo.s.lock().unwrap());
        kubo
    }
}

impl KuboStore for FakeKubo {
    async fn fetch_dag(&self, cid: &str, limits: FetchLimits) -> anyhow::Result<Fetched> {
        self.entered_fetch.notify_one();
        let _open = self.gate.read().await;
        let mut s = self.s.lock().unwrap();
        s.fetched.push(cid.to_string());
        if s.fail_fetch.contains(cid) {
            anyhow::bail!("simulated fetch failure");
        }
        if s.sizes.get(cid).copied().unwrap_or(0) > limits.max_bytes {
            return Ok(Fetched::TooLarge);
        }
        Ok(Fetched::Complete)
    }

    async fn dag_size_local(&self, cids: &[&str]) -> anyhow::Result<u64> {
        let s = self.s.lock().unwrap();
        let mut total = 0;
        for cid in cids {
            if s.fail_stat.contains(*cid) {
                anyhow::bail!("simulated dag/stat failure");
            }
            total += s.sizes.get(*cid).copied().unwrap_or(0);
        }
        Ok(total)
    }

    async fn mfs_put(&self, cid: &str, path: &str) -> anyhow::Result<()> {
        let mut s = self.s.lock().unwrap();
        s.put_calls.push(cid.to_string());
        if s.fail_put.contains(cid) {
            anyhow::bail!("simulated files/cp failure");
        }
        s.mfs.insert(path.to_string(), cid.to_string());
        Ok(())
    }

    async fn mfs_remove(&self, path: &str) -> anyhow::Result<()> {
        let mut s = self.s.lock().unwrap();
        if s.fail_remove.contains(path) {
            anyhow::bail!("simulated files/rm failure");
        }
        let prefix = format!("{path}/");
        s.mfs.retain(|p, _| p != path && !p.starts_with(&prefix));
        Ok(())
    }

    async fn mfs_list(&self, path: &str) -> anyhow::Result<Vec<MfsEntry>> {
        let s = self.s.lock().unwrap();
        let prefix = format!("{path}/");
        let mut entries: BTreeMap<String, MfsEntry> = BTreeMap::new();
        for (p, cid) in &s.mfs {
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
                    cid: if is_dir { String::new() } else { cid.clone() },
                },
            );
        }
        Ok(entries.into_values().collect())
    }

    async fn mfs_stat_cid(&self, path: &str) -> anyhow::Result<Option<String>> {
        Ok(self.s.lock().unwrap().mfs.get(path).cloned())
    }
}

#[derive(Default)]
pub(super) struct FakeNip05 {
    pub(super) results: Mutex<HashMap<String, VerificationResult>>,
    pub(super) calls: Mutex<usize>,
}

impl FakeNip05 {
    pub(super) fn set(&self, d: &str, pubkey_hex: &str, result: VerificationResult) {
        self.results
            .lock()
            .unwrap()
            .insert(format!("{d}:{pubkey_hex}"), result);
    }

    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl Nip05Verify for FakeNip05 {
    async fn verify(&self, d: &str, pubkey_hex: &str) -> VerificationResult {
        *self.calls.lock().unwrap() += 1;
        let key = format!("{d}:{pubkey_hex}");
        self.results
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or_else(|| panic!("unexpected nip05 verify call for {key}"))
    }
}

pub(super) const REPORT_TTL: u64 = 3 * 86_400;

#[derive(Default)]
pub(super) struct FakeRelayState {
    pub(super) stored: Vec<Event>,
    pub(super) sent: Vec<Event>,
    pub(super) fail_fetch: bool,
    pub(super) reject: bool,
}

pub(super) struct FakeRelay {
    pub(super) keys: Keys,
    pub(super) s: Mutex<FakeRelayState>,
}

impl Default for FakeRelay {
    fn default() -> Self {
        Self {
            keys: Keys::generate(),
            s: Mutex::default(),
        }
    }
}

impl ReportRelay for FakeRelay {
    fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    async fn fetch_own_reports(&self, _report_kind: u16) -> anyhow::Result<Vec<Event>> {
        let s = self.s.lock().unwrap();
        if s.fail_fetch {
            anyhow::bail!("simulated fetch failure");
        }
        Ok(s.stored.clone())
    }

    async fn send_report(&self, report: EventBuilder) -> anyhow::Result<bool> {
        let event = report.finalize(&self.keys)?;
        let mut s = self.s.lock().unwrap();
        if s.reject {
            return Ok(false);
        }
        s.sent.push(event.clone());
        s.stored.push(event);
        Ok(true)
    }
}

pub(super) fn test_config(policy: PolicyConfig) -> Config {
    Config {
        nostr: NostrConfig {
            secret_key: "unused".to_string().into(),
            relays: vec![],
            mirror_set: "site-mirror".to_string(),
            site_event_kind: 35980,
            replica_event_kind: 35981,
        },
        ipfs: IpfsConfig {
            api: "http://127.0.0.1:5001".to_string(),
            mfs_root: "/swing".to_string(),
        },
        policy,
        agent: AgentConfig {
            state_dir: std::path::PathBuf::from("./data"),
            poll_interval: Duration::from_secs(300),
            fetch_timeout: Duration::from_secs(60),
            fetch_idle_timeout: Duration::from_secs(10),
            concurrency: 2,
            report_ttl: Duration::from_secs(REPORT_TTL),
        },
        publish: PublishConfig {
            nip05: Nip05Mode::Off,
            keep_versions: 5,
        },
        dashboard: DashboardConfig {
            listen: DashboardListen::Off,
            allowed_hosts: Vec::new(),
            gateway: None,
            custom_css: None,
            desktop_page: None,
            desktop_page_css: None,
            desktop_banner: None,
            max_upload: 2 * (1u64 << 30),
        },
        config_path: None,
    }
}

pub(super) fn default_policy() -> PolicyConfig {
    PolicyConfig {
        max_total_storage: 1_000_000,
        max_per_site: 100_000,
        max_per_account: 1_000_000,
        max_sites_per_account: 10,
        max_update_size: 100_000,
        keep_versions: 5,
        keep_days: 365,
        min_update_interval: 0,
        remove_on_unfollow: true,
        nip05: Nip05Mode::Off,
        nip05_cache_ttl: 86_400,
    }
}

pub(super) fn nip05_policy(mode: Nip05Mode) -> PolicyConfig {
    PolicyConfig {
        nip05: mode,
        ..default_policy()
    }
}

pub(super) struct Fixture {
    pub(super) agent: Arc<Agent<FakeKubo, FakeNip05, FakeRelay>>,
    pub(super) pubkey: PublicKey,
    pub(super) state_path: std::path::PathBuf,
    pub(super) _dir: tempfile::TempDir,
}

impl Fixture {
    pub(super) fn new(policy: PolicyConfig, kubo: FakeKubo) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let pubkey = Keys::generate().public_key();
        let agent = Agent::new(
            test_config(policy),
            kubo,
            FakeNip05::default(),
            FakeRelay::default(),
            State::default(),
            state_path.clone(),
        );
        agent.replace_targets(HashSet::from([pubkey]));
        Self {
            agent: Arc::new(agent),
            pubkey,
            state_path,
            _dir: dir,
        }
    }

    pub(super) fn key(&self, d: &str) -> SiteKey {
        state::site_key(&self.pubkey.to_hex(), d)
    }

    pub(super) fn path(&self, d: &str, created_at: u64) -> String {
        self.agent
            .layout
            .agent_version(&self.pubkey.to_hex(), d, created_at)
    }

    pub(super) fn event(
        &self,
        d: &str,
        cid: &str,
        size: Option<u64>,
        created_at: u64,
    ) -> crate::nostr::SiteEvent {
        crate::nostr::SiteEvent {
            pubkey: self.pubkey,
            d: d.to_string(),
            cid: cid.to_string(),
            url: None,
            size,
            title: None,
            message: None,
            created_at,
        }
    }

    pub(super) async fn seed(&self, d: &str, cid: &str, size: u64, created_at: u64) {
        self.agent.state.lock().await.apply_store(
            &self.key(d),
            VersionRecord {
                cid: cid.to_string(),
                size,
                created_at,
                stored_at: created_at,
            },
        );
        let path = self.path(d, created_at);
        let mut kubo = self.kubo();
        kubo.mfs.insert(path, cid.to_string());
        kubo.sizes.insert(cid.to_string(), size);
    }

    pub(super) async fn apply(&self, ev: crate::nostr::SiteEvent) {
        self.agent.apply_site_event(&ev).await;
    }

    pub(super) fn kubo(&self) -> std::sync::MutexGuard<'_, FakeKuboState> {
        self.agent.ipfs.s.lock().unwrap()
    }

    pub(super) async fn cids(&self, d: &str) -> Vec<String> {
        self.agent
            .state
            .lock()
            .await
            .sites
            .get(&self.key(d))
            .map(|vs| vs.iter().map(|v| v.cid.clone()).collect())
            .unwrap_or_default()
    }

    pub(super) async fn site_bytes(&self, d: &str) -> u64 {
        self.agent.state.lock().await.site_bytes(&self.key(d))
    }

    pub(super) fn relay(&self) -> std::sync::MutexGuard<'_, FakeRelayState> {
        self.agent.reporter.s.lock().unwrap()
    }

    pub(super) fn take_reports(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = std::mem::take(&mut self.relay().sent)
            .iter()
            .map(|e| {
                assert_eq!(e.pubkey, self.agent.own);
                assert_eq!(
                    e.tags.expiration(),
                    Some(e.created_at + Duration::from_secs(REPORT_TTL))
                );
                let cids = e
                    .tags
                    .iter()
                    .filter(|t| t.kind() == "cid")
                    .filter_map(|t| t.content().map(str::to_string))
                    .collect();
                (e.tags.identifier().unwrap(), cids)
            })
            .collect();
        out.sort();
        out
    }

    pub(super) fn own_key(&self, d: &str) -> SiteKey {
        state::site_key(&self.agent.own.to_hex(), d)
    }

    pub(super) fn publish_path(&self, d: &str, name: &str) -> String {
        format!(
            "{}/{name}",
            self.agent.layout.publish_site(&self.agent.own.to_hex(), d)
        )
    }

    pub(super) async fn sent_created_at(&self, key: &str) -> u64 {
        self.agent.reports.lock().await.sent[key].created_at
    }

    pub(super) async fn verification(&self, d: &str) -> Option<String> {
        self.agent
            .state
            .lock()
            .await
            .verifications
            .get(&self.key(d))
            .map(|v| v.status.clone())
    }
}

pub(super) const D: &str = "example.com";

pub(super) fn sized(entries: &[(&str, u64)]) -> FakeKubo {
    FakeKubo::with(|s| {
        for (cid, size) in entries {
            s.sizes.insert(cid.to_string(), *size);
        }
    })
}
