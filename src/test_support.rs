use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use nostr_sdk::prelude::*;

use crate::ipfs::{FetchLimits, Fetched, KuboStore, MfsEntry};
use crate::nostr::{SiteEvent, build_replica_report_builder};

pub(crate) fn keys() -> Keys {
    Keys::generate()
}

pub(crate) const CID_A: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
pub(crate) const CID_B: &str = "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR";

pub(crate) fn replica_report_event(
    reporter: &Keys,
    author: &PublicKey,
    d: &str,
    cids: &[&str],
    created_at: u64,
    expiration: u64,
) -> Event {
    build_replica_report_builder(
        35981,
        35980,
        author,
        d,
        &cids.iter().map(|c| c.to_string()).collect(),
        Timestamp::from_secs(expiration),
    )
    .custom_created_at(Timestamp::from_secs(created_at))
    .finalize(reporter)
    .unwrap()
}

pub(crate) fn site_event_fixture(pubkey: PublicKey, d: &str, created_at: u64) -> SiteEvent {
    SiteEvent {
        pubkey,
        d: d.to_string(),
        cid: CID_A.to_string(),
        url: None,
        size: None,
        title: None,
        message: None,
        created_at,
        id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
    }
}

#[derive(Default)]
pub(crate) struct FakeKuboState {
    pub(crate) mfs: BTreeMap<String, String>,
    pub(crate) put_calls: Vec<String>,
    pub(crate) fetched: Vec<String>,
    pub(crate) fail_fetch: HashSet<String>,
    pub(crate) fail_stat: HashSet<String>,
    pub(crate) fail_put: HashSet<String>,
    pub(crate) fail_remove: HashSet<String>,
    pub(crate) fail_list: HashSet<String>,
    pub(crate) fail_mfs_stat_cid: HashSet<String>,
    pub(crate) sizes: HashMap<String, u64>,
    pub(crate) unions: BTreeMap<String, u64>,
    pub(crate) files: HashSet<String>,
    pub(crate) dag_stats: Vec<String>,
}

impl FakeKuboState {
    pub(crate) fn stores(&self, cid: &str) -> bool {
        self.mfs.values().any(|c| c == cid)
    }

    pub(crate) fn paths(&self) -> Vec<String> {
        self.mfs.keys().cloned().collect()
    }
}

#[derive(Default)]
pub(crate) struct FakeKubo {
    pub(crate) s: Mutex<FakeKuboState>,
    pub(crate) gate: tokio::sync::RwLock<()>,
    pub(crate) entered_fetch: tokio::sync::Notify,
}

impl FakeKubo {
    pub(crate) fn with(f: impl FnOnce(&mut FakeKuboState)) -> Self {
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
        let key = cids.join(",");
        let mut s = self.s.lock().unwrap();
        s.dag_stats.push(key.clone());
        for cid in cids {
            if s.fail_stat.contains(*cid) {
                anyhow::bail!("simulated dag/stat failure");
            }
        }
        if let Some(&total) = s.unions.get(&key) {
            return Ok(total);
        }
        Ok(cids
            .iter()
            .map(|c| s.sizes.get(*c).copied().unwrap_or(0))
            .sum())
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
        if s.fail_list.contains(path) {
            anyhow::bail!("simulated files/ls failure");
        }
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
        let s = self.s.lock().unwrap();
        if s.fail_mfs_stat_cid.contains(path) {
            anyhow::bail!("simulated files/stat failure");
        }
        Ok(s.mfs.get(path).cloned())
    }

    async fn is_directory(&self, cid: &str) -> anyhow::Result<bool> {
        Ok(!self.s.lock().unwrap().files.contains(cid))
    }
}

// Refuses `connect` like Primal does for an app it already knows, so the tests fail if SWING ever sends one.
pub(crate) struct TestSigner {
    pub sign: bool,
}

impl nostr_connect::prelude::NostrConnectSignerActions for TestSigner {
    fn approve(&self, _app: &PublicKey, req: &NostrConnectRequest) -> bool {
        match req {
            NostrConnectRequest::Connect { .. } => false,
            NostrConnectRequest::SignEvent(_) => self.sign,
            _ => true,
        }
    }
}

pub(crate) fn serve_test_signer(uri: &str, user: &Keys, sign: bool) {
    use nostr_connect::prelude::{NostrConnectKeys, NostrConnectRemoteSigner};

    let uri = NostrConnectUri::parse(uri).unwrap();
    let remote = NostrConnectRemoteSigner::from_uri(
        uri,
        NostrConnectKeys::new(Keys::generate(), user.clone()),
        None,
    )
    .unwrap();
    tokio::spawn(async move {
        // The app has to be listening before the signer answers the QR code.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        remote.serve(TestSigner { sign }).await
    });
}
