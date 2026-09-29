#![cfg(test)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, Response, StatusCode, header};
use nostr_sdk::prelude::Keys;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::config::{CheckMode, Config};
use crate::shutdown::ExitRequest;
use crate::signer::Signer;

use super::AppState;

pub(crate) const TEST_TOKEN: &str = "test-token";

pub(crate) fn test_exit() -> ExitRequest {
    ExitRequest::new(CancellationToken::new())
}

pub(crate) fn test_config(ui: bool) -> (Config, String) {
    let secret_hex = Keys::generate().secret_key().to_secret_hex();
    let toml =
        format!("[nostr]\nsecret_key = \"{secret_hex}\"\nrelays = [\"wss://relay.example\"]\n");
    let mut config = crate::config::build_config_from_str(&toml, |_| None).unwrap();
    config.dashboard.ui = ui;
    config.policy.max_total_storage = 1;
    config.policy.max_per_site = 1;
    config.policy.max_per_account = 1;
    config.policy.max_sites_per_account = 1;
    config.policy.max_update_size = 1;
    config.policy.keep_versions = 1;
    config.policy.keep_days = 1;
    config.policy.min_update_interval = 0;
    config.policy.nip05 = CheckMode::Off;
    config.policy.nip05_cache_ttl = 1;
    config.agent.fetch_timeout = Duration::from_secs(60);
    config.agent.fetch_idle_timeout = Duration::from_secs(10);
    config.agent.concurrency = 1;
    config.agent.report_ttl = Duration::from_secs(3600);
    config.publish.nip05 = CheckMode::Off;
    config.publish.keep_versions = 1;
    config.kubo.storage_max = 1;
    config.config_path = PathBuf::from("./swing.toml");
    config.config_exists = true;
    (config, secret_hex)
}

pub(crate) fn test_keys(secret_hex: &str) -> Option<Signer> {
    Some(Signer::Local(Keys::parse(secret_hex).unwrap()))
}

pub(crate) fn build_state(
    config: Config,
    exit: ExitRequest,
    signer: Option<Signer>,
    token: &str,
) -> Arc<AppState> {
    Arc::new(
        AppState::new(
            Arc::new(config),
            Arc::new(Notify::new()),
            exit,
            signer,
            token.to_string(),
        )
        .unwrap(),
    )
}

pub(crate) fn test_state() -> Arc<AppState> {
    let (config, secret_hex) = test_config(true);
    build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN)
}

pub(crate) fn test_state_with(state_dir: PathBuf, max_upload: u64) -> Arc<AppState> {
    let (mut config, secret_hex) = test_config(true);
    config.agent.state_dir = state_dir;
    config.dashboard.max_upload = max_upload;
    build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN)
}

pub(crate) async fn call_anonymous(app: Router, req: Request<Body>) -> Response<Body> {
    app.oneshot(req).await.unwrap()
}

pub(crate) async fn call(app: Router, mut req: Request<Body>) -> Response<Body> {
    req.headers_mut()
        .entry(header::AUTHORIZATION)
        .or_insert(HeaderValue::from_static("Bearer test-token"));
    call_anonymous(app, req).await
}

pub(crate) async fn error_body(resp: Response<Body>) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).expect("error body must be JSON")
}

pub(crate) async fn send_json(
    app: Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Host", "127.0.0.1:8082")
        .header("x-swing-dashboard", "1");
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let resp = call(app, builder.body(body).unwrap()).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}
