mod api;
mod assets;
mod config_dto;
pub(crate) mod dto;
mod error;
pub mod guard;
mod locks;
mod mascots;
mod precheck;
mod publish;
mod session;
mod setup;
#[cfg(test)]
mod test_support;
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
use nostr_sdk::prelude::{PublicKey, Timestamp};
use tokio::net::TcpListener;
use tokio::sync::{Notify, RwLock, Semaphore, oneshot};
use tower_http::timeout::TimeoutLayer;
use tracing::info;

use crate::activity::Activity;
use crate::auth::{self, LoginCodes};
use crate::config::Config;
use crate::ipfs::IpfsClient;
use crate::nostr::RelayClient;
use crate::shutdown::ExitRequest;
use crate::signer::{Pairing, PairingState, Signer};
use crate::stats;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const INSTANCE_ID_BYTES: usize = 8;
const RELAY_QUERY_PERMITS: usize = 4;

pub struct AppState {
    relay: RwLock<Option<Arc<RelayClient>>>,
    ipfs: RwLock<Option<IpfsClient>>,
    config: Arc<Config>,
    display_config: RwLock<Arc<Config>>,
    notify: Arc<Notify>,
    started_at: u64,
    instance: String,
    locks: locks::Locks,
    relay_queries: Semaphore,
    own_pubkey: Option<PublicKey>,
    pub(crate) signer: Option<Signer>,
    pairing: std::sync::Mutex<Option<Pairing>>,
    desktop: Option<DesktopAssets>,
    mascots: Option<mascots::MascotRegistry>,
    exit: ExitRequest,
    restart_required: std::sync::atomic::AtomicBool,
    token: std::sync::RwLock<String>,
    login_codes: LoginCodes,
    pub(crate) stats: Arc<stats::Recorder>,
    pub(crate) activity: Arc<Activity>,
}

impl AppState {
    pub fn new(
        config: Arc<Config>,
        notify: Arc<Notify>,
        exit: ExitRequest,
        signer: Option<Signer>,
        token: String,
    ) -> Result<Self> {
        let own_pubkey = signer.as_ref().map(Signer::public_key);
        let desktop = if config.dashboard.ui {
            Some(DesktopAssets::load(&config.dashboard)?)
        } else {
            None
        };
        let mascots = if config.dashboard.ui {
            Some(mascots::MascotRegistry::load(
                config.dashboard.mascots_dir.as_deref(),
            ))
        } else {
            None
        };
        let display_config = RwLock::new(config.clone());
        Ok(Self {
            relay: RwLock::new(None),
            ipfs: RwLock::new(None),
            config,
            display_config,
            notify,
            started_at: Timestamp::now().as_secs(),
            instance: auth::random_hex(INSTANCE_ID_BYTES),
            locks: locks::Locks::default(),
            relay_queries: Semaphore::new(RELAY_QUERY_PERMITS),
            own_pubkey,
            signer,
            pairing: std::sync::Mutex::new(None),
            desktop,
            mascots,
            exit,
            restart_required: std::sync::atomic::AtomicBool::new(false),
            token: std::sync::RwLock::new(token),
            login_codes: LoginCodes::default(),
            stats: Arc::default(),
            activity: Arc::default(),
        })
    }

    pub fn token(&self) -> String {
        self.token.read().expect("token lock").clone()
    }

    pub fn set_token(&self, token: String) {
        *self.token.write().expect("token lock") = token;
    }

    pub fn setup_mode(&self) -> bool {
        self.own_pubkey.is_none()
    }

    pub fn pairing_state(&self) -> Option<PairingState> {
        self.pairing
            .lock()
            .expect("pairing lock")
            .as_ref()
            .map(Pairing::state)
    }

    pub async fn display_config(&self) -> Arc<Config> {
        self.display_config.read().await.clone()
    }

    pub async fn set_display_config(&self, config: Config) {
        *self.display_config.write().await = Arc::new(config);
    }

    pub async fn set_ready(&self, relay: Arc<RelayClient>, ipfs: IpfsClient) {
        *self.relay.write().await = Some(relay);
        *self.ipfs.write().await = Some(ipfs);
    }

    pub async fn set_not_ready(&self) {
        *self.relay.write().await = None;
        *self.ipfs.write().await = None;
    }

    pub async fn relay(&self) -> Option<Arc<RelayClient>> {
        self.relay.read().await.clone()
    }

    pub async fn ipfs(&self) -> Option<IpfsClient> {
        self.ipfs.read().await.clone()
    }
}

fn ui_router() -> Router<Arc<AppState>> {
    assets::register(Router::new())
        .route("/desktop-page.html", get(assets::desktop_page))
        .route("/desktop-page.css", get(assets::desktop_page_css))
        .route("/desktop-banner", get(assets::desktop_banner))
        .route("/custom.css", get(assets::custom_css))
        .route("/mascots/index.json", get(mascots::index))
        .route("/mascots/{id}/{file}", get(mascots::file))
        .route("/login", get(session::login_page))
}

pub fn router(state: Arc<AppState>) -> Router {
    let max_upload = usize::try_from(state.config.dashboard.max_upload).unwrap_or(usize::MAX);

    let upload_route = Router::new()
        .route("/api/publish/upload", post(upload::publish_upload))
        .route("/api/publish/check", post(precheck::publish_check))
        .layer(DefaultBodyLimit::max(max_upload))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            UPLOAD_TIMEOUT,
        ));

    let mut app: Router<Arc<AppState>> = Router::new()
        .route("/api/overview", get(api::overview))
        .route("/api/activity", get(api::activity))
        .route("/api/stats", get(api::stats))
        .route("/api/sites", get(api::sites))
        .route("/api/status", get(api::status))
        .route("/api/mirror", get(api::mirror_list))
        .route("/api/mirror/add", post(api::mirror_add))
        .route("/api/mirror/remove", post(api::mirror_remove))
        .route("/api/webring", get(api::webring))
        .route("/api/replicas", get(api::replicas))
        .route("/api/publish/sites", get(api::publish_sites))
        .route(
            "/api/publish/previous-files",
            get(api::publish_previous_files),
        )
        .route("/api/config", get(api::config).put(api::update_config))
        .route("/api/setup", post(setup::setup))
        .route(
            "/api/setup/signer",
            get(setup::pairing_status).post(setup::start_pairing),
        )
        .route("/api/signer/reconnect", post(setup::reconnect_signer))
        .route("/api/shutdown", post(api::shutdown))
        .route("/api/restart", post(api::restart))
        .route("/api/login", post(session::login))
        .route("/api/identity", post(session::identity))
        .merge(
            Router::new()
                .route("/api/login-code", post(session::login_code))
                .route("/api/token/rotate", post(session::rotate_token))
                .route_layer(axum::middleware::from_fn(guard::require_bearer)),
        );

    if state.config.dashboard.ui {
        app = app.merge(ui_router());
    }

    app.layer(TimeoutLayer::with_status_code(
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
    info!(%addr, "dashboard listening; run `swing dashboard open` to log in");
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
    use super::test_support::*;
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

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
        let (mut config, secret_hex) = test_config(true);
        config.dashboard.allowed_hosts = vec!["my.example".to_string()];
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
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
    async fn the_literal_listen_ip_is_accepted_as_host() {
        for listen in ["192.168.1.5:8082", "[fd00::1]:8082"] {
            let (mut config, secret_hex) = test_config(true);
            config.dashboard.listen = listen.parse().unwrap();
            let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
            let req = Request::builder()
                .uri("/api/overview")
                .header("Host", listen)
                .body(Body::empty())
                .unwrap();
            let resp = call(router(Arc::clone(&state)), req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{listen}");

            let req = Request::builder()
                .uri("/api/overview")
                .header("Host", "192.168.1.99:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(router(state), req).await;
            assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{listen}");
        }
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
    async fn mirror_changes_wait_for_the_one_in_progress() {
        let state = test_state();
        let held = state.locks.mirror_writes().await;
        let key = nostr_sdk::prelude::Keys::generate().public_key().to_hex();
        let req = Request::builder()
            .method("POST")
            .uri("/api/mirror/remove")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(format!("{{\"keys\":[\"{key}\"]}}")))
            .unwrap();
        let pending = tokio::spawn(call(router(state.clone()), req));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!pending.is_finished());
        drop(held);
        let resp = pending.await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn requests_without_relay_report_service_unavailable_not_a_panic() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/api/sites")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
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

    #[tokio::test]
    async fn set_ready_then_not_ready_flips_availability() {
        let (config, secret_hex) = test_config(true);
        let secret_key = secret_hex.clone();
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
        let relay = Arc::new(
            crate::nostr::RelayClient::connect(test_keys(&secret_key).unwrap(), &[])
                .await
                .unwrap(),
        );
        let ipfs = crate::ipfs::IpfsClient::new("http://127.0.0.1:5001".to_string());
        state.set_ready(relay, ipfs).await;
        assert!(state.relay().await.is_some());
        assert!(state.ipfs().await.is_some());

        state.set_not_ready().await;
        assert!(state.relay().await.is_none());
        assert!(state.ipfs().await.is_none());
    }

    #[tokio::test]
    async fn ui_disabled_hides_static_routes_but_keeps_the_api() {
        let (mut config, secret_hex) = test_config(true);
        config.dashboard.ui = false;
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);

        let app = router(Arc::clone(&state));
        let req = Request::builder()
            .uri("/")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let app = router(state);
        let req = Request::builder()
            .uri("/api/overview")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
