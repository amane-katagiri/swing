mod ready;

use super::super::router;
use super::super::test_support::*;
use super::AppState;
use axum::body::Body;
use axum::http::StatusCode;
use nostr_sdk::prelude::Keys;
use std::sync::Arc;

use crate::shutdown::ExitRequest;

#[tokio::test]
async fn config_endpoint_never_exposes_the_secret_key_value() {
    let (config, secret_hex) = test_config(true);
    let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
    let app = router(state);
    let req = get("/api/config");
    let resp = call(app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains(&secret_hex));
    assert!(text.contains("\"(set, hidden)\""));
}

#[tokio::test]
async fn relay_queries_are_refused_once_every_permit_stays_taken() {
    use axum::response::IntoResponse;
    use std::time::Duration;
    let semaphore = tokio::sync::Semaphore::new(1);
    let held = super::acquire_within(&semaphore, Duration::ZERO)
        .await
        .ok()
        .unwrap();
    let refused = super::acquire_within(&semaphore, Duration::from_millis(10)).await;
    let Err(err) = refused else {
        panic!("expected Busy");
    };
    assert_eq!(
        err.into_response().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(held);
    assert!(
        super::acquire_within(&semaphore, Duration::ZERO)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn requests_over_a_count_limit_are_bad_request() {
    let keys = |n: usize| -> Vec<String> {
        (0..n)
            .map(|_| Keys::generate().public_key().to_hex())
            .collect()
    };
    let query = |name: &str| {
        (0..101)
            .map(|i| format!("{name}=key-{i}"))
            .collect::<Vec<_>>()
            .join("&")
    };
    let cases = [
        (
            "webring depth",
            "GET",
            "/api/webring?depth=5".to_string(),
            None,
            "depth must be between 0 and",
        ),
        (
            "webring roots",
            "GET",
            format!("/api/webring?{}", query("root")),
            None,
            "root must include at most 100 entries",
        ),
        (
            "replica keys",
            "GET",
            format!("/api/replicas?{}", query("key")),
            None,
            "key must include at most 100 entries",
        ),
        (
            "mirror add keys",
            "POST",
            "/api/mirror/add".to_string(),
            Some(serde_json::json!({ "keys": keys(101) })),
            "at most 100 entries",
        ),
        (
            "mirror preview keys",
            "POST",
            "/api/mirror/preview".to_string(),
            Some(serde_json::json!({ "keys": keys(101) })),
            "at most 100 entries",
        ),
        (
            "empty mirror preview",
            "POST",
            "/api/mirror/preview".to_string(),
            Some(serde_json::json!({ "keys": [] })),
            "at least one entry",
        ),
    ];
    for (case, method, uri, body, error) in cases {
        let (status, body) = send_json(router(test_state()), method, &uri, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}");
        assert!(
            body["error"].as_str().unwrap().contains(error),
            "{case}: {body}"
        );
    }
}

#[tokio::test]
async fn endpoints_without_their_backend_are_service_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let (config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
    let setup_mode = build_state(config, test_exit(), None, TEST_TOKEN);
    for (case, uri, state, error) in [
        (
            "sites before the relay",
            "/api/sites",
            test_state(),
            "agent is not ready",
        ),
        (
            "publish sites before the relay",
            "/api/publish/sites",
            test_state(),
            "agent is not ready",
        ),
        (
            "status before ipfs",
            "/api/status",
            test_state(),
            "agent is not ready",
        ),
        (
            "sites in setup mode",
            "/api/sites",
            setup_mode,
            "agent is not configured",
        ),
    ] {
        let (status, body) = send_json(router(state), "GET", uri, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{case}");
        assert_eq!(body["error"], error, "{case}");
    }
}

#[tokio::test]
async fn previous_files_checks_the_site_before_the_relay() {
    for (uri, status) in [
        ("/api/publish/previous-files", StatusCode::BAD_REQUEST),
        (
            "/api/publish/previous-files?site=a%0Ab",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/publish/previous-files?site=example.com",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    ] {
        let req = get(uri);
        let resp = call(router(test_state()), req).await;
        assert_eq!(resp.status(), status, "{uri}");
    }
}

async fn ready_state_with_relays(
    state_dir: &std::path::Path,
    relays: &[String],
) -> (Arc<AppState>, Keys) {
    let (mut config, secret_hex) = test_config(true);
    config.agent.state_dir = state_dir.to_path_buf();
    let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
    let relay = crate::nostr::RelayClient::connect(test_keys(&secret_hex).unwrap(), relays)
        .await
        .unwrap();
    let ipfs = crate::ipfs::IpfsClient::new("http://127.0.0.1:1".to_string());
    state.set_ready(Arc::new(relay), ipfs).await;
    (state, Keys::parse(&secret_hex).unwrap())
}

#[tokio::test]
async fn mirror_add_past_the_follow_set_cap_is_conflict_not_bad_gateway() {
    let (_local, url) = crate::test_support::relay_holding(&[]).await;
    let dir = tempfile::tempdir().unwrap();
    let (state, keys) = ready_state_with_relays(dir.path(), std::slice::from_ref(&url)).await;

    let full = crate::test_support::full_follow_set(&keys, &state.config.nostr.mirror_set);
    crate::test_support::seed_relay(&url, &[full]).await;

    let extra = Keys::generate().public_key().to_hex();
    let (status, body) = send_json(
        router(state),
        "POST",
        "/api/mirror/add",
        Some(serde_json::json!({ "keys": [extra] })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("-entry limit"), "{message}");
}

#[tokio::test]
async fn mirror_add_relay_failure_stays_bad_gateway() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _) = ready_state_with_relays(dir.path(), &[]).await;
    let extra = Keys::generate().public_key().to_hex();
    let (status, body) = send_json(
        router(state),
        "POST",
        "/api/mirror/add",
        Some(serde_json::json!({ "keys": [extra] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn mirror_add_when_no_relay_answers_is_bad_gateway() {
    let (_refusing, url) = crate::test_support::refusing_relay(None).await;
    let dir = tempfile::tempdir().unwrap();
    let (state, _) = ready_state_with_relays(dir.path(), std::slice::from_ref(&url)).await;
    let extra = Keys::generate().public_key().to_hex();
    let (status, body) = send_json(
        router(state),
        "POST",
        "/api/mirror/add",
        Some(serde_json::json!({ "keys": [extra] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("no relay answered"), "{message}");
}

fn test_state_with_exit(exit: ExitRequest) -> Arc<AppState> {
    let (config, secret_hex) = test_config(true);
    build_state(config, exit, test_keys(&secret_hex), TEST_TOKEN)
}

#[tokio::test]
async fn shutdown_without_marker_header_is_forbidden() {
    let token = tokio_util::sync::CancellationToken::new();
    let app = router(test_state_with_exit(ExitRequest::new(token.clone())));
    let req = request("POST", "/api/shutdown")
        .body(Body::empty())
        .unwrap();
    let resp = call(app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(!token.is_cancelled());
}

#[tokio::test]
async fn shutdown_with_marker_header_is_accepted_and_cancels_the_token() {
    let token = tokio_util::sync::CancellationToken::new();
    let app = router(test_state_with_exit(ExitRequest::new(token.clone())));
    let req = request("POST", "/api/shutdown")
        .header("x-swing-dashboard", "1")
        .body(Body::empty())
        .unwrap();
    let resp = call(app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let json = json_body(resp).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["action"], "stop");
    assert!(token.is_cancelled());
}

#[tokio::test]
async fn restart_with_marker_header_is_accepted_sets_the_flag_and_cancels() {
    let token = tokio_util::sync::CancellationToken::new();
    let exit = ExitRequest::new(token.clone());
    let app = router(test_state_with_exit(exit.clone()));
    let req = request("POST", "/api/restart")
        .header("x-swing-dashboard", "1")
        .body(Body::empty())
        .unwrap();
    let resp = call(app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let json = json_body(resp).await;
    assert_eq!(json["action"], "restart");
    assert!(token.is_cancelled());
    assert!(exit.restart_requested());
}

#[tokio::test]
async fn put_config_updates_the_file_and_marks_restart_required() {
    let dir = tempfile::tempdir().unwrap();
    let (config, secret_hex) = test_config_in_dir(dir.path(), true, true);
    let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
    let app = router(state);
    let body = serde_json::json!({
        "items": { "policy.max_total_storage": "20GB" }
    })
    .to_string();
    let req = request("PUT", "/api/config")
        .header("x-swing-dashboard", "1")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = call(app.clone(), req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["restart_required"], true);
    assert!(dir.path().join("swing.toml").exists());
    let policy = json["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "policy")
        .unwrap();
    let item = policy["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "max_total_storage")
        .unwrap();
    assert_eq!(item["source"], "file");
    assert_eq!(item["raw"], "20 GiB");

    let get_req = get("/api/config");
    let get_resp = call(app, get_req).await;
    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_json = json_body(get_resp).await;
    assert_eq!(get_json["restart_required"], true);
    let policy = get_json["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "policy")
        .unwrap();
    let item = policy["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "max_total_storage")
        .unwrap();
    assert_eq!(item["source"], "file");
}

fn put_config_body(key: &str, value: &str) -> serde_json::Value {
    serde_json::json!({ "items": { key: value } })
}

#[tokio::test]
async fn put_config_twice_without_a_config_file_keeps_both_items() {
    let dir = tempfile::tempdir().unwrap();
    let (config, secret_hex) = test_config_in_dir(dir.path(), true, true);
    let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
    let app = router(state);
    let first = put_config_body("policy.max_total_storage", "20GB");
    let (status, _) = send_json(app.clone(), "PUT", "/api/config", Some(first)).await;
    assert_eq!(status, StatusCode::OK);
    let second = put_config_body("nostr.mirror_set", "second-set");
    let (status, _) = send_json(app, "PUT", "/api/config", Some(second)).await;
    assert_eq!(status, StatusCode::OK);

    let path = dir.path().join("swing.toml");
    let saved = crate::config::Config::load(Some(&path)).unwrap();
    assert_eq!(saved.policy.max_total_storage, 20 * (1u64 << 30));
    assert_eq!(saved.nostr.mirror_set, "second-set");
}

#[tokio::test]
async fn an_invalid_config_value_is_bad_request() {
    let dir = tempfile::tempdir().unwrap();
    let (config, secret_hex) = test_config_in_dir(dir.path(), true, true);
    let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
    let body = put_config_body("policy.max_total_storage", "not-a-size");
    let (status, json) = send_json(router(state), "PUT", "/api/config", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["error"].as_str().unwrap().contains("invalid size"));
    assert!(!dir.path().join("swing.toml").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn an_unwritable_config_directory_is_internal_error() {
    let dir = tempfile::tempdir().unwrap();
    let Some(locked) = read_only_config_dir(dir.path()) else {
        return;
    };
    let (config, secret_hex) = test_config_in_dir(&locked, true, true);
    let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
    let body = put_config_body("policy.max_total_storage", "20GB");
    let (status, json) = send_json(router(state), "PUT", "/api/config", Some(body)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let error = json["error"].as_str().unwrap();
    assert!(error.contains("swing log"), "{error}");
    assert!(!error.contains("swing.toml"), "{error}");
}

#[tokio::test]
async fn mirror_preview_works_in_setup_mode_and_marks_invalid_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
    let state = build_state(config, test_exit(), None, TEST_TOKEN);
    let pk = Keys::generate().public_key();
    let body = serde_json::json!({ "keys": [pk.to_hex(), "nope", crate::mirror::npub(&pk)] });
    let (status, json) = send_json(router(state), "POST", "/api/mirror/preview", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    let keys = json["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0]["npub"], crate::mirror::npub(&pk));
    assert_eq!(
        keys[0]["fingerprint"],
        serde_json::json!(crate::fingerprint::fingerprint(&pk))
    );
    assert_eq!(keys[1]["input"], "nope");
    assert!(keys[1]["npub"].is_null());
    assert!(keys[1]["fingerprint"].is_null());
}

#[tokio::test]
async fn overview_reports_setup_mode_with_null_identity() {
    let dir = tempfile::tempdir().unwrap();
    let (config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
    let state = build_state(config, test_exit(), None, TEST_TOKEN);
    let app = router(state);
    let req = get("/api/overview");
    let resp = call(app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["setup"], true);
    assert!(json["pubkey"].is_null());
    assert!(json["npub"].is_null());
    assert!(json["signer"].is_null());
}

#[tokio::test]
async fn activity_reads_the_newest_stored_at_without_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    let (mut config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
    config.agent.state_dir = dir.path().join("state");
    let state_path = config.agent.state_dir.join("state.json");
    let state = build_state(config, test_exit(), None, TEST_TOKEN);

    let (status, json) = send_json(router(Arc::clone(&state)), "GET", "/api/activity", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["latest_stored_at"].is_null());
    assert!(json["latest_published_at"].is_null());
    assert!(json["latest_replica_report_at"].is_null());

    let mut saved = crate::state::State::default();
    for (d, stored_at) in [("x.example", 20), ("y.example", 50)] {
        saved.apply_store(
            &crate::state::site_key("aa", d),
            crate::state::VersionRecord {
                stored_at,
                ..crate::test_support::version(&format!("bafy{d}"), 1, 1)
            },
        );
    }
    saved.save(&state_path).await.unwrap();

    let (status, json) = send_json(router(state), "GET", "/api/activity", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["latest_stored_at"], 50);
}

#[tokio::test]
async fn activity_reads_what_the_agent_and_publishes_recorded() {
    let state = test_state();
    state.activity.record_published(300);
    state.activity.record_published(200);
    state.activity.record_replica_report(400);

    let (status, json) = send_json(router(state), "GET", "/api/activity", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["latest_published_at"], 300);
    assert_eq!(json["latest_replica_report_at"], 400);
}

#[tokio::test]
async fn overview_tells_whether_a_signer_app_signs() {
    let (_status, json) = send_json(router(test_state()), "GET", "/api/overview", None).await;
    assert_eq!(json["signer"]["remote"], false);
    assert!(json["signer"]["last_failure"].is_null());
}

#[tokio::test]
async fn stats_returns_samples_newer_than_since() {
    let state = test_state();
    let state_kubo_managed = state.config.kubo.managed;
    for at in [100, 160, 220] {
        state.stats.push(crate::stats::Sample {
            at,
            swing: Some(crate::stats::ProcessSample {
                cpu_percent: Some(1.5),
                rss_bytes: 4096,
            }),
            kubo: None,
            traffic: None,
        });
    }

    let (status, json) = send_json(
        router(Arc::clone(&state)),
        "GET",
        "/api/stats?since=100",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["interval"], 60);
    assert_eq!(json["kubo_managed"], state_kubo_managed);
    let ats: Vec<u64> = json["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["at"].as_u64().unwrap())
        .collect();
    assert_eq!(ats, vec![160, 220]);
    assert_eq!(json["samples"][0]["swing"]["rss_bytes"], 4096);
    assert!(json["samples"][0]["kubo"].is_null());

    let (status, _) = send_json(router(state), "GET", "/api/stats?since=x", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
