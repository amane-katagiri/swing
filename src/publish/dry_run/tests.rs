use std::sync::{Arc, Mutex};

use axum::extract::RawQuery;
use axum::http::{StatusCode, Uri};
use axum::routing::post;
use nostr_sdk::prelude::*;

use super::*;
use crate::config;
use crate::publish::ModeOverrides;
use crate::test_support::CID_A;

const HASHED: &str = "bafybeicwrird3ditgfyu4snq6i3x6i564gfcywffrwcgumnjhyiva3p2ra";

type Calls = Arc<Mutex<Vec<String>>>;

struct FakeKubo {
    api: String,
    calls: Calls,
}

async fn fake_kubo(newest_version: Option<u64>) -> FakeKubo {
    let calls = Calls::default();
    let (add_log, ls_log, files_log, other_log) =
        (calls.clone(), calls.clone(), calls.clone(), calls.clone());
    let router = axum::Router::new()
        .route(
            "/api/v0/add",
            post(
                move |RawQuery(q): RawQuery, _body: axum::body::Bytes| async move {
                    add_log
                        .lock()
                        .unwrap()
                        .push(format!("add?{}", q.unwrap_or_default()));
                    format!(r#"{{"Name":"x","Hash":"{HASHED}","Size":"3"}}"#)
                },
            ),
        )
        .route(
            "/api/v0/files/ls",
            post(move || async move {
                files_log.lock().unwrap().push("files/ls".to_string());
                match newest_version {
                    Some(v) => (
                        StatusCode::OK,
                        format!(r#"{{"Entries":[{{"Name":"{v}","Type":1,"Hash":"bafy"}}]}}"#),
                    ),
                    None => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        r#"{"Message":"file does not exist","Code":0,"Type":"error"}"#.to_string(),
                    ),
                }
            }),
        )
        .route(
            "/api/v0/ls",
            post(move || async move {
                ls_log.lock().unwrap().push("ls".to_string());
                axum::Json(serde_json::json!({ "Objects": [{ "Hash": CID_A, "Links": [
                    { "Name": "index.html", "Hash": "f1", "Type": 2 },
                ] }] }))
            }),
        )
        .fallback(move |uri: Uri| async move {
            other_log.lock().unwrap().push(uri.path().to_string());
            StatusCode::INTERNAL_SERVER_ERROR
        });
    let addr = crate::test_support::serve_router(router).await;
    FakeKubo {
        api: format!("http://{addr}"),
        calls,
    }
}

fn site_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, body) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    dir
}

fn test_config(state: &Path, keys: Option<&Keys>, kubo: Option<&str>, relays: &[String]) -> Config {
    let mut toml = String::from("[nostr]\n");
    if let Some(keys) = keys {
        toml.push_str(&format!(
            "secret_key = \"{}\"\n",
            keys.secret_key().to_secret_hex()
        ));
    }
    if let Some(api) = kubo {
        toml.push_str(&format!(
            "[ipfs]\napi = \"{api}\"\n[kubo]\nmanaged = false\n"
        ));
    }
    let mut config = config::build_config_from_str(&toml, |_| None).unwrap();
    config.agent.state_dir = state.to_path_buf();
    config.kubo.repo = state.join("kubo");
    config.config_path = state.join("swing.toml");
    config.nostr.relays = relays.to_vec();
    config
}

fn request(modes: ModeOverrides) -> Request {
    Request {
        d: "example.com".into(),
        url: None,
        title: None,
        message: None,
        note: false,
        modes,
        yes: false,
    }
}

fn modes(unchanged: &str) -> ModeOverrides {
    ModeOverrides {
        nip05: Some("off".into()),
        check_dotfiles: None,
        check_size: None,
        check_links: None,
        check_unchanged: Some(unchanged.into()),
    }
}

async fn relay_with_previous(keys: &Keys, cid: &str) -> (LocalRelay, String) {
    let local = LocalRelay::new();
    local.run().await.unwrap();
    let url = local.url().await.to_string();
    let event = crate::nostr::build_site_event_builder(
        35980,
        &crate::nostr::SiteFields {
            d: "example.com",
            cid,
            url: None,
            size: Some(10),
            title: None,
            message: None,
        },
    )
    .custom_created_at(Timestamp::from_secs(Timestamp::now().as_secs() - 100))
    .finalize(keys)
    .unwrap();
    let client = Client::default();
    client.add_relay(url.as_str()).await.unwrap();
    client.connect().await;
    client.send_event(&event).await.unwrap();
    client.shutdown().await;
    (local, url)
}

async fn events_on(url: &str, author: PublicKey) -> usize {
    let client = Client::default();
    client.add_relay(url).await.unwrap();
    client.connect().await;
    let events = client
        .fetch_events(Filter::new().author(author))
        .timeout(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    client.shutdown().await;
    events.len()
}

struct NeverVerified;

impl Nip05Verify for NeverVerified {
    async fn verify(&self, _d: &str, _pubkey_hex: &str) -> crate::nip05::VerificationResult {
        crate::nip05::VerificationResult::Mismatch
    }
}

#[tokio::test]
async fn a_dry_run_only_hashes_and_reads_and_never_signs_or_sends() {
    let keys = Keys::generate();
    let (_relay, url) = relay_with_previous(&keys, CID_A).await;
    let kubo = fake_kubo(None).await;
    let state = tempfile::tempdir().unwrap();
    let site = site_dir(&[("index.html", "hi"), ("blog/new.html", "new")]);
    let config = test_config(
        state.path(),
        Some(&keys),
        Some(&kubo.api),
        std::slice::from_ref(&url),
    );
    let request = request(modes("require"));
    let evaluation = evaluate(&config, site.path(), &request, &NeverVerified)
        .await
        .unwrap();

    assert_eq!(evaluation.cid.as_deref(), Ok(HASHED));
    assert_eq!(evaluation.outcome(), Outcome::Publish);
    assert_eq!(evaluation.new_files, vec!["blog/new.html"]);
    assert_eq!(evaluation.unchanged.status, UnchangedStatus::Changed);
    let calls = kubo.calls.lock().unwrap().clone();
    let adds: Vec<&String> = calls.iter().filter(|c| c.starts_with("add?")).collect();
    assert_eq!(adds.len(), 1, "{calls:?}");
    assert!(adds[0].contains("only-hash=true"), "{calls:?}");
    assert!(adds[0].contains("pin=false"), "{calls:?}");
    assert!(
        calls
            .iter()
            .all(|c| c.starts_with("add?") || c == "files/ls" || c == "ls"),
        "{calls:?}"
    );
    assert_eq!(events_on(&url, keys.public_key()).await, 1);
}

#[tokio::test]
async fn the_same_cid_under_require_is_reported_as_unchanged_without_stopping() {
    let keys = Keys::generate();
    let (_relay, url) = relay_with_previous(&keys, HASHED).await;
    let kubo = fake_kubo(None).await;
    let state = tempfile::tempdir().unwrap();
    let site = site_dir(&[("index.html", "hi")]);
    let config = test_config(
        state.path(),
        Some(&keys),
        Some(&kubo.api),
        std::slice::from_ref(&url),
    );
    let request = request(modes("require"));
    let evaluation = evaluate(&config, site.path(), &request, &NeverVerified)
        .await
        .unwrap();
    assert_eq!(evaluation.outcome(), Outcome::Unchanged);
    assert!(evaluation.blockers().is_empty());
    let report = serde_json::to_value(evaluation.report()).unwrap();
    assert_eq!(report["outcome"], "unchanged");
    assert_eq!(report["unchanged"]["status"], "unchanged");
    assert_eq!(report["unchanged"]["skips_publish"], true);
    assert_eq!(report["previous"]["status"], "found");
    assert_eq!(report["previous"]["cid"], HASHED);
}

#[tokio::test]
async fn every_stop_condition_is_collected_and_fails_the_run() {
    let keys = Keys::generate();
    let future = Timestamp::now().as_secs() + crate::nostr::MAX_FUTURE_SKEW + 3600;
    let kubo = fake_kubo(Some(future)).await;
    let state = tempfile::tempdir().unwrap();
    let site = site_dir(&[
        ("index.html", r#"<a href="/about/">a</a>"#),
        (".env", "SECRET=1"),
    ]);
    let config = test_config(state.path(), Some(&keys), Some(&kubo.api), &[]);
    let mut overrides = modes("require");
    overrides.nip05 = Some("require".into());
    overrides.check_links = Some("require".into());
    let request = request(overrides);
    let evaluation = evaluate(&config, site.path(), &request, &NeverVerified)
        .await
        .unwrap();
    let checks: Vec<&str> = evaluation.blockers().iter().map(|b| b.check).collect();
    assert_eq!(checks, ["nip05", "dotfiles", "links", "clock"]);
    assert_eq!(evaluation.outcome(), Outcome::Blocked);

    let report = serde_json::to_value(evaluation.report()).unwrap();
    assert_eq!(report["outcome"], "blocked");
    assert_eq!(report["nip05"]["status"], "mismatch");
    assert_eq!(report["nip05"]["stops"], true);
    assert_eq!(report["checks"]["dotfiles"]["paths"][0], ".env");
    assert_eq!(report["checks"]["size"]["stops"], false);
    let links = &report["checks"]["links"];
    assert_eq!(links["stops"], true);
    let finding = links["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "root_relative")
        .unwrap();
    assert_eq!(finding["breaks"], true);
    assert_eq!(finding["file"], "index.html");
    assert_eq!(finding["reference"], "/about/");
    assert_eq!(report["clock"]["status"], "stops");
    assert_eq!(report["clock"]["mfs_versions"]["newest"], future);

    let mut without_nip05 = modes("require");
    without_nip05.check_links = Some("require".into());
    let err = run(
        config.clone(),
        site.path(),
        self::request(without_nip05),
        true,
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().starts_with("the publish would stop: "),
        "{err}"
    );
    let calls = kubo.calls.lock().unwrap().clone();
    assert!(
        calls
            .iter()
            .all(|c| c.starts_with("add?") || c == "files/ls"),
        "{calls:?}"
    );
}

#[tokio::test]
async fn without_a_key_or_kubo_the_local_checks_still_run_and_the_rest_is_unknown() {
    let state = tempfile::tempdir().unwrap();
    let site = site_dir(&[("index.html", "hi"), (".env", "x")]);
    let config = test_config(state.path(), None, None, &[]);
    let mut overrides = modes("require");
    overrides.nip05 = Some("require".into());
    overrides.check_dotfiles = Some("warn".into());
    let request = request(overrides);
    let evaluation = evaluate(&config, site.path(), &request, &NeverVerified)
        .await
        .unwrap();
    assert_eq!(evaluation.outcome(), Outcome::Publish);

    let report = serde_json::to_value(evaluation.report()).unwrap();
    assert_eq!(report["format"], FORMAT);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["site"], "example.com");
    assert_eq!(report["url"], serde_json::Value::Null);
    assert_eq!(report["pubkey"]["status"], "unknown");
    assert!(
        report["pubkey"]["detail"]
            .as_str()
            .unwrap()
            .starts_with("missing Nostr key")
    );
    assert_eq!(report["nip05"]["status"], "unknown");
    assert_eq!(report["nip05"]["stops"], false);
    assert_eq!(report["checks"]["dotfiles"]["status"], "found");
    assert_eq!(report["checks"]["dotfiles"]["stops"], false);
    assert_eq!(report["checks"]["size"]["files"], 2);
    assert_eq!(report["checks"]["size"]["bytes"], 3);
    assert_eq!(report["ipfs"]["status"], "unknown");
    assert!(
        report["ipfs"]["detail"]
            .as_str()
            .unwrap()
            .contains("Kubo is not running")
    );
    assert_eq!(report["previous"]["status"], "unknown");
    assert_eq!(report["new_files"]["status"], "unknown");
    assert_eq!(report["new_files"]["count"], 2);
    assert_eq!(report["unchanged"]["status"], "unknown");
    assert_eq!(report["clock"]["status"], "ok");
    assert_eq!(report["clock"]["mfs_versions"]["status"], "unknown");
    assert_eq!(report["clock"]["relays_answered"], 0);
    assert_eq!(report["outcome"], "publish");
    assert_eq!(report["blockers"], serde_json::json!([]));
    assert_eq!(report["guide_url"], SITE_GUIDE_URL);
}

#[tokio::test]
async fn the_unchanged_check_is_reported_even_when_off_but_never_skips() {
    let keys = Keys::generate();
    let (_relay, url) = relay_with_previous(&keys, HASHED).await;
    let kubo = fake_kubo(None).await;
    let state = tempfile::tempdir().unwrap();
    let site = site_dir(&[("index.html", "hi")]);
    let config = test_config(
        state.path(),
        Some(&keys),
        Some(&kubo.api),
        std::slice::from_ref(&url),
    );
    let request = request(modes("off"));
    let evaluation = evaluate(&config, site.path(), &request, &NeverVerified)
        .await
        .unwrap();
    assert_eq!(evaluation.unchanged.status, UnchangedStatus::Unchanged);
    assert_eq!(evaluation.outcome(), Outcome::Publish);
    let report = serde_json::to_value(evaluation.report()).unwrap();
    assert_eq!(report["unchanged"]["mode"], "off");
    assert_eq!(report["unchanged"]["skips_publish"], false);
}

#[tokio::test]
async fn argument_errors_and_protected_paths_fail_before_the_evaluation() {
    let state = tempfile::tempdir().unwrap();
    let site = site_dir(&[("index.html", "hi")]);
    let config = test_config(state.path(), None, None, &[]);
    let mut bad = request(modes("require"));
    bad.d = "bad\nsite".into();
    let err = evaluate(&config, site.path(), &bad, &NeverVerified)
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("invalid --site"), "{err}");

    let mut config = config;
    config.agent.state_dir = site.path().join("data");
    std::fs::create_dir(&config.agent.state_dir).unwrap();
    let request = request(modes("require"));
    let err = evaluate(&config, site.path(), &request, &NeverVerified)
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("[agent].state_dir"), "{err}");
}
