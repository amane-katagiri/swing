use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::config::{CheckMode, Config, IpfsApi, PolicyConfig};
use crate::nip05::{Nip05Verify, VerificationResult};
use crate::nostr::{Paged, ReportRelay};
use crate::state::{self, SiteKey, State, VersionRecord};

use super::Agent;

pub(super) use crate::test_support::{FakeKubo, FakeKuboState};

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
    pub(super) partial_fetch: bool,
    pub(super) own_fetches: usize,
    pub(super) reject: bool,
    pub(super) fail_sign: bool,
    pub(super) send_attempts: usize,
    pub(super) about_since: Vec<Option<u64>>,
    pub(super) about_reporters: Vec<Vec<PublicKey>>,
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

    async fn fetch_own_reports(&self, _report_kind: u16) -> anyhow::Result<Paged> {
        let mut s = self.s.lock().unwrap();
        s.own_fetches += 1;
        if s.fail_fetch {
            anyhow::bail!("simulated fetch failure");
        }
        Ok(Paged {
            events: s.stored.clone(),
            complete: !s.partial_fetch,
        })
    }

    async fn fetch_reports_about(
        &self,
        _report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> anyhow::Result<Vec<Event>> {
        let mut s = self.s.lock().unwrap();
        s.about_since.push(since);
        s.about_reporters.push(reporters.to_vec());
        if s.fail_fetch {
            anyhow::bail!("simulated fetch failure");
        }
        Ok(s.stored
            .iter()
            .filter(|e| e.tags.public_keys().any(|pk| pk == author))
            .filter(|e| since.is_none_or(|since| e.created_at.as_secs() >= since))
            .cloned()
            .collect())
    }

    async fn send_report(&self, report: EventBuilder) -> anyhow::Result<bool> {
        let event = report.finalize(&self.keys)?;
        let mut s = self.s.lock().unwrap();
        s.send_attempts += 1;
        if s.fail_sign {
            anyhow::bail!("simulated signer failure");
        }
        if s.reject {
            return Ok(false);
        }
        s.sent.push(event.clone());
        s.stored.push(event);
        Ok(true)
    }
}

pub(super) fn test_config(policy: PolicyConfig) -> Config {
    let mut config = crate::config::build_config_from_str("", |_| None).unwrap();
    config.nostr.secret_key = Some("unused".to_string().into());
    config.nostr.relays = Vec::new();
    config.nostr.mirror_set = "site-mirror".to_string();
    config.ipfs.api = IpfsApi::Url("http://127.0.0.1:5001".to_string());
    config.policy = policy;
    config.agent.fetch_timeout = Duration::from_secs(60);
    config.agent.fetch_idle_timeout = Duration::from_secs(10);
    config.agent.concurrency = 2;
    config.publish.nip05 = CheckMode::Off;
    config.dashboard.gateway = None;
    config.kubo.storage_max = 1_000_000;
    config.config_path = std::path::PathBuf::from("./swing.toml");
    config.config_exists = true;
    config.sources = std::collections::BTreeMap::new();
    config
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
        nip05: CheckMode::Off,
        nip05_cache_ttl: 86_400,
    }
}

pub(super) fn nip05_policy(mode: CheckMode) -> PolicyConfig {
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
            Arc::default(),
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
            id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
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

    pub(super) async fn choose_reporters(&self, reporters: &[PublicKey]) {
        let event = EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier(&self.agent.config.nostr.mirror_set))
            .tags(reporters.iter().map(|pk| Tag::public_key(*pk)))
            .finalize(&self.agent.reporter.keys)
            .unwrap();
        self.agent.state.lock().await.follow_set = Some(event);
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
