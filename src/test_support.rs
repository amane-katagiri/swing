use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use nostr_sdk::prelude::*;

use crate::ipfs::{AutoNat, FetchLimits, Fetched, KuboNode, KuboStore, MfsEntry, Reachability};
use crate::nostr::{SiteEvent, SiteFields, build_replica_report_builder, build_site_event_builder};

pub(crate) fn keys() -> Keys {
    Keys::generate()
}

pub(crate) fn sorted_pubkeys(n: usize) -> Vec<PublicKey> {
    let mut pks: Vec<PublicKey> = (0..n).map(|_| Keys::generate().public_key()).collect();
    pks.sort();
    pks
}

pub(crate) const MINIMAL_TOML: &str = "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n";

pub(crate) fn config_at(
    text: &str,
    path: std::path::PathBuf,
    exists: bool,
) -> crate::config::Config {
    let mut config = crate::config::build_config_from_str(text, |_| None).unwrap();
    config.config_path = path;
    config.config_exists = exists;
    config
}

pub(crate) fn unused_dir() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("swing-test-unused-{}-{n}", std::process::id()))
}

pub(crate) const CID_A: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
pub(crate) const CID_B: &str = "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR";

pub(crate) const PEER_OWN: &str = "12D3KooWFK7pX3nw57FVQT5J99MvHcwhrcpe4hn6xsVm9WqfWJYy";
pub(crate) const PEER_A: &str = "12D3KooWQqg6fSEnehBFPAzVpSwWzFheYrfktHPXLoNDJUisx9LJ";
pub(crate) const PEER_B: &str = "12D3KooWM767xB2P4epkwMAcYQoVDPpnNPa2hofDgzRbTQsnsRQ6";

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

pub(crate) fn version(cid: &str, size: u64, at: u64) -> crate::state::VersionRecord {
    crate::state::VersionRecord {
        cid: cid.to_string(),
        size,
        created_at: at,
        stored_at: at,
        event: None,
    }
}

pub(crate) fn replica_report(
    reporter: &Keys,
    author: &PublicKey,
    d: &str,
    cids: &[&str],
    created_at: u64,
) -> Event {
    replica_report_event(reporter, author, d, cids, created_at, created_at + 100)
}

pub(crate) fn site_event_fixture(
    pubkey: PublicKey,
    d: &str,
    cid: &str,
    created_at: u64,
) -> SiteEvent {
    SiteEvent {
        pubkey,
        d: d.to_string(),
        cid: cid.to_string(),
        url: None,
        size: None,
        title: None,
        message: None,
        created_at,
        id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
    }
}

pub(crate) fn make_site_event(
    keys: &Keys,
    kind: u16,
    d: &str,
    cid: &str,
    created_at: u64,
) -> Event {
    build_site_event_builder(
        kind,
        &SiteFields {
            d,
            cid,
            url: Some("https://example.com/"),
            size: Some(1234),
            ..Default::default()
        },
    )
    .custom_created_at(Timestamp::from_secs(created_at))
    .finalize(keys)
    .unwrap()
}

pub(crate) fn site_event_with(keys: &Keys, d: &str, title: &str, content: &str) -> Event {
    EventBuilder::new(Kind::Custom(35980), content)
        .tag(Tag::identifier(d))
        .tag(Tag::custom("cid", [CID_A.to_string()]))
        .tag(Tag::custom("title", [title.to_string()]))
        .finalize(keys)
        .unwrap()
}

pub(crate) fn follow_set(
    keys: &Keys,
    d: &str,
    title: Option<&str>,
    content: &str,
    p: &[PublicKey],
    created_at: u64,
) -> Event {
    EventBuilder::new(Kind::Custom(crate::nostr::FOLLOW_SET_KIND), content)
        .tag(Tag::identifier(d))
        .tags(title.map(|t| Tag::custom("title", [t.to_string()])))
        .tags(p.iter().map(|pk| Tag::public_key(*pk)))
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(keys)
        .unwrap()
}

pub(crate) fn full_follow_set(keys: &Keys, d: &str) -> Event {
    let p: Vec<PublicKey> = (0..crate::nostr::budget::MAX_FOLLOW_SET_ENTRIES)
        .map(|_| Keys::generate().public_key())
        .collect();
    follow_set(keys, d, None, "", &p, Timestamp::now().as_secs())
}

pub(crate) fn anchor_ad_event(
    keys: &Keys,
    peer: &str,
    reach: crate::nostr::AdReachability,
    created_at: u64,
    expiration: u64,
) -> Event {
    crate::nostr::build_anchor_ad_builder(peer, reach, Timestamp::from_secs(expiration))
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(keys)
        .unwrap()
}

pub(crate) fn raw_event(keys: &Keys, kind: u16, tags: &[&[&str]], created_at: u64) -> Event {
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()))
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(keys)
        .unwrap()
}

pub(crate) fn deletion_event(keys: &Keys, tags: &[[&str; 2]], created_at: u64) -> Event {
    numbered_deletion_event(keys, tags, created_at, "")
}

pub(crate) fn numbered_deletion_event(
    keys: &Keys,
    tags: &[[&str; 2]],
    created_at: u64,
    content: &str,
) -> Event {
    let mut builder = EventBuilder::new(Kind::EventDeletion, content);
    for tag in tags {
        builder = builder.tag(Tag::parse(*tag).unwrap());
    }
    builder
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(keys)
        .unwrap()
}

pub(crate) async fn seed_relay(url: &str, events: &[Event]) {
    let seeder = Client::default();
    seeder.add_relay(url).await.unwrap();
    seeder.connect().await;
    for event in events {
        seeder.send_event(event).await.unwrap();
    }
    seeder.shutdown().await;
}

pub(crate) async fn serve_relay(local: LocalRelay, events: &[Event]) -> (LocalRelay, String) {
    local.run().await.unwrap();
    let url = local.url().await.to_string();
    seed_relay(&url, events).await;
    (local, url)
}

pub(crate) async fn relay_holding(events: &[Event]) -> (LocalRelay, String) {
    serve_relay(LocalRelay::new(), events).await
}

type DbFuture<'a, T> = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = std::result::Result<T, nostr_database::error::Error>>
            + Send
            + 'a,
    >,
>;

#[derive(Debug)]
pub(crate) struct ServesForgeries(pub(crate) Vec<Event>);

impl NostrDatabase for ServesForgeries {
    fn backend(&self) -> &'static str {
        "forgeries"
    }

    fn features(&self) -> Features {
        Features {
            persistent: false,
            event_expiration: false,
            full_text_search: false,
            request_to_vanish: false,
        }
    }

    fn save_event<'a>(&'a self, _event: &'a Event) -> DbFuture<'a, SaveEventStatus> {
        Box::pin(async { Ok(SaveEventStatus::Success) })
    }

    fn check_id<'a>(&'a self, _event_id: &'a EventId) -> DbFuture<'a, DatabaseEventStatus> {
        Box::pin(async { Ok(DatabaseEventStatus::NotExistent) })
    }

    fn event_by_id<'a>(&'a self, _event_id: &'a EventId) -> DbFuture<'a, Option<Event>> {
        Box::pin(async { Ok(None) })
    }

    fn count(&self, _filter: Filter) -> DbFuture<'_, usize> {
        Box::pin(async { Ok(self.0.len()) })
    }

    fn query(&self, _filter: Filter) -> DbFuture<'_, std::collections::BTreeSet<Event>> {
        Box::pin(async { Ok(self.0.iter().cloned().collect()) })
    }

    fn delete(&self, _filter: Filter) -> DbFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn wipe(&self) -> DbFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

pub(crate) async fn forging_relay(events: Vec<Event>) -> (LocalRelay, String) {
    serve_relay(
        LocalRelayBuilder::default()
            .database(ServesForgeries(events))
            .build(),
        &[],
    )
    .await
}

#[derive(Debug)]
pub(crate) struct RefusesQueries {
    pub(crate) only: Option<Kind>,
}

impl QueryPolicy for RefusesQueries {
    fn admit_query<'a>(
        &'a self,
        query: &'a mut Filter,
        _addr: &'a std::net::SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = QueryPolicyResult> + Send + 'a>> {
        Box::pin(async move {
            let refused = self.only.is_none_or(|only| {
                query
                    .kinds
                    .as_ref()
                    .is_some_and(|kinds| kinds.contains(&only))
            });
            if refused {
                QueryPolicyResult::Reject {
                    prefix: MachineReadablePrefix::Error,
                    message: "down for maintenance".into(),
                }
            } else {
                QueryPolicyResult::Accept
            }
        })
    }
}

pub(crate) async fn refusing_relay(only: Option<Kind>) -> (LocalRelay, String) {
    serve_relay(
        LocalRelayBuilder::default()
            .query_policy(RefusesQueries { only })
            .build(),
        &[],
    )
    .await
}

#[derive(Default)]
pub(crate) struct FakeKuboState {
    pub(crate) mfs: BTreeMap<String, String>,
    pub(crate) put_calls: Vec<String>,
    pub(crate) fetched: Vec<String>,
    pub(crate) fail_fetch: HashSet<String>,
    pub(crate) fail_stat: HashSet<String>,
    pub(crate) flaky_stat: HashSet<String>,
    pub(crate) fail_put: HashSet<String>,
    pub(crate) fail_remove: HashSet<String>,
    pub(crate) fail_list: HashSet<String>,
    pub(crate) fail_mfs_stat_cid: HashSet<String>,
    pub(crate) sizes: HashMap<String, u64>,
    pub(crate) unions: BTreeMap<String, u64>,
    pub(crate) files: HashSet<String>,
    pub(crate) dag_stats: Vec<String>,
    pub(crate) dir_checks: Vec<String>,
    pub(crate) fail_dir_check: HashSet<String>,
    pub(crate) peering: std::collections::BTreeSet<String>,
    pub(crate) fail_peering: std::collections::BTreeSet<String>,
    pub(crate) fail_peering_list: bool,
    pub(crate) reachability: Option<Reachability>,
    pub(crate) fail_node: bool,
    pub(crate) node_calls: usize,
    pub(crate) connected: std::collections::BTreeSet<String>,
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

impl KuboNode for FakeKubo {
    async fn peer_id(&self) -> anyhow::Result<String> {
        let mut s = self.s.lock().unwrap();
        s.node_calls += 1;
        if s.fail_node {
            anyhow::bail!("simulated id failure");
        }
        Ok(PEER_OWN.to_string())
    }

    async fn autonat(&self) -> anyhow::Result<AutoNat> {
        let mut s = self.s.lock().unwrap();
        s.node_calls += 1;
        if s.fail_node {
            anyhow::bail!("simulated autonat failure");
        }
        Ok(AutoNat {
            reachability: s.reachability.unwrap_or(Reachability::Unknown),
            nat: None,
            reachable: Vec::new(),
            unreachable: Vec::new(),
            unknown: Vec::new(),
        })
    }

    async fn peering_list(&self) -> anyhow::Result<std::collections::BTreeSet<String>> {
        let s = self.s.lock().unwrap();
        if s.fail_peering_list {
            anyhow::bail!("simulated peering/ls failure");
        }
        Ok(s.peering.clone())
    }

    async fn peering_add(&self, peer_id: &str) -> anyhow::Result<()> {
        let mut s = self.s.lock().unwrap();
        if s.fail_peering.contains(peer_id) {
            anyhow::bail!("simulated peering/add failure");
        }
        s.peering.insert(peer_id.to_string());
        Ok(())
    }

    async fn peering_remove(&self, peer_id: &str) -> anyhow::Result<()> {
        let mut s = self.s.lock().unwrap();
        if s.fail_peering.contains(peer_id) {
            anyhow::bail!("simulated peering/rm failure");
        }
        s.peering.remove(peer_id);
        Ok(())
    }

    async fn connected_peers(&self) -> anyhow::Result<std::collections::BTreeSet<String>> {
        let mut s = self.s.lock().unwrap();
        s.node_calls += 1;
        if s.fail_node {
            anyhow::bail!("simulated swarm/peers failure");
        }
        Ok(s.connected.clone())
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
                anyhow::bail!("block was not found locally (offline): ipld: could not find {cid}");
            }
            if s.flaky_stat.contains(*cid) {
                anyhow::bail!("simulated dag/stat timeout");
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
        let mut s = self.s.lock().unwrap();
        s.dir_checks.push(cid.to_string());
        if s.fail_dir_check.contains(cid) {
            anyhow::bail!("simulated files/stat failure");
        }
        Ok(!s.files.contains(cid))
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

pub(crate) fn identity_route<F>(answer: F) -> axum::routing::MethodRouter
where
    F: Fn() -> (&'static str, String) + Clone + Send + Sync + 'static,
{
    use crate::dashboard::dto::{IdentityDto, IdentityRequestDto};
    use axum::Json;

    axum::routing::post(move |Json(req): Json<IdentityRequestDto>| async move {
        let (token, instance) = answer();
        Json(IdentityDto {
            proof: crate::auth::identity_proof(token, &req.nonce),
            instance,
        })
    })
}

#[cfg(unix)]
pub(crate) fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

pub(crate) fn unused_local_addr() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

pub(crate) async fn serve_router(router: axum::Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

#[derive(Debug)]
pub(crate) struct RefusesWrites;

impl WritePolicy for RefusesWrites {
    fn admit_event<'a>(
        &'a self,
        _event: &'a Event,
        _addr: &'a std::net::SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = WritePolicyResult> + Send + 'a>> {
        Box::pin(async {
            WritePolicyResult::Reject {
                prefix: MachineReadablePrefix::Blocked,
                message: "read-only relay".into(),
                status: false,
            }
        })
    }
}

pub(crate) async fn read_only_relay() -> (LocalRelay, String) {
    serve_relay(
        LocalRelayBuilder::default()
            .write_policy(RefusesWrites)
            .build(),
        &[],
    )
    .await
}

pub(crate) async fn events_on_relay(url: &str, filter: Filter) -> Vec<Event> {
    let client = Client::default();
    client.add_relay(url).await.unwrap();
    client.connect().await;
    let events = client
        .fetch_events(filter)
        .timeout(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    client.shutdown().await;
    events.into_iter().collect()
}

#[derive(Default)]
pub(crate) struct KuboApiState {
    pub(crate) mfs: BTreeMap<String, String>,
    pub(crate) add_cid: String,
    pub(crate) adds: Vec<String>,
    pub(crate) removed: Vec<String>,
    pub(crate) fail_list: bool,
    pub(crate) fail_remove: HashSet<String>,
    pub(crate) links: HashMap<String, Vec<(String, String, u8)>>,
    pub(crate) dag_size: u64,
    pub(crate) taken: HashSet<String>,
    pub(crate) calls: Vec<String>,
    pub(crate) unexpected: Vec<String>,
}

impl KuboApiState {
    pub(crate) fn paths(&self) -> Vec<String> {
        self.mfs.keys().cloned().collect()
    }
}

#[derive(Clone)]
pub(crate) struct KuboApi {
    pub(crate) url: String,
    pub(crate) s: std::sync::Arc<Mutex<KuboApiState>>,
}

fn kubo_args(query: Option<String>) -> Vec<String> {
    fn decode(raw: &str) -> String {
        let bytes = raw.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }
    query
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| pair.strip_prefix("arg="))
        .map(decode)
        .collect()
}

fn kubo_error(message: &str) -> (axum::http::StatusCode, String) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        serde_json::json!({ "Message": message, "Code": 0, "Type": "error" }).to_string(),
    )
}

fn kubo_ok(body: impl Into<String>) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::OK, body.into())
}

impl KuboApi {
    pub(crate) async fn serve(add_cid: &str) -> Self {
        use axum::extract::{RawQuery, State};
        use axum::routing::post;
        type Shared = State<std::sync::Arc<Mutex<KuboApiState>>>;

        let s = std::sync::Arc::new(Mutex::new(KuboApiState {
            add_cid: add_cid.to_string(),
            dag_size: 42,
            ..Default::default()
        }));
        let router = axum::Router::new()
            .route(
                "/api/v0/add",
                post(
                    |State(s): Shared, RawQuery(q): RawQuery, _body: axum::body::Bytes| async move {
                        let mut s = s.lock().unwrap();
                        s.adds.push(q.unwrap_or_default());
                        format!(r#"{{"Name":"x","Hash":"{}","Size":"3"}}"#, s.add_cid)
                    },
                ),
            )
            .route("/api/v0/files/mkdir", post(|| async { "" }))
            .route(
                "/api/v0/files/cp",
                post(|State(s): Shared, RawQuery(q): RawQuery| async move {
                    let args = kubo_args(q);
                    let cid = args[0].strip_prefix("/ipfs/").unwrap().to_string();
                    let mut s = s.lock().unwrap();
                    if s.mfs.contains_key(&args[1]) || s.taken.contains(&args[1]) {
                        return kubo_error(
                            "cp: cannot put node: directory already has entry by that name",
                        );
                    }
                    s.mfs.insert(args[1].clone(), cid);
                    kubo_ok("")
                }),
            )
            .route(
                "/api/v0/files/rm",
                post(|State(s): Shared, RawQuery(q): RawQuery| async move {
                    let path = kubo_args(q).remove(0);
                    let mut s = s.lock().unwrap();
                    if s.fail_remove.contains(&path) {
                        return kubo_ok("Error: simulated files/rm failure\n");
                    }
                    let prefix = format!("{path}/");
                    s.mfs.retain(|p, _| p != &path && !p.starts_with(&prefix));
                    s.removed.push(path);
                    kubo_ok("")
                }),
            )
            .route(
                "/api/v0/files/ls",
                post(|State(s): Shared, RawQuery(q): RawQuery| async move {
                    let path = kubo_args(q).remove(0);
                    let s = s.lock().unwrap();
                    if s.fail_list {
                        return kubo_error("simulated files/ls failure");
                    }
                    let prefix = format!("{path}/");
                    let mut entries = BTreeMap::new();
                    for (p, cid) in &s.mfs {
                        if let Some(rest) = p.strip_prefix(&prefix) {
                            let (name, kind, hash) = match rest.split_once('/') {
                                Some((name, _)) => (name, 1, String::new()),
                                None => (rest, 0, cid.clone()),
                            };
                            entries.insert(
                                name.to_string(),
                                serde_json::json!({ "Name": name, "Type": kind, "Size": 0, "Hash": hash }),
                            );
                        }
                    }
                    if entries.is_empty() {
                        return kubo_error("files/ls: file does not exist");
                    }
                    let entries: Vec<_> = entries.into_values().collect();
                    kubo_ok(serde_json::json!({ "Entries": entries }).to_string())
                }),
            )
            .route(
                "/api/v0/files/stat",
                post(|State(s): Shared, RawQuery(q): RawQuery| async move {
                    let path = kubo_args(q).remove(0);
                    if let Some(cid) = path.strip_prefix("/ipfs/") {
                        return kubo_ok(
                            serde_json::json!({ "Hash": cid, "Type": "directory" }).to_string(),
                        );
                    }
                    match s.lock().unwrap().mfs.get(&path) {
                        Some(cid) => kubo_ok(
                            serde_json::json!({ "Hash": cid, "Type": "directory" }).to_string(),
                        ),
                        None => kubo_error("files/stat: file does not exist"),
                    }
                }),
            )
            .route(
                "/api/v0/dag/stat",
                post(|State(s): Shared| async move {
                    let size = s.lock().unwrap().dag_size;
                    format!(r#"{{"TotalSize":{size}}}"#)
                }),
            )
            .route(
                "/api/v0/ls",
                post(|State(s): Shared, RawQuery(q): RawQuery| async move {
                    let cid = kubo_args(q).remove(0);
                    let s = s.lock().unwrap();
                    let links: Vec<_> = s
                        .links
                        .get(&cid)
                        .into_iter()
                        .flatten()
                        .map(|(name, hash, kind)| {
                            serde_json::json!({ "Name": name, "Hash": hash, "Type": kind })
                        })
                        .collect();
                    axum::Json(
                        serde_json::json!({ "Objects": [{ "Hash": cid, "Links": links }] }),
                    )
                }),
            )
            .fallback(|State(s): Shared, uri: axum::http::Uri| async move {
                s.lock().unwrap().unexpected.push(uri.path().to_string());
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            })
            .layer(axum::middleware::map_request_with_state(
                std::sync::Arc::clone(&s),
                |State(s): Shared, req: axum::extract::Request| async move {
                    let path = req.uri().path();
                    let call = path.strip_prefix("/api/v0/").unwrap_or(path).to_string();
                    s.lock().unwrap().calls.push(call);
                    req
                },
            ))
            .with_state(std::sync::Arc::clone(&s));
        let addr = serve_router(router).await;
        Self {
            url: format!("http://{addr}"),
            s,
        }
    }

    pub(crate) fn client(&self) -> crate::ipfs::IpfsClient {
        crate::ipfs::IpfsClient::new(self.url.clone())
    }

    pub(crate) fn paths(&self) -> Vec<String> {
        self.s.lock().unwrap().paths()
    }

    pub(crate) fn with(&self, f: impl FnOnce(&mut KuboApiState)) {
        f(&mut self.s.lock().unwrap());
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.s.lock().unwrap().calls.clone()
    }
}

pub(crate) async fn kubo_replying(
    replies: &[(&'static str, u16, &'static str)],
) -> crate::ipfs::IpfsClient {
    let mut router = axum::Router::new();
    for &(endpoint, status, body) in replies {
        router = router.route(
            &format!("/api/v0/{endpoint}"),
            axum::routing::post(move || async move {
                (axum::http::StatusCode::from_u16(status).unwrap(), body)
            }),
        );
    }
    let addr = serve_router(router).await;
    crate::ipfs::IpfsClient::new(format!("http://{addr}"))
}

pub(crate) struct RawServer {
    pub(crate) addr: std::net::SocketAddr,
    requests: std::sync::Arc<Mutex<Vec<String>>>,
}

impl RawServer {
    pub(crate) fn port(&self) -> u16 {
        self.addr.port()
    }

    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

pub(crate) fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub(crate) async fn serve_raw(response: impl Into<String>) -> RawServer {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let response: std::sync::Arc<str> = response.into().into();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
    let seen = std::sync::Arc::clone(&requests);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let seen = std::sync::Arc::clone(&seen);
            let response = std::sync::Arc::clone(&response);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase());
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    RawServer { addr, requests }
}

pub(crate) fn random_pubkey() -> PublicKey {
    Keys::generate().public_key()
}

pub(crate) fn remote_signer_file(
    app: &Keys,
    signer: PublicKey,
    relay: &str,
    user: PublicKey,
) -> crate::signer::RemoteSignerFile {
    crate::signer::RemoteSignerFile {
        app_secret_key: app.secret_key().to_secret_hex().into(),
        signer_pubkey: signer.to_hex(),
        relays: vec![relay.to_string()],
        user_pubkey: user.to_hex(),
    }
}

pub(crate) fn unanswered_signer(relay: &str, user: PublicKey) -> crate::signer::Signer {
    let file = remote_signer_file(&Keys::generate(), random_pubkey(), relay, user);
    let remote =
        crate::signer::RemoteSigner::from_file(&file, std::time::Duration::from_millis(500))
            .unwrap();
    crate::signer::Signer::Remote(std::sync::Arc::new(remote))
}
