use std::sync::Arc;

use axum::http::StatusCode;
use nostr_sdk::prelude::*;

use super::super::router;
use super::super::test_support::*;
use super::AppState;
use crate::signer::Signer;
use crate::test_support::{
    CID_B, KuboApi, events_on_relay, make_site_event, read_only_relay, relay_holding,
    unanswered_signer,
};

const ADDED: &str = "bafybeicwrird3ditgfyu4snq6i3x6i564gfcywffrwcgumnjhyiva3p2ra";
const SITE: &str = "example.com";

struct Rig {
    _dir: tempfile::TempDir,
    _relay: LocalRelay,
    url: String,
    kubo: KuboApi,
    keys: Keys,
    state: Arc<AppState>,
    site_path: String,
}

#[derive(Default)]
struct Setup {
    read_only: bool,
    unanswered: bool,
    previous_cid: Option<&'static str>,
    old_versions: Vec<u64>,
    repo_around_state: bool,
}

async fn rig(setup: Setup) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let keys = Keys::generate();
    let previous: Vec<Event> = setup
        .previous_cid
        .map(|cid| make_site_event(&keys, 35980, SITE, cid, Timestamp::now().as_secs() - 100))
        .into_iter()
        .collect();
    let (relay, url) = if setup.read_only {
        read_only_relay().await
    } else {
        relay_holding(&previous).await
    };
    let kubo = KuboApi::serve(ADDED).await;
    let site_path =
        crate::mfs::MfsLayout::new("/swing").publish_site(&keys.public_key().to_hex(), SITE);
    for t in &setup.old_versions {
        kubo.s
            .lock()
            .unwrap()
            .mfs
            .insert(format!("{site_path}/{t}"), CID_B.into());
    }
    let (mut config, _) = test_config(true);
    config.agent.state_dir = dir.path().join("data");
    if setup.repo_around_state {
        config.kubo.repo = dir.path().to_path_buf();
    }
    config.nostr.relays = vec![url.clone()];
    let signer = if setup.unanswered {
        unanswered_signer(&url, keys.public_key())
    } else {
        Signer::Local(keys.clone())
    };
    let state = ready_state(config, signer, &kubo).await;
    Rig {
        _dir: dir,
        _relay: relay,
        url,
        kubo,
        keys,
        state,
        site_path,
    }
}

impl Rig {
    async fn upload(&self) -> (StatusCode, serde_json::Value) {
        let boundary = "swing-test-boundary";
        let body = multipart_body(
            boundary,
            &[
                ("site", None, SITE.as_bytes()),
                ("nip05", None, b"off"),
                ("check_links", None, b"off"),
                ("file", Some("index.html"), b"<p>hi</p>"),
                ("file", Some("css/site.css"), b"p{}"),
            ],
        );
        let resp = call(
            router(Arc::clone(&self.state)),
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        let status = resp.status();
        (status, error_body(resp).await)
    }

    async fn site_events(&self) -> Vec<Event> {
        events_on_relay(
            &self.url,
            Filter::new()
                .author(self.keys.public_key())
                .kind(Kind::Custom(35980)),
        )
        .await
    }

    fn uploads_left(&self) -> usize {
        let root = self
            .state
            .config
            .agent
            .state_dir
            .join(crate::publish::DASHBOARD_UPLOAD_DIR);
        std::fs::read_dir(root).map_or(0, |d| d.count())
    }

    fn removed(&self) -> Vec<String> {
        self.kubo.s.lock().unwrap().removed.clone()
    }

    fn adds(&self) -> usize {
        self.kubo.s.lock().unwrap().adds.len()
    }
}

#[tokio::test]
async fn an_uploaded_site_is_published_and_the_old_version_pruned() {
    let old = Timestamp::now().as_secs() - 100;
    let rig = rig(Setup {
        old_versions: vec![old],
        ..Default::default()
    })
    .await;

    let (status, json) = rig.upload().await;

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["published"], true);
    assert_eq!(json["files"], 2);
    assert_eq!(json["site"], SITE);
    assert_eq!(json["cid"], ADDED);
    assert_eq!(json["size"], 42);
    let created_at = json["created_at"].as_u64().unwrap();
    let current = format!("{}/{created_at}", rig.site_path);
    assert_eq!(json["mfs_path"], current.as_str());
    assert_eq!(json["relays"][0]["relay"], rig.url.as_str());
    assert_eq!(json["relays"][0]["ok"], true);
    assert_eq!(json["pruned"], serde_json::json!([old.to_string()]));
    assert!(json["prune_error"].is_null());
    assert_eq!(json["checks"]["unchanged"]["status"], "no_previous");

    assert_eq!(rig.kubo.paths(), vec![current]);
    assert_eq!(rig.uploads_left(), 0);
    let events = rig.site_events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].created_at.as_secs(), created_at);
    assert_eq!(rig.state.activity.latest_published_at(), Some(created_at));
}

#[tokio::test]
async fn an_unchanged_site_withdraws_the_staged_version_without_announcing() {
    let rig = rig(Setup {
        previous_cid: Some(ADDED),
        ..Default::default()
    })
    .await;

    let (status, json) = rig.upload().await;

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["published"], false);
    assert!(json["mfs_path"].is_null());
    assert!(json["created_at"].is_null());
    assert_eq!(json["checks"]["unchanged"]["status"], "unchanged");
    assert_eq!(rig.removed().len(), 1);
    assert!(rig.kubo.paths().is_empty());
    assert_eq!(rig.site_events().await.len(), 1);
    assert_eq!(rig.state.activity.latest_published_at(), None);
    assert_eq!(rig.uploads_left(), 0);
}

#[tokio::test]
async fn an_unsigned_site_event_withdraws_the_staged_version() {
    let rig = rig(Setup {
        unanswered: true,
        ..Default::default()
    })
    .await;

    let (status, json) = rig.upload().await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{json}");
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("signing site event"),
        "{json}"
    );
    assert_eq!(rig.removed().len(), 1);
    assert!(rig.kubo.paths().is_empty());
    assert!(rig.site_events().await.is_empty());
    assert_eq!(rig.uploads_left(), 0);
}

#[tokio::test]
async fn when_no_relay_accepts_the_event_nothing_is_pruned_or_recorded() {
    let old = Timestamp::now().as_secs() - 100;
    let rig = rig(Setup {
        read_only: true,
        old_versions: vec![old],
        ..Default::default()
    })
    .await;

    let (status, json) = rig.upload().await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{json}");
    let error = json["error"].as_str().unwrap();
    assert!(
        error.starts_with(crate::publish::NO_RELAY_ACCEPTED),
        "{error}"
    );
    assert!(error.contains("read-only relay"), "{error}");
    assert!(rig.removed().is_empty());
    assert_eq!(rig.kubo.paths().len(), 2);
    assert_eq!(rig.state.activity.latest_published_at(), None);
    assert_eq!(rig.uploads_left(), 0);
}

#[tokio::test]
async fn an_upload_inside_the_kubo_repo_is_refused_before_adding() {
    let rig = rig(Setup {
        repo_around_state: true,
        ..Default::default()
    })
    .await;

    let (status, json) = rig.upload().await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    assert!(
        json["error"].as_str().unwrap().contains("[kubo].repo"),
        "{json}"
    );
    assert_eq!(rig.adds(), 0);
    assert_eq!(rig.uploads_left(), 0);
}

#[tokio::test]
async fn a_newest_version_from_the_far_future_is_bad_request_before_adding() {
    let far = Timestamp::now().as_secs() + 30 * 86_400;
    let rig = rig(Setup {
        old_versions: vec![far],
        ..Default::default()
    })
    .await;

    let (status, json) = rig.upload().await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    assert!(json["error"].is_string());
    assert_eq!(rig.adds(), 0);
    assert!(rig.site_events().await.is_empty());
}
