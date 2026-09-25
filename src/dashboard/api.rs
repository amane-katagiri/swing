use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::{FromRequest, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nostr_sdk::prelude::*;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tracing::error;

use crate::config;
use crate::health;
use crate::mfs::MfsLayout;
use crate::mirror;
use crate::nip05;
use crate::publish;
use crate::replicas;
use crate::settings;
use crate::signer::{self, Pairing, PairingRequest, PairingState, Signer};
use crate::webring;

use super::AppState;
use super::dto;

const MAX_KEYS: usize = 100;

pub enum ApiError {
    BadRequest(String),
    Unauthorized(String),
    PayloadTooLarge(String),
    NotReady,
    NotConfigured,
    Upstream(String),
    Conflict(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            ApiError::PayloadTooLarge(msg) => (StatusCode::PAYLOAD_TOO_LARGE, msg),
            ApiError::NotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent is not ready".to_string(),
            ),
            ApiError::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent is not configured".to_string(),
            ),
            ApiError::Upstream(msg) => (StatusCode::BAD_GATEWAY, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

fn upstream(e: anyhow::Error) -> ApiError {
    ApiError::Upstream(format!("{e:#}"))
}

fn not_ready(state: &AppState) -> ApiError {
    if state.setup_mode() {
        ApiError::NotConfigured
    } else {
        ApiError::NotReady
    }
}

impl AppState {
    async fn require_relay(&self) -> Result<Arc<crate::nostr::RelayClient>, ApiError> {
        self.relay().await.ok_or_else(|| not_ready(self))
    }

    async fn require_ipfs(&self) -> Result<crate::ipfs::IpfsClient, ApiError> {
        self.ipfs().await.ok_or_else(|| not_ready(self))
    }
}

// axum maps deserialize errors to 422, which this API reserves for the publish NIP-05 require failure.
pub struct AppJson<T>(pub T);

impl<T, S> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(AppJson(value)),
            Err(rejection) => {
                let message = rejection.to_string();
                Err(if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::PayloadTooLarge(message)
                } else {
                    ApiError::BadRequest(message)
                })
            }
        }
    }
}

pub async fn overview(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::OverviewDto>, ApiError> {
    let pubkey = state.own_pubkey;
    Ok(Json(dto::OverviewDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        setup: pubkey.is_none(),
        pubkey: pubkey.map(|pk| pk.to_hex()),
        npub: pubkey.map(|pk| mirror::npub(&pk)),
        relays: state.config.nostr.relays.clone(),
        mirror_set: state.config.nostr.mirror_set.clone(),
        gateway: state.config.dashboard.gateway.clone(),
        started_at: state.started_at,
        instance: state.instance.clone(),
        max_upload: state.config.dashboard.max_upload,
        signer: state.signer.as_ref().map(dto::signer_dto),
    }))
}

pub async fn sites(State(state): State<Arc<AppState>>) -> Result<Json<dto::SitesDto>, ApiError> {
    let relay = state.require_relay().await?;
    let view = mirror::collect_sites(&relay, &state.config)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::sites_dto(
        &view,
        state.config.dashboard.gateway.as_deref(),
    )))
}

pub async fn status(State(state): State<Arc<AppState>>) -> Result<Json<dto::StatusDto>, ApiError> {
    let ipfs = state.require_ipfs().await?;
    let report = health::collect_status(&ipfs, &state.config)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::status_dto(&report)))
}

pub async fn mirror_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::MirrorListDto>, ApiError> {
    let relay = state.require_relay().await?;
    let view = mirror::collect_mirror_list(&relay, &state.config)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::mirror_list_dto(&view)))
}

#[derive(Debug, Deserialize)]
pub struct MirrorKeysRequest {
    keys: Vec<String>,
}

fn validate_keys(keys: &[String]) -> Result<(), ApiError> {
    if keys.is_empty() {
        return Err(ApiError::BadRequest(
            "keys must include at least one entry".to_string(),
        ));
    }
    if keys.len() > MAX_KEYS {
        return Err(ApiError::BadRequest(format!(
            "keys must include at most {MAX_KEYS} entries"
        )));
    }
    mirror::parse_pubkey_inputs(keys).map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    Ok(())
}

fn finish_mirror_change(
    state: &AppState,
    change: mirror::MirrorChange,
) -> Result<Json<dto::MirrorChangeDto>, ApiError> {
    if change.published && !change.relay_results.iter().any(|r| r.ok) {
        return Err(ApiError::Upstream(
            "no relay accepted the mirror set update".to_string(),
        ));
    }
    if change.published {
        state.notify.notify_one();
    }
    Ok(Json(dto::mirror_change_dto(&change)))
}

fn mirror_add_error(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<mirror::FollowSetCapExceeded>() {
        Some(cap) => ApiError::Conflict(cap.to_string()),
        None => upstream(e),
    }
}

pub async fn mirror_add(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<MirrorKeysRequest>,
) -> Result<Json<dto::MirrorChangeDto>, ApiError> {
    validate_keys(&req.keys)?;
    let relay = state.require_relay().await?;
    let change = mirror::apply_add(&relay, &state.config, &req.keys)
        .await
        .map_err(mirror_add_error)?;
    finish_mirror_change(&state, change)
}

pub async fn mirror_remove(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<MirrorKeysRequest>,
) -> Result<Json<dto::MirrorChangeDto>, ApiError> {
    validate_keys(&req.keys)?;
    let relay = state.require_relay().await?;
    let change = mirror::apply_remove(&relay, &state.config, &req.keys)
        .await
        .map_err(upstream)?;
    finish_mirror_change(&state, change)
}

const MAX_WEBRING_DEPTH: usize = 4;
const DEFAULT_WEBRING_DEPTH: usize = 2;

pub async fn webring(
    State(state): State<Arc<AppState>>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<dto::WebringDto>, ApiError> {
    let mut root_inputs = Vec::new();
    let mut depth = DEFAULT_WEBRING_DEPTH;
    for (key, value) in &params {
        match key.as_str() {
            "root" => root_inputs.push(value.clone()),
            "depth" => {
                depth = value
                    .parse()
                    .map_err(|_| ApiError::BadRequest(format!("invalid depth: {value}")))?;
            }
            _ => {}
        }
    }
    if depth > MAX_WEBRING_DEPTH {
        return Err(ApiError::BadRequest(format!(
            "depth must be between 0 and {MAX_WEBRING_DEPTH}"
        )));
    }
    if root_inputs.len() > MAX_KEYS {
        return Err(ApiError::BadRequest(format!(
            "root must include at most {MAX_KEYS} entries"
        )));
    }
    let relay = state.require_relay().await?;
    let roots = if root_inputs.is_empty() {
        vec![
            state
                .own_pubkey
                .expect("own_pubkey is set whenever the relay is ready"),
        ]
    } else {
        mirror::parse_pubkey_inputs(&root_inputs)
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?
    };

    let view = webring::collect(&relay, &state.config, &roots, depth)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::webring_dto(&view)))
}

pub async fn replicas(
    State(state): State<Arc<AppState>>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<dto::ReplicasDto>, ApiError> {
    let key_inputs: Vec<String> = params
        .into_iter()
        .filter(|(k, _)| k == "key")
        .map(|(_, v)| v)
        .collect();
    if key_inputs.len() > MAX_KEYS {
        return Err(ApiError::BadRequest(format!(
            "key must include at most {MAX_KEYS} entries"
        )));
    }
    let relay = state.require_relay().await?;
    let authors = if key_inputs.is_empty() {
        vec![
            state
                .own_pubkey
                .expect("own_pubkey is set whenever the relay is ready"),
        ]
    } else {
        mirror::parse_pubkey_inputs(&key_inputs)
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?
    };

    let authors_data = replicas::collect(&relay, &state.config, &authors)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::replicas_dto(&authors_data)))
}

pub(super) struct PublishFields {
    pub site: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub nip05: Option<String>,
}

pub(super) enum PublishOutcome {
    Success(dto::PublishResultDto),
    Nip05Failed(Response),
}

async fn run_publish_nip05(
    nip05_mode: config::Nip05Mode,
    site: &str,
    pubkey_hex: &str,
) -> Result<dto::Nip05ResultDto, Response> {
    if nip05_mode == config::Nip05Mode::Off {
        return Ok(dto::nip05_off_dto());
    }
    let verifier = nip05::HttpNip05Verifier::public_only();
    let outcome = publish::check_nip05(&verifier, nip05_mode, site, pubkey_hex).await;
    if let Some(abort) = outcome.abort {
        let body = serde_json::json!({
            "error": abort,
            "nip05": dto::nip05_result_dto(&outcome.result),
        });
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
    }
    Ok(dto::nip05_result_dto(&outcome.result))
}

pub(super) async fn run_publish(
    state: &AppState,
    dir: &std::path::Path,
    fields: PublishFields,
) -> Result<PublishOutcome, ApiError> {
    publish::validate_site_fields(&fields.site, fields.url.as_deref()).map_err(|e| match e {
        publish::SiteFieldError::InvalidD(err) => {
            ApiError::BadRequest(format!("invalid site: {err:#}"))
        }
        publish::SiteFieldError::InvalidUrl(url) => {
            ApiError::BadRequest(format!("invalid url: {url} is not an http or https URL"))
        }
        publish::SiteFieldError::InvalidTitle => unreachable!(),
    })?;
    let title = publish::normalize_title(fields.title.as_deref()).map_err(|_| {
        ApiError::BadRequest(
            "invalid title: must not exceed 256 bytes and must not contain control characters"
                .to_string(),
        )
    })?;
    let nip05_mode =
        publish::resolve_nip05_mode(fields.nip05.as_deref(), state.config.publish.nip05)
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;

    let Ok(_permit) = state.publish_lock.try_lock() else {
        return Err(ApiError::Conflict(
            "a publish is already running".to_string(),
        ));
    };

    let pubkey_hex = state.own_pubkey.ok_or_else(|| not_ready(state))?.to_hex();

    let nip05_dto = match run_publish_nip05(nip05_mode, &fields.site, &pubkey_hex).await {
        Ok(dto) => dto,
        Err(resp) => return Ok(PublishOutcome::Nip05Failed(resp)),
    };

    let ipfs = state.require_ipfs().await?;
    let layout = MfsLayout::new(state.config.ipfs.mfs_root.clone());
    let created_at = Timestamp::now();
    let stage = publish::add_and_measure(
        &ipfs,
        &layout,
        &pubkey_hex,
        &fields.site,
        created_at.as_secs(),
        dir,
    )
    .await
    .map_err(upstream)?;

    let relay = state.require_relay().await?;
    let relay_results = publish::sign_and_send(
        &relay,
        &publish::SiteAnnouncement {
            site_event_kind: state.config.nostr.site_event_kind,
            d: &fields.site,
            cid: &stage.cid,
            url: fields.url.as_deref(),
            size: stage.size,
            title,
            message: fields.message.as_deref(),
            created_at,
        },
    )
    .await
    .map_err(upstream)?;

    if !relay_results.iter().any(|r| r.ok) {
        return Err(ApiError::Upstream(
            "no relay accepted the site event; old versions were kept".to_string(),
        ));
    }

    let site_path = layout.publish_site(&pubkey_hex, &fields.site);
    let prune =
        publish::prune_old_versions_collect(&ipfs, &site_path, state.config.publish.keep_versions)
            .await;

    let gateway_url = dto::gateway_url(state.config.dashboard.gateway.as_deref(), &stage.cid, true);

    Ok(PublishOutcome::Success(dto::PublishResultDto {
        site: fields.site,
        url: fields.url,
        title: title.map(str::to_string),
        message: fields.message,
        nip05: nip05_dto,
        cid: stage.cid,
        size: stage.size,
        created_at: created_at.as_secs(),
        mfs_path: stage.path,
        relays: relay_results
            .iter()
            .map(dto::RelayResultDto::from)
            .collect(),
        pruned: prune.pruned().into_iter().map(str::to_string).collect(),
        prune_error: prune.error_summary(),
        gateway_url,
    }))
}

pub async fn publish_sites(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::PublishSitesDto>, ApiError> {
    let relay = state.require_relay().await?;
    let own_pubkey = state
        .own_pubkey
        .expect("own_pubkey is set whenever the relay is ready");
    let events = relay
        .fetch_site_events(state.config.nostr.site_event_kind, &[own_pubkey])
        .await
        .map_err(upstream)?;
    let parsed: Vec<crate::nostr::SiteEvent> = events
        .iter()
        .filter_map(|e| crate::nostr::parse_site_event(e, state.config.nostr.site_event_kind).ok())
        .collect();
    let latest = crate::nostr::select_latest(&parsed, Timestamp::now().as_secs());
    let sites = crate::nostr::cap_sites_per_author(
        latest.values(),
        crate::nostr::budget::MAX_SITES_PER_AUTHOR_LISTED,
    );
    Ok(Json(dto::publish_sites_dto(
        &sites,
        state.config.dashboard.gateway.as_deref(),
    )))
}

pub async fn config(State(state): State<Arc<AppState>>) -> Json<dto::ConfigDto> {
    let restart_required = state.restart_required.load(Ordering::SeqCst);
    let current = state.display_config().await;
    Json(dto::config_dto(&current, restart_required))
}

#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    items: BTreeMap<String, settings::InputValue>,
}

fn internal(context: &str, e: impl std::fmt::Display) -> ApiError {
    let message = format!("{e:#}");
    error!(error = %message, "{context}");
    ApiError::Internal(message)
}

fn settings_error(e: settings::EditError) -> ApiError {
    match e {
        settings::EditError::Invalid(e) => ApiError::BadRequest(format!("{e:#}")),
        settings::EditError::Io(e) => internal("saving the config file failed", e),
    }
}

pub async fn update_config(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<UpdateConfigRequest>,
) -> Result<Json<dto::ConfigDto>, ApiError> {
    let _writes = state.config_writes.lock().await;
    let updated = settings::update(&state.config, &req.items).map_err(settings_error)?;
    state.restart_required.store(true, Ordering::SeqCst);
    let dto = dto::config_dto(&updated, true);
    state.set_display_config(updated).await;
    Ok(Json(dto))
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    secret_key: Option<String>,
    #[serde(default)]
    remote_signer: bool,
    #[serde(default)]
    items: BTreeMap<String, settings::InputValue>,
}

fn ensure_setup_mode(state: &AppState) -> Result<(), ApiError> {
    if state.setup_mode() {
        Ok(())
    } else {
        Err(ApiError::Conflict(
            "swing is already configured; setup is no longer available".to_string(),
        ))
    }
}

fn uses_signer_app(state: &AppState) -> bool {
    state.signer.as_ref().is_some_and(Signer::is_remote)
}

fn ensure_can_pair(state: &AppState) -> Result<(), ApiError> {
    if state.setup_mode() || uses_signer_app(state) {
        Ok(())
    } else {
        Err(ApiError::Conflict(
            "a signer app can be paired only during setup or when swing already uses one"
                .to_string(),
        ))
    }
}

fn ready_pairing(state: &AppState) -> Result<Box<signer::PairedSigner>, ApiError> {
    match state.pairing_state() {
        Some(PairingState::Ready(paired)) => Ok(paired),
        _ => Err(ApiError::Conflict(
            "no signer app is connected yet; scan the QR code first".to_string(),
        )),
    }
}

fn schedule_restart(state: &AppState) {
    let exit = state.exit.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        exit.restart();
    });
}

pub async fn setup(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<SetupRequest>,
) -> Result<Response, ApiError> {
    ensure_setup_mode(&state)?;
    let mut writes = state.config_writes.lock().await;
    if writes.setup_done {
        return Err(ApiError::Conflict(
            "setup is already done; swing is restarting".to_string(),
        ));
    }
    let pubkey = if req.remote_signer {
        let paired = ready_pairing(&state)?;
        settings::setup(&state.config, None, &req.items).map_err(settings_error)?;
        paired
            .file
            .save(&state.config.agent.state_dir)
            .map_err(|e| internal("saving the signer app connection failed", e))?;
        *state.pairing.lock().expect("pairing lock") = None;
        paired.user
    } else {
        let keys = settings::setup_keys(req.secret_key.as_deref())
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
        settings::setup(&state.config, Some(&keys), &req.items).map_err(settings_error)?;
        keys.public_key()
    };
    writes.setup_done = true;
    drop(writes);
    let npub = mirror::npub(&pubkey);
    schedule_restart(&state);
    let body = Json(serde_json::json!({ "ok": true, "npub": npub, "restart": true }));
    Ok((StatusCode::OK, body).into_response())
}

pub async fn reconnect_signer(State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    if !uses_signer_app(&state) {
        return Err(ApiError::Conflict(
            "swing does not use a signer app".to_string(),
        ));
    }
    let paired = ready_pairing(&state)?;
    let own = state.own_pubkey.expect("a signer implies a public key");
    if paired.user != own {
        return Err(ApiError::Conflict(format!(
            "the signer app signs as {}, not as this swing's {}; connect the same Nostr account",
            mirror::npub(&paired.user),
            mirror::npub(&own)
        )));
    }
    let writes = state.config_writes.lock().await;
    paired
        .file
        .save(&state.config.agent.state_dir)
        .map_err(|e| internal("saving the signer app connection failed", e))?;
    drop(writes);
    *state.pairing.lock().expect("pairing lock") = None;
    state.restart_required.store(true, Ordering::SeqCst);
    schedule_restart(&state);
    let body = Json(serde_json::json!({ "ok": true, "npub": mirror::npub(&own), "restart": true }));
    Ok((StatusCode::OK, body).into_response())
}

#[derive(Debug, Deserialize)]
pub struct StartPairingRequest {
    relays: Vec<String>,
}

pub async fn start_pairing(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<StartPairingRequest>,
) -> Result<Json<dto::PairingStartDto>, ApiError> {
    ensure_can_pair(&state)?;
    let bad_request = |e: anyhow::Error| ApiError::BadRequest(format!("{e:#}"));
    let relays = signer::parse_pairing_relays(&req.relays).map_err(bad_request)?;
    let pairing = Pairing::start(PairingRequest::for_config(&state.config.nostr, relays))
        .map_err(bad_request)?;
    let uri = pairing.uri().to_string();
    let qr_svg = signer::qr_svg(&uri).map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    *state.pairing.lock().expect("pairing lock") = Some(pairing);
    Ok(Json(dto::PairingStartDto { uri, qr_svg }))
}

pub async fn pairing_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::PairingStatusDto>, ApiError> {
    ensure_can_pair(&state)?;
    Ok(Json(dto::pairing_status_dto(
        state.pairing_state().as_ref(),
    )))
}

pub async fn shutdown(State(state): State<Arc<AppState>>) -> Response {
    let body = Json(serde_json::json!({ "ok": true, "action": "stop" }));
    state.exit.stop();
    (StatusCode::ACCEPTED, body).into_response()
}

pub async fn restart(State(state): State<Arc<AppState>>) -> Response {
    let body = Json(serde_json::json!({ "ok": true, "action": "restart" }));
    state.exit.restart();
    (StatusCode::ACCEPTED, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::super::router;
    use super::super::test_support::*;
    use super::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use nostr_sdk::prelude::{Client, EventBuilder, FinalizeEvent, Keys, Kind, PublicKey, Tag};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::shutdown::ExitRequest;
    use crate::signer::{Pairing, PairingState, Signer};

    #[tokio::test]
    async fn config_endpoint_never_exposes_the_secret_key_value() {
        let (config, secret_hex) = test_config(true);
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
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
    async fn publish_sites_without_relay_reports_service_unavailable() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/api/publish/sites")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn status_without_ipfs_reports_service_unavailable() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/api/status")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
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
        let local = nostr_sdk::prelude::LocalRelay::new();
        local.run().await.unwrap();
        let url = local.url().await.to_string();
        let dir = tempfile::tempdir().unwrap();
        let (state, keys) = ready_state_with_relays(dir.path(), std::slice::from_ref(&url)).await;

        let mut tags = vec![Tag::identifier(state.config.nostr.mirror_set.clone())];
        tags.extend(
            (0..crate::nostr::budget::MAX_FOLLOW_SET_ENTRIES)
                .map(|_| Tag::public_key(Keys::generate().public_key())),
        );
        let full = EventBuilder::new(Kind::Custom(30000), "")
            .tags(tags)
            .finalize(&keys)
            .unwrap();
        let client = Client::default();
        client.add_relay(url.as_str()).await.unwrap();
        client.connect().await;
        client.send_event(&full).await.unwrap();

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

    fn test_state_with_exit(exit: ExitRequest) -> Arc<AppState> {
        let (config, secret_hex) = test_config(true);
        build_state(config, exit, test_keys(&secret_hex), TEST_TOKEN)
    }

    #[tokio::test]
    async fn shutdown_without_marker_header_is_forbidden() {
        let token = tokio_util::sync::CancellationToken::new();
        let app = router(test_state_with_exit(ExitRequest::new(token.clone())));
        let req = Request::builder()
            .method("POST")
            .uri("/api/shutdown")
            .header("Host", "127.0.0.1:8082")
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
        let req = Request::builder()
            .method("POST")
            .uri("/api/shutdown")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let json = error_body(resp).await;
        assert_eq!(json["ok"], true);
        assert_eq!(json["action"], "stop");
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn restart_with_marker_header_is_accepted_sets_the_flag_and_cancels() {
        let token = tokio_util::sync::CancellationToken::new();
        let exit = ExitRequest::new(token.clone());
        let app = router(test_state_with_exit(exit.clone()));
        let req = Request::builder()
            .method("POST")
            .uri("/api/restart")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let json = error_body(resp).await;
        assert_eq!(json["action"], "restart");
        assert!(token.is_cancelled());
        assert!(exit.restart_requested());
    }

    fn test_config_in_dir(
        dir: &std::path::Path,
        ui: bool,
        with_key: bool,
    ) -> (crate::config::Config, String) {
        let (mut config, secret_hex) = test_config(ui);
        config.config_path = dir.join("swing.toml");
        config.config_exists = false;
        if !with_key {
            config.nostr.secret_key = None;
        }
        (config, secret_hex)
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
        let req = Request::builder()
            .method("PUT")
            .uri("/api/config")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = call(app.clone(), req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = error_body(resp).await;
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

        let get_req = Request::builder()
            .uri("/api/config")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let get_resp = call(app, get_req).await;
        assert_eq!(get_resp.status(), StatusCode::OK);
        let get_json = error_body(get_resp).await;
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

    #[tokio::test]
    async fn post_setup_generates_a_key_and_schedules_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
        let state = build_state(config, test_exit(), None, TEST_TOKEN);
        assert!(state.setup_mode());
        let app = router(state);
        let body = serde_json::json!({ "secret_key": null, "items": {} }).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/api/setup")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = error_body(resp).await;
        assert_eq!(json["ok"], true);
        assert_eq!(json["restart"], true);
        assert!(json["npub"].as_str().unwrap().starts_with("npub1"));
        assert!(dir.path().join("swing.toml").exists());
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
    async fn setup_after_put_config_keeps_the_put_item() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        let put = put_config_body("policy.max_total_storage", "20GB");
        let (status, _) = send_json(app.clone(), "PUT", "/api/config", Some(put)).await;
        assert_eq!(status, StatusCode::OK);
        let body = serde_json::json!({ "secret_key": null, "items": {} });
        let (status, _) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::OK);

        let path = dir.path().join("swing.toml");
        let saved = crate::config::Config::load(Some(&path)).unwrap();
        assert_eq!(saved.policy.max_total_storage, 20 * (1u64 << 30));
        assert!(saved.nostr.secret_key.is_some());
    }

    #[tokio::test]
    async fn a_second_setup_before_the_restart_is_conflict_and_keeps_the_first_key() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        let body = serde_json::json!({ "secret_key": null, "items": {} });
        let (status, first) =
            send_json(app.clone(), "POST", "/api/setup", Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);

        let path = dir.path().join("swing.toml");
        let saved = crate::config::Config::load(Some(&path)).unwrap();
        let secret = saved.nostr.secret_key.unwrap();
        let keys = Keys::parse(secret.expose_secret()).unwrap();
        assert_eq!(first["npub"], crate::mirror::npub(&keys.public_key()));
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
    fn read_only_config_dir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        use std::os::unix::fs::PermissionsExt;

        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        // Root, or a filesystem that ignores the mode, can still write here; skip under those runners.
        if std::fs::write(locked.join("probe"), "").is_ok() {
            return None;
        }
        Some(locked)
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
        assert!(json["error"].as_str().unwrap().contains("swing.toml"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn setup_into_an_unwritable_config_directory_is_internal_error() {
        let dir = tempfile::tempdir().unwrap();
        let Some(locked) = read_only_config_dir(dir.path()) else {
            return;
        };
        let app = router(setup_mode_state(&locked));
        let body = serde_json::json!({ "secret_key": null, "items": {} });
        let (status, _) = send_json(app.clone(), "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let bad = serde_json::json!({ "secret_key": "not-a-key", "items": {} });
        let (status, _) = send_json(app, "POST", "/api/setup", Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    fn setup_mode_state(dir: &std::path::Path) -> Arc<AppState> {
        let (mut config, _secret_hex) = test_config_in_dir(dir, true, false);
        config.agent.state_dir = dir.join("data");
        build_state(config, test_exit(), None, TEST_TOKEN)
    }

    #[tokio::test]
    async fn signer_pairing_is_not_available_with_a_local_key() {
        let app = router(test_state());
        let body = serde_json::json!({ "relays": ["wss://relay.example"] });
        let (status, _) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = send_json(app, "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn starting_a_pairing_returns_a_qr_code_and_waits_for_the_signer() {
        let dir = tempfile::tempdir().unwrap();
        let state = setup_mode_state(dir.path());
        let app = router(Arc::clone(&state));
        let (status, json) = send_json(app.clone(), "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "idle");

        let body = serde_json::json!({ "relays": ["ws://127.0.0.1:1"] });
        let (status, json) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let uri = reqwest::Url::parse(json["uri"].as_str().unwrap()).unwrap();
        assert_eq!(uri.scheme(), "nostrconnect");
        let perms = uri
            .query_pairs()
            .find(|(k, _)| k == "perms")
            .map(|(_, v)| v.into_owned());
        assert_eq!(
            perms.as_deref(),
            Some("get_public_key,sign_event:35981,sign_event:35980,sign_event:30000")
        );
        assert!(json["qr_svg"].as_str().unwrap().contains("<svg"));

        let (status, json) = send_json(app, "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "waiting");
    }

    #[tokio::test]
    async fn invalid_signer_relays_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        for relays in [
            serde_json::json!([]),
            serde_json::json!(["https://relay.example"]),
            serde_json::json!([
                "wss://a", "wss://b", "wss://c", "wss://d", "wss://e", "wss://f"
            ]),
        ] {
            let body = serde_json::json!({ "relays": relays });
            let (status, _) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{relays}");
        }
    }

    fn remote_signer_file(user: PublicKey) -> crate::signer::RemoteSignerFile {
        crate::signer::RemoteSignerFile {
            app_secret_key: Keys::generate().secret_key().to_secret_hex(),
            signer_pubkey: Keys::generate().public_key().to_hex(),
            relays: vec!["wss://relay.example".to_string()],
            user_pubkey: user.to_hex(),
        }
    }

    fn remote_signer_state(dir: &std::path::Path, user: PublicKey) -> Arc<AppState> {
        let (mut config, _secret_hex) = test_config_in_dir(dir, true, false);
        config.agent.state_dir = dir.join("data");
        let remote = crate::signer::RemoteSigner::from_file(
            &remote_signer_file(user),
            Duration::from_secs(1),
        )
        .unwrap();
        build_state(
            config,
            test_exit(),
            Some(Signer::Remote(Arc::new(remote))),
            TEST_TOKEN,
        )
    }

    fn finish_pairing(state: &AppState, file: crate::signer::RemoteSignerFile, user: PublicKey) {
        *state.pairing.lock().unwrap() = Some(Pairing::finished(PairingState::Ready(Box::new(
            crate::signer::PairedSigner {
                file,
                user,
                probe_signed: true,
                probe_error: None,
            },
        ))));
    }

    #[tokio::test]
    async fn a_running_signer_app_user_can_pair_again() {
        let dir = tempfile::tempdir().unwrap();
        let user = Keys::generate().public_key();
        let state = remote_signer_state(dir.path(), user);
        let app = router(Arc::clone(&state));
        let (status, json) = send_json(app.clone(), "GET", "/api/overview", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["signer"]["remote"], true);
        assert_eq!(
            json["signer"]["relays"],
            serde_json::json!(["wss://relay.example"])
        );

        let body = serde_json::json!({ "relays": ["ws://127.0.0.1:1"] });
        let (status, _) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send_json(app.clone(), "POST", "/api/signer/reconnect", None).await;
        assert_eq!(status, StatusCode::CONFLICT);

        let file = remote_signer_file(user);
        finish_pairing(&state, file.clone(), user);
        let (status, json) = send_json(app, "POST", "/api/signer/reconnect", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["restart"], true);
        let saved = crate::signer::RemoteSignerFile::load(&state.config.agent.state_dir)
            .unwrap()
            .unwrap();
        assert_eq!(saved, file);
        assert!(state.pairing_state().is_none());
    }

    #[tokio::test]
    async fn reconnecting_a_different_account_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let user = Keys::generate().public_key();
        let state = remote_signer_state(dir.path(), user);
        let other = Keys::generate().public_key();
        finish_pairing(&state, remote_signer_file(other), other);
        let (status, json) = send_json(
            router(Arc::clone(&state)),
            "POST",
            "/api/signer/reconnect",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .contains("same Nostr account")
        );
        assert!(
            crate::signer::RemoteSignerFile::load(&state.config.agent.state_dir)
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn reconnecting_needs_a_signer_app() {
        let (status, _) =
            send_json(router(test_state()), "POST", "/api/signer/reconnect", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn setup_with_a_signer_needs_a_finished_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        let body = serde_json::json!({ "remote_signer": true, "items": {} });
        let (status, _) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn setup_with_a_paired_signer_saves_it_without_a_secret_key() {
        let dir = tempfile::tempdir().unwrap();
        let state = setup_mode_state(dir.path());
        let user = Keys::generate().public_key();
        let file = crate::signer::RemoteSignerFile {
            app_secret_key: Keys::generate().secret_key().to_secret_hex(),
            signer_pubkey: Keys::generate().public_key().to_hex(),
            relays: vec!["wss://relay.example".to_string()],
            user_pubkey: user.to_hex(),
        };
        *state.pairing.lock().unwrap() = Some(Pairing::finished(PairingState::Ready(Box::new(
            crate::signer::PairedSigner {
                file: file.clone(),
                user,
                probe_signed: true,
                probe_error: None,
            },
        ))));
        let app = router(Arc::clone(&state));
        let (status, json) = send_json(app.clone(), "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "ready");
        assert_eq!(json["probe_signed"], true);

        let body = serde_json::json!({ "remote_signer": true, "items": {} });
        let (status, json) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["npub"], crate::mirror::npub(&user));
        let saved = crate::signer::RemoteSignerFile::load(&state.config.agent.state_dir)
            .unwrap()
            .unwrap();
        assert_eq!(saved, file);
        let written = std::fs::read_to_string(dir.path().join("swing.toml")).unwrap();
        assert!(!written.contains("secret_key"));
        assert!(state.pairing_state().is_none());
    }

    #[tokio::test]
    async fn post_setup_when_already_configured_is_conflict() {
        let app = router(test_state());
        let body = serde_json::json!({ "secret_key": null, "items": {} }).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/api/setup")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn setup_mode_relay_endpoints_report_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let (config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
        let state = build_state(config, test_exit(), None, TEST_TOKEN);
        let app = router(state);
        let req = Request::builder()
            .uri("/api/sites")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json = error_body(resp).await;
        assert_eq!(json["error"], "agent is not configured");
    }

    #[tokio::test]
    async fn overview_reports_setup_mode_with_null_identity() {
        let dir = tempfile::tempdir().unwrap();
        let (config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
        let state = build_state(config, test_exit(), None, TEST_TOKEN);
        let app = router(state);
        let req = Request::builder()
            .uri("/api/overview")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = error_body(resp).await;
        assert_eq!(json["setup"], true);
        assert!(json["pubkey"].is_null());
        assert!(json["npub"].is_null());
        assert!(json["signer"].is_null());
    }

    #[tokio::test]
    async fn overview_tells_whether_a_signer_app_signs() {
        let (_status, json) = send_json(router(test_state()), "GET", "/api/overview", None).await;
        assert_eq!(json["signer"]["remote"], false);
        assert!(json["signer"]["last_failure"].is_null());
    }
}
