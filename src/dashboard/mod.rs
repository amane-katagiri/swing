mod api;
mod assets;
mod dto;
pub mod guard;
mod upload;

pub use assets::DesktopAssets;
pub use upload::cleanup_upload_dir;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::routing::{get, post};
use nostr_sdk::prelude::{Keys, PublicKey, Timestamp};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, oneshot};
use tower_http::timeout::TimeoutLayer;
use tracing::info;

use crate::config::Config;
use crate::ipfs::IpfsClient;
use crate::nostr::RelayClient;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct AppState {
    pub relay: Option<Arc<RelayClient>>,
    pub config: Arc<Config>,
    pub ipfs: IpfsClient,
    pub notify: Arc<Notify>,
    pub started_at: u64,
    pub publish_lock: Mutex<()>,
    pub own_pubkey: PublicKey,
    pub desktop: DesktopAssets,
}

impl AppState {
    pub fn new(
        relay: Option<Arc<RelayClient>>,
        config: Arc<Config>,
        notify: Arc<Notify>,
    ) -> Result<Self> {
        let ipfs = IpfsClient::new(config.ipfs.api.clone());
        let own_pubkey = Keys::parse(config.nostr.secret_key.expose_secret())
            .context("parsing configured secret key")?
            .public_key();
        let desktop = DesktopAssets::load(&config.dashboard)?;
        Ok(Self {
            relay,
            config,
            ipfs,
            notify,
            started_at: Timestamp::now().as_secs(),
            publish_lock: Mutex::new(()),
            own_pubkey,
            desktop,
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let max_upload = usize::try_from(state.config.dashboard.max_upload).unwrap_or(usize::MAX);

    let upload_route = Router::new()
        .route("/api/publish/upload", post(upload::publish_upload))
        .layer(DefaultBodyLimit::max(max_upload))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            UPLOAD_TIMEOUT,
        ));

    Router::new()
        .route("/", get(assets::index))
        .route("/style.css", get(assets::style))
        .route("/desktop.css", get(assets::desktop_css))
        .route("/boot.js", get(assets::boot_js))
        .route("/app.js", get(assets::app_js))
        .route("/graph.js", get(assets::graph_js))
        .route("/storage.js", get(assets::storage_js))
        .route("/i18n.js", get(assets::i18n_js))
        .route("/util.js", get(assets::util_js))
        .route("/ui.js", get(assets::ui_js))
        .route("/sites.js", get(assets::sites_js))
        .route("/webring.js", get(assets::webring_js))
        .route("/publish.js", get(assets::publish_js))
        .route("/settings.js", get(assets::settings_js))
        .route("/desktop.js", get(assets::desktop_js))
        .route("/desktop-page.html", get(assets::desktop_page))
        .route("/desktop-page.css", get(assets::desktop_page_css))
        .route("/desktop-frame.css", get(assets::desktop_frame_css))
        .route("/desktop-banner.png", get(assets::desktop_banner))
        .route(
            "/fonts/pixelmplus12-regular.woff2",
            get(assets::font_pixelmplus12_regular),
        )
        .route(
            "/fonts/pixelmplus12-bold.woff2",
            get(assets::font_pixelmplus12_bold),
        )
        .route("/custom.css", get(assets::custom_css))
        .route("/api/overview", get(api::overview))
        .route("/api/sites", get(api::sites))
        .route("/api/status", get(api::status))
        .route("/api/mirror", get(api::mirror_list))
        .route("/api/mirror/add", post(api::mirror_add))
        .route("/api/mirror/remove", post(api::mirror_remove))
        .route("/api/webring", get(api::webring))
        .route("/api/replicas", get(api::replicas))
        .route("/api/publish/sites", get(api::publish_sites))
        .route("/api/config", get(api::config))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .merge(upload_route)
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            guard::security_middleware,
        ))
        .with_state(state)
}

pub async fn serve(
    listener: TcpListener,
    state: Arc<AppState>,
    shutdown: oneshot::Receiver<()>,
) -> Result<()> {
    let addr = listener
        .local_addr()
        .context("reading dashboard listener address")?;
    info!(%addr, "dashboard listening");
    let app = router(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.await;
        })
        .await
        .context("dashboard server error")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use nostr_sdk::prelude::Keys;
    use std::path::PathBuf;
    use std::time::Duration;
    use tower::ServiceExt;

    use crate::config::{
        AgentConfig, DashboardConfig, DashboardListen, IpfsConfig, Nip05Mode, NostrConfig,
        PolicyConfig, PublishConfig,
    };

    fn test_config(listen: DashboardListen) -> (Config, String) {
        let secret_hex = Keys::generate().secret_key().to_secret_hex();
        let config = Config {
            nostr: NostrConfig {
                secret_key: secret_hex.clone().into(),
                relays: vec!["wss://relay.example".to_string()],
                mirror_set: "swing".to_string(),
                site_event_kind: 35980,
                replica_event_kind: 35981,
            },
            ipfs: IpfsConfig {
                api: "http://127.0.0.1:5001".to_string(),
                mfs_root: "/swing".to_string(),
            },
            policy: PolicyConfig {
                max_total_storage: 1,
                max_per_site: 1,
                max_per_account: 1,
                max_sites_per_account: 1,
                max_update_size: 1,
                keep_versions: 1,
                keep_days: 1,
                min_update_interval: 0,
                remove_on_unfollow: true,
                nip05: Nip05Mode::Off,
                nip05_cache_ttl: 1,
            },
            agent: AgentConfig {
                state_dir: PathBuf::from("./data"),
                poll_interval: Duration::from_secs(300),
                fetch_timeout: Duration::from_secs(60),
                fetch_idle_timeout: Duration::from_secs(10),
                concurrency: 1,
                report_ttl: Duration::from_secs(3600),
            },
            publish: PublishConfig {
                nip05: Nip05Mode::Off,
                keep_versions: 1,
            },
            dashboard: DashboardConfig {
                listen,
                allowed_hosts: Vec::new(),
                gateway: Some("http://localhost:8080".to_string()),
                custom_css: None,
                desktop_page: None,
                desktop_page_css: None,
                desktop_banner: None,
                max_upload: 2 * (1u64 << 30),
            },
            config_path: None,
        };
        (config, secret_hex)
    }

    fn test_state() -> Arc<AppState> {
        let (config, _secret_hex) = test_config(DashboardListen::Off);
        Arc::new(AppState::new(None, Arc::new(config), Arc::new(Notify::new())).unwrap())
    }

    async fn call(app: Router, req: Request<Body>) -> axum::http::Response<Body> {
        app.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn index_is_served_as_html_with_security_headers() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert!(resp.headers().get("content-security-policy").is_some());
        assert_eq!(
            resp.headers().get("referrer-policy").unwrap(),
            "no-referrer"
        );
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "SAMEORIGIN");
    }

    #[tokio::test]
    async fn style_and_scripts_have_expected_content_types() {
        for (path, expected) in [
            ("/style.css", "text/css; charset=utf-8"),
            ("/desktop.css", "text/css; charset=utf-8"),
            ("/boot.js", "text/javascript; charset=utf-8"),
            ("/app.js", "text/javascript; charset=utf-8"),
            ("/graph.js", "text/javascript; charset=utf-8"),
            ("/storage.js", "text/javascript; charset=utf-8"),
            ("/i18n.js", "text/javascript; charset=utf-8"),
            ("/util.js", "text/javascript; charset=utf-8"),
            ("/ui.js", "text/javascript; charset=utf-8"),
            ("/sites.js", "text/javascript; charset=utf-8"),
            ("/webring.js", "text/javascript; charset=utf-8"),
            ("/publish.js", "text/javascript; charset=utf-8"),
            ("/settings.js", "text/javascript; charset=utf-8"),
            ("/desktop.js", "text/javascript; charset=utf-8"),
            ("/desktop-page.html", "text/html; charset=utf-8"),
            ("/desktop-page.css", "text/css; charset=utf-8"),
            ("/desktop-frame.css", "text/css; charset=utf-8"),
            ("/desktop-banner.png", "image/png"),
            ("/fonts/pixelmplus12-regular.woff2", "font/woff2"),
            ("/fonts/pixelmplus12-bold.woff2", "font/woff2"),
            ("/custom.css", "text/css; charset=utf-8"),
        ] {
            let app = router(test_state());
            let req = Request::builder()
                .uri(path)
                .header("Host", "127.0.0.1:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(app, req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers().get("content-type").unwrap(),
                expected,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn desktop_page_assets_come_from_the_configured_files() {
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("page.html");
        let page_css = dir.path().join("page.css");
        let banner = dir.path().join("banner.gif");
        std::fs::write(&page, "<!doctype html><title>mine</title>").unwrap();
        std::fs::write(&page_css, "body { color: red }").unwrap();
        std::fs::write(&banner, b"GIF89a").unwrap();

        let (mut config, _secret_hex) = test_config(DashboardListen::Off);
        config.dashboard.desktop_page = Some(page);
        config.dashboard.desktop_page_css = Some(page_css);
        config.dashboard.desktop_banner = Some(banner);
        let state =
            Arc::new(AppState::new(None, Arc::new(config), Arc::new(Notify::new())).unwrap());

        for (path, content_type, expected) in [
            (
                "/desktop-page.html",
                "text/html; charset=utf-8",
                &b"<!doctype html><title>mine</title>"[..],
            ),
            (
                "/desktop-page.css",
                "text/css; charset=utf-8",
                b"body { color: red }",
            ),
            ("/desktop-banner.png", "image/gif", b"GIF89a"),
        ] {
            let app = router(Arc::clone(&state));
            let req = Request::builder()
                .uri(path)
                .header("Host", "127.0.0.1:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(app, req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers().get("content-type").unwrap(),
                content_type,
                "{path}"
            );
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), expected, "{path}");
        }
    }

    #[test]
    fn an_unreadable_desktop_page_fails_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, _secret_hex) = test_config(DashboardListen::Off);
        config.dashboard.desktop_page = Some(dir.path().join("missing.html"));
        let err = match AppState::new(None, Arc::new(config), Arc::new(Notify::new())) {
            Ok(_) => panic!("expected a startup error"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("missing.html"));
    }

    #[test]
    fn a_banner_with_an_unknown_extension_fails_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let banner = dir.path().join("banner.bmp");
        std::fs::write(&banner, b"BM").unwrap();
        let (mut config, _secret_hex) = test_config(DashboardListen::Off);
        config.dashboard.desktop_banner = Some(banner);
        let err = match AppState::new(None, Arc::new(config), Arc::new(Notify::new())) {
            Ok(_) => panic!("expected a startup error"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("banner.bmp"));
    }

    #[tokio::test]
    async fn bad_host_header_is_forbidden() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/api/config")
            .header("Host", "evil.example")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn allowed_host_from_config_is_accepted() {
        let (mut config, _secret_hex) = test_config(DashboardListen::Off);
        config.dashboard.allowed_hosts = vec!["my.example".to_string()];
        let state =
            Arc::new(AppState::new(None, Arc::new(config), Arc::new(Notify::new())).unwrap());
        let app = router(state);
        let req = Request::builder()
            .uri("/api/config")
            .header("Host", "my.example:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn write_request_without_marker_header_is_forbidden() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/add")
            .header("Host", "127.0.0.1:8082")
            .header("content-type", "application/json")
            .body(Body::from("{\"keys\":[]}"))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn write_request_with_mismatched_origin_is_forbidden() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/add")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("origin", "http://evil.example")
            .header("content-type", "application/json")
            .body(Body::from("{\"keys\":[]}"))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn write_request_without_body_keys_is_bad_request_when_guard_passes() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/add")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from("{\"keys\":[]}"))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn config_endpoint_never_exposes_the_secret_key_value() {
        let (config, secret_hex) = test_config(DashboardListen::Off);
        let state =
            Arc::new(AppState::new(None, Arc::new(config), Arc::new(Notify::new())).unwrap());
        let app = router(state);
        let req = Request::builder()
            .uri("/api/config")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
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
    async fn webring_depth_over_the_limit_is_bad_request() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/api/webring?depth=5")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn requests_without_relay_report_a_server_error_not_a_panic() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/api/sites")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    async fn error_body(resp: axum::http::Response<Body>) -> serde_json::Value {
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).expect("error body must be JSON")
    }

    #[tokio::test]
    async fn malformed_json_body_is_bad_request_with_json_error_even_without_relay() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/add")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from("{not json"))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = error_body(resp).await;
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn json_body_missing_a_field_is_bad_request_not_unprocessable() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/add")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = error_body(resp).await;
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn missing_content_type_is_bad_request_with_json_error() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/add")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .body(Body::from("{\"keys\":[\"abc\"]}"))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = error_body(resp).await;
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn mirror_add_rejects_more_than_the_key_limit() {
        let keys: Vec<String> = (0..101).map(|i| format!("key-{i}")).collect();
        let req_body = serde_json::json!({ "keys": keys }).to_string();
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/add")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(req_body))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webring_rejects_more_roots_than_the_limit() {
        let query: String = (0..101)
            .map(|i| format!("root=key-{i}"))
            .collect::<Vec<_>>()
            .join("&");
        let app = router(test_state());
        let req = Request::builder()
            .uri(format!("/api/webring?{query}"))
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn replicas_rejects_more_keys_than_the_limit() {
        let query: String = (0..101)
            .map(|i| format!("key=key-{i}"))
            .collect::<Vec<_>>()
            .join("&");
        let app = router(test_state());
        let req = Request::builder()
            .uri(format!("/api/replicas?{query}"))
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_path_is_still_guarded_by_host_check() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/no-such-route")
            .header("Host", "evil.example")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    fn test_state_with(state_dir: PathBuf, max_upload: u64) -> Arc<AppState> {
        let (mut config, _secret_hex) = test_config(DashboardListen::Off);
        config.agent.state_dir = state_dir;
        config.dashboard.max_upload = max_upload;
        Arc::new(AppState::new(None, Arc::new(config), Arc::new(Notify::new())).unwrap())
    }

    fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, filename, content) in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match filename {
                Some(fname) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{fname}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(content);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        body
    }

    fn multipart_request(uri: &str, boundary: &str, body: Vec<u8>) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap()
    }

    fn upload_dir_entries(state_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        match std::fs::read_dir(state_dir.join("upload")) {
            Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => panic!("reading upload dir: {e}"),
        }
    }

    #[tokio::test]
    async fn upload_rejects_an_invalid_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
        let boundary = "SwingTestBoundary";
        let body = multipart_body(
            boundary,
            &[
                ("site", None, b"example.com"),
                ("file", Some("../evil.txt"), b"hello"),
            ],
        );
        let app = router(state);
        let resp = call(
            app,
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn upload_rejects_more_files_than_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
        let boundary = "SwingTestBoundary";
        let mut parts: Vec<(&str, Option<&str>, &[u8])> = vec![("site", None, b"example.com")];
        let names: Vec<String> = (0..=upload::MAX_UPLOAD_FILES)
            .map(|i| format!("f{i}.txt"))
            .collect();
        for name in &names {
            parts.push(("file", Some(name.as_str()), b"x"));
        }
        let body = multipart_body(boundary, &parts);
        let app = router(state);
        let resp = call(
            app,
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn upload_rejects_zero_files() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
        let boundary = "SwingTestBoundary";
        let body = multipart_body(boundary, &[("site", None, b"example.com")]);
        let app = router(state);
        let resp = call(
            app,
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn upload_rejects_a_missing_site() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
        let boundary = "SwingTestBoundary";
        let body = multipart_body(boundary, &[("file", Some("index.html"), b"<html></html>")]);
        let app = router(state);
        let resp = call(
            app,
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn upload_over_the_body_limit_is_rejected_with_json_413() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 16);
        let boundary = "SwingTestBoundary";
        let body = multipart_body(
            boundary,
            &[
                ("site", None, b"example.com"),
                ("file", Some("index.html"), &[b'a'; 4096]),
            ],
        );
        let app = router(state);
        let resp = call(
            app,
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = error_body(resp).await;
        assert!(body["error"].is_string());
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn upload_rejects_an_invalid_site_before_touching_the_relay() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
        let boundary = "SwingTestBoundary";
        let body = multipart_body(
            boundary,
            &[
                ("site", None, b""),
                ("file", Some("index.html"), b"<html></html>"),
            ],
        );
        let app = router(state);
        let resp = call(
            app,
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = error_body(resp).await;
        assert!(body["error"].as_str().unwrap().contains("invalid site"));
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn upload_rejects_an_invalid_title_before_touching_the_relay() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
        let boundary = "SwingTestBoundary";
        let body = multipart_body(
            boundary,
            &[
                ("site", None, b"example.com"),
                ("title", None, b"bad\ntitle"),
                ("file", Some("index.html"), b"<html></html>"),
            ],
        );
        let app = router(state);
        let resp = call(
            app,
            multipart_request("/api/publish/upload", boundary, body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = error_body(resp).await;
        assert!(body["error"].as_str().unwrap().contains("invalid title"));
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn publish_sites_without_relay_reports_a_server_error() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/api/publish/sites")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
