use std::sync::Arc;

use axum::http::StatusCode;
use nostr_sdk::prelude::*;

use super::super::router;
use super::super::test_support::*;
use super::AppState;
use crate::mfs::MfsLayout;
use crate::mirror::MirrorSet;
use crate::signer::Signer;
use crate::state::{State, VersionRecord, site_key};
use crate::test_support::{
    CID_A, KuboApi, make_site_event, read_only_relay, relay_holding, replica_report_event,
};

const GATEWAY: &str = "http://gw.example/";
const OWN_CID: &str = "bafybeicwrird3ditgfyu4snq6i3x6i564gfcywffrwcgumnjhyiva3p2ra";

fn follow_set(author: &Keys, members: &[PublicKey], created_at: u64) -> Event {
    let mut set = MirrorSet::empty();
    set.add(members);
    set.build_event_builder("swing")
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(author)
        .unwrap()
}

struct World {
    _dir: tempfile::TempDir,
    _relay: LocalRelay,
    url: String,
    own: Keys,
    friend: Keys,
    kubo: KuboApi,
    state: Arc<AppState>,
    stored_at: u64,
    created_at: u64,
}

async fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let (own, friend, reporter) = (Keys::generate(), Keys::generate(), Keys::generate());
    let now = Timestamp::now().as_secs();
    let created_at = now - 600;
    let events = [
        follow_set(&own, &[friend.public_key()], now - 60),
        follow_set(&friend, &[own.public_key()], now - 60),
        make_site_event(&friend, 35980, "friend.example", CID_A, created_at),
        make_site_event(&own, 35980, "own.example", OWN_CID, now - 120),
        replica_report_event(
            &reporter,
            &friend.public_key(),
            "friend.example",
            &[CID_A],
            now - 30,
            now + 3600,
        ),
    ];
    let (relay, url) = relay_holding(&events).await;

    let kubo = KuboApi::serve(CID_A).await;
    let layout = MfsLayout::new("/swing");
    let friend_hex = friend.public_key().to_hex();
    let stored_at = now - 300;
    {
        let mut s = kubo.s.lock().unwrap();
        s.mfs.insert(
            layout.agent_version(&friend_hex, "friend.example", created_at),
            CID_A.into(),
        );
        s.links.insert(
            OWN_CID.into(),
            vec![
                ("index.html".into(), "f1".into(), 2),
                ("css".into(), "dir".into(), 1),
            ],
        );
        s.links
            .insert("dir".into(), vec![("site.css".into(), "f2".into(), 2)]);
    }

    let (mut config, _) = test_config(true);
    config.agent.state_dir = dir.path().join("data");
    config.nostr.relays = vec![url.clone()];
    config.dashboard.gateway = Some(GATEWAY.into());
    let mut saved = State::default();
    saved.apply_store(
        &site_key(&friend_hex, "friend.example"),
        VersionRecord {
            cid: CID_A.into(),
            size: 42,
            created_at,
            stored_at,
            event: None,
        },
    );
    saved
        .save(&config.agent.state_dir.join("state.json"))
        .await
        .unwrap();

    let state = ready_state(config, Signer::Local(own.clone()), &kubo).await;
    World {
        _dir: dir,
        _relay: relay,
        url,
        own,
        friend,
        kubo,
        state,
        stored_at,
        created_at,
    }
}

impl World {
    async fn get(&self, uri: &str) -> serde_json::Value {
        let (status, json) = send_json(router(Arc::clone(&self.state)), "GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {json}");
        json
    }

    async fn post(&self, uri: &str, keys: &[PublicKey]) -> (StatusCode, serde_json::Value) {
        let keys: Vec<String> = keys.iter().map(PublicKey::to_hex).collect();
        send_json(
            router(Arc::clone(&self.state)),
            "POST",
            uri,
            Some(serde_json::json!({ "keys": keys })),
        )
        .await
    }

    fn own_hex(&self) -> String {
        self.own.public_key().to_hex()
    }

    fn friend_hex(&self) -> String {
        self.friend.public_key().to_hex()
    }
}

#[tokio::test]
async fn sites_lists_the_followed_accounts_with_what_is_stored() {
    let w = world().await;
    let json = w.get("/api/sites").await;
    assert_eq!(json["follow_set"]["found"], true, "{json}");
    assert!(json["follow_set"]["note"].is_null());
    assert!(json["replicas_error"].is_null(), "{json}");
    let accounts = json["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1, "{json}");
    assert_eq!(accounts[0]["pubkey"], w.friend_hex());
    assert_eq!(
        accounts[0]["npub"],
        crate::mirror::npub(&w.friend.public_key())
    );
    let site = &accounts[0]["sites"][0];
    assert_eq!(site["d"], "friend.example");
    assert_eq!(site["cid"], CID_A);
    assert_eq!(site["url"], "https://example.com/");
    assert_eq!(site["created_at"], w.created_at);
    assert_eq!(site["stored"], true);
    assert_eq!(site["stored_at"], w.stored_at);
    assert_eq!(site["stored_size"], 42);
    assert_eq!(
        site["gateway_url"],
        format!("http://gw.example/ipfs/{CID_A}/")
    );
    assert_eq!(json["unfollowed"]["accounts"], serde_json::json!([]));
}

#[tokio::test]
async fn status_checks_each_stored_version_in_kubo() {
    let w = world().await;
    let json = w.get("/api/status").await;
    let versions = json["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 1, "{json}");
    assert_eq!(versions[0]["pubkey"], w.friend_hex());
    assert_eq!(versions[0]["d"], "friend.example");
    assert_eq!(versions[0]["cid"], CID_A);
    assert_eq!(versions[0]["health"], "ok", "{json}");
    assert_eq!(json["sites"][0]["actual"], 42);
    assert_eq!(json["problems"], 0, "{json}");
    assert_eq!(json["garbage"], serde_json::json!([]));
    assert!(w.kubo.s.lock().unwrap().unexpected.is_empty());
}

#[tokio::test]
async fn mirror_lists_the_members_of_the_own_follow_set() {
    let w = world().await;
    let json = w.get("/api/mirror").await;
    assert!(json["note"].is_null(), "{json}");
    let members = json["members"].as_array().unwrap();
    assert_eq!(members.len(), 1, "{json}");
    assert_eq!(members[0]["pubkey"], w.friend_hex());
}

#[tokio::test]
async fn mirror_add_and_remove_publish_the_new_set_and_wake_the_agent() {
    let w = world().await;
    let newcomer = Keys::generate().public_key();
    let woken = w.state.notify.notified();
    tokio::pin!(woken);
    woken.as_mut().enable();

    let (status, json) = w
        .post("/api/mirror/add", &[newcomer, w.friend.public_key()])
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["published"], true);
    assert_eq!(json["follow_set_found"], true);
    assert_eq!(json["changed"][0]["pubkey"], newcomer.to_hex());
    assert_eq!(json["unchanged"][0]["pubkey"], w.friend_hex());
    assert_eq!(json["members"].as_array().unwrap().len(), 2);
    assert_eq!(json["relays"][0]["relay"], w.url.as_str());
    assert_eq!(json["relays"][0]["ok"], true);
    tokio::time::timeout(std::time::Duration::from_secs(1), woken)
        .await
        .expect("a published change wakes the agent");

    let (status, json) = w.post("/api/mirror/remove", &[newcomer]).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["published"], true);
    assert_eq!(json["changed"][0]["pubkey"], newcomer.to_hex());
    let listed = w.get("/api/mirror").await;
    assert_eq!(listed["members"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["members"][0]["pubkey"], w.friend_hex());
}

#[tokio::test]
async fn a_mirror_change_no_relay_accepts_is_bad_gateway() {
    let dir = tempfile::tempdir().unwrap();
    let (_relay, url) = read_only_relay().await;
    let (mut config, secret_hex) = test_config(true);
    config.agent.state_dir = dir.path().join("data");
    config.nostr.relays = vec![url];
    let kubo = KuboApi::serve(CID_A).await;
    let state = ready_state(config, test_keys(&secret_hex).unwrap(), &kubo).await;
    let newcomer = Keys::generate().public_key().to_hex();
    let (status, json) = send_json(
        router(state),
        "POST",
        "/api/mirror/add",
        Some(serde_json::json!({ "keys": [newcomer] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{json}");
    assert_eq!(json["error"], "no relay accepted the mirror set update");
}

#[tokio::test]
async fn webring_draws_the_mutual_follow_from_the_own_account() {
    let w = world().await;
    let json = w.get("/api/webring?depth=1").await;
    assert_eq!(json["depth"], 1);
    let nodes = json["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2, "{json}");
    assert_eq!(nodes[0]["pubkey"], w.own_hex());
    assert_eq!(nodes[0]["root"], true);
    assert_eq!(nodes[1]["pubkey"], w.friend_hex());
    assert_eq!(nodes[1]["depth"], 1);
    assert_eq!(nodes[1]["names"], serde_json::json!(["friend.example"]));
    let edges = json["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1, "{json}");
    assert_eq!(edges[0]["mutual"], true);
    assert!(json["text"].as_str().unwrap().contains("friend.example"));
}

#[tokio::test]
async fn replicas_counts_the_reports_for_the_requested_account() {
    let w = world().await;
    let json = w
        .get(&format!("/api/replicas?key={}", w.friend_hex()))
        .await;
    let authors = json["authors"].as_array().unwrap();
    assert_eq!(authors.len(), 1, "{json}");
    assert_eq!(authors[0]["pubkey"], w.friend_hex());
    let site = &authors[0]["sites"][0];
    assert_eq!(site["d"], "friend.example");
    assert_eq!(site["cid"], CID_A);
    assert_eq!(site["reports"], 1, "{json}");
    assert_eq!(site["reporters"].as_array().unwrap().len(), 1, "{json}");
}

#[tokio::test]
async fn publish_sites_lists_the_own_sites() {
    let w = world().await;
    let json = w.get("/api/publish/sites").await;
    let sites = json["sites"].as_array().unwrap();
    assert_eq!(sites.len(), 1, "{json}");
    assert_eq!(sites[0]["d"], "own.example");
    assert_eq!(sites[0]["cid"], OWN_CID);
}

#[tokio::test]
async fn previous_files_lists_the_files_of_the_last_published_version() {
    let w = world().await;
    let json = w.get("/api/publish/previous-files?site=own.example").await;
    assert_eq!(json["status"], "listed", "{json}");
    assert_eq!(json["previous_cid"], OWN_CID);
    assert_eq!(
        json["files"],
        serde_json::json!(["css/site.css", "index.html"]),
        "{json}"
    );

    let json = w
        .get("/api/publish/previous-files?site=never.example")
        .await;
    assert_eq!(json["status"], "no_previous", "{json}");
    assert!(json["previous_cid"].is_null(), "{json}");
}

#[tokio::test]
async fn overview_carries_what_the_tray_reads() {
    let (_relay, url) = relay_holding(&[]).await;
    let user = Keys::generate();
    let signer = crate::test_support::unanswered_signer(&url, user.public_key());
    assert!(
        signer
            .sign(EventBuilder::new(Kind::TextNote, "x"))
            .await
            .is_err()
    );
    let (config, _) = test_config(true);
    let state = build_state(config, test_exit(), Some(signer), TEST_TOKEN);

    let (status, json) = send_json(router(Arc::clone(&state)), "GET", "/api/overview", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["setup"], false);
    assert_eq!(json["instance"], state.instance.as_str());
    assert!(!state.instance.is_empty());
    assert_eq!(json["pubkey"], user.public_key().to_hex());
    assert_eq!(json["signer"]["remote"], true);
    assert_eq!(json["signer"]["relays"], serde_json::json!([url]));
    let failure = json
        .pointer("/signer/last_failure")
        .expect("the tray reads /signer/last_failure");
    assert!(failure["message"].is_string(), "{json}");
    assert!(failure["at"].is_u64(), "{json}");
}
