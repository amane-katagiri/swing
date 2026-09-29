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
use crate::webring;

use super::AppState;
use super::dto;

const MAX_KEYS: usize = 100;
const RELAY_QUERY_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

pub enum ApiError {
    BadRequest(String),
    Unauthorized(String),
    PayloadTooLarge(String),
    NotReady,
    NotConfigured,
    Busy,
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
            ApiError::Busy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "too many relay queries are running; try again later".to_string(),
            ),
            ApiError::Upstream(msg) => (StatusCode::BAD_GATEWAY, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::Internal(detail) => {
                error!(error = %detail, "dashboard request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error; see the swing log for details".to_string(),
                )
            }
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

    async fn relay_query_permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, ApiError> {
        acquire_within(&self.relay_queries, RELAY_QUERY_WAIT).await
    }

    async fn require_ipfs(&self) -> Result<crate::ipfs::IpfsClient, ApiError> {
        self.ipfs().await.ok_or_else(|| not_ready(self))
    }

    fn require_own_pubkey(&self) -> Result<PublicKey, ApiError> {
        self.own_pubkey.ok_or_else(|| not_ready(self))
    }
}

async fn acquire_within(
    semaphore: &tokio::sync::Semaphore,
    wait: std::time::Duration,
) -> Result<tokio::sync::SemaphorePermit<'_>, ApiError> {
    match tokio::time::timeout(wait, semaphore.acquire()).await {
        Ok(Ok(permit)) => Ok(permit),
        _ => Err(ApiError::Busy),
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

pub async fn activity(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::ActivityDto>, ApiError> {
    let saved = mirror::load_state(&state.config)
        .await
        .map_err(|e| internal("reading state.json", e))?;
    Ok(Json(dto::ActivityDto {
        latest_stored_at: saved.latest_stored_at(),
        latest_published_at: state.activity.latest_published_at(),
        latest_replica_report_at: state.activity.latest_replica_report_at(),
    }))
}

pub async fn stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<dto::StatsDto>, ApiError> {
    let mut since = 0;
    for (key, value) in &params {
        if key == "since" {
            since = value
                .parse()
                .map_err(|_| ApiError::BadRequest(format!("invalid since: {value}")))?;
        }
    }
    Ok(Json(dto::StatsDto {
        interval: crate::stats::SAMPLE_INTERVAL.as_secs(),
        kubo_managed: state.config.kubo.managed,
        samples: state.stats.since(since),
    }))
}

pub async fn sites(State(state): State<Arc<AppState>>) -> Result<Json<dto::SitesDto>, ApiError> {
    let _permit = state.relay_query_permit().await?;
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
    let _permit = state.relay_query_permit().await?;
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
    let _permit = state.relay_query_permit().await?;
    let relay = state.require_relay().await?;
    let roots = if root_inputs.is_empty() {
        vec![state.require_own_pubkey()?]
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
    let _permit = state.relay_query_permit().await?;
    let relay = state.require_relay().await?;
    let authors = if key_inputs.is_empty() {
        vec![state.require_own_pubkey()?]
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
    pub modes: publish::ModeOverrides,
}

pub(super) enum PublishOutcome {
    Success(Box<dto::PublishResultDto>),
    CheckFailed(Response),
}

fn check_failed(error: String, body: serde_json::Value) -> Response {
    let mut body = body;
    body["error"] = serde_json::Value::String(error);
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

async fn run_publish_nip05(
    nip05_mode: config::CheckMode,
    site: &str,
    pubkey_hex: &str,
) -> Result<dto::Nip05ResultDto, Response> {
    if nip05_mode == config::CheckMode::Off {
        return Ok(dto::nip05_off_dto());
    }
    let verifier = nip05::HttpNip05Verifier::public_only();
    let outcome = publish::check_nip05(&verifier, nip05_mode, site, pubkey_hex).await;
    if let Some(abort) = outcome.abort {
        let body = serde_json::json!({ "nip05": dto::nip05_result_dto(&outcome.result) });
        return Err(check_failed(abort, body));
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
    })?;
    let title = publish::normalize_title(fields.title.as_deref()).map_err(|_| {
        ApiError::BadRequest(
            "invalid title: must not exceed 256 bytes and must not contain control characters"
                .to_string(),
        )
    })?;
    let modes =
        publish::resolve_modes(&fields.modes, &state.config.publish).map_err(|(name, e)| {
            ApiError::BadRequest(format!("invalid {}: {e:#}", name.replace('-', "_")))
        })?;
    publish::refuse_protected_paths(dir, &state.config)
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;

    let Ok(_permit) = state.publish_lock.try_lock() else {
        return Err(ApiError::Conflict(
            "a publish is already running".to_string(),
        ));
    };

    let pubkey_hex = state.require_own_pubkey()?.to_hex();

    let nip05_dto = match run_publish_nip05(modes.nip05, &fields.site, &pubkey_hex).await {
        Ok(dto) => dto,
        Err(resp) => return Ok(PublishOutcome::CheckFailed(resp)),
    };

    let local = publish::LocalChecks::run(
        dir,
        modes.check_dotfiles,
        modes.check_size,
        &state.config.publish.dotfiles_allow,
    )
    .await
    .map_err(|e| internal("listing the uploaded files failed", e))?;
    if let Some(abort) = local.abort_message() {
        let body = serde_json::json!({
            "nip05": nip05_dto,
            "checks": dto::publish_checks_dto(&local, None),
        });
        return Ok(PublishOutcome::CheckFailed(check_failed(abort, body)));
    }

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
    let unchanged = publish::check_unchanged(
        &relay,
        modes.check_unchanged,
        state.config.nostr.site_event_kind,
        &fields.site,
        &stage.cid,
    )
    .await;
    let gateway_url = dto::gateway_url(state.config.dashboard.gateway.as_deref(), &stage.cid, true);
    let checks = dto::publish_checks_dto(&local, Some(&unchanged));

    if unchanged.stops_publish() {
        ipfs.mfs_remove(&stage.path)
            .await
            .map_err(|e| upstream(e.context(format!("could not remove {}", stage.path))))?;
        return Ok(PublishOutcome::Success(Box::new(dto::PublishResultDto {
            published: false,
            site: fields.site,
            url: fields.url,
            title: title.map(str::to_string),
            message: fields.message,
            nip05: nip05_dto,
            checks,
            cid: stage.cid,
            size: stage.size,
            created_at: None,
            mfs_path: None,
            relays: Vec::new(),
            pruned: Vec::new(),
            prune_error: None,
            gateway_url,
        })));
    }

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
    state.activity.record_published(created_at.as_secs());

    let site_path = layout.publish_site(&pubkey_hex, &fields.site);
    let prune =
        publish::prune_old_versions_collect(&ipfs, &site_path, state.config.publish.keep_versions)
            .await;

    Ok(PublishOutcome::Success(Box::new(dto::PublishResultDto {
        published: true,
        site: fields.site,
        url: fields.url,
        title: title.map(str::to_string),
        message: fields.message,
        nip05: nip05_dto,
        checks,
        cid: stage.cid,
        size: stage.size,
        created_at: Some(created_at.as_secs()),
        mfs_path: Some(stage.path),
        relays: relay_results
            .iter()
            .map(dto::RelayResultDto::from)
            .collect(),
        pruned: prune.pruned().into_iter().map(str::to_string).collect(),
        prune_error: prune.error_summary(),
        gateway_url,
    })))
}

pub async fn publish_sites(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::PublishSitesDto>, ApiError> {
    let _permit = state.relay_query_permit().await?;
    let relay = state.require_relay().await?;
    let own_pubkey = state.require_own_pubkey()?;
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

pub(super) fn internal(context: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(format!("{context}: {e:#}"))
}

pub(super) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| internal("a background task failed", e))?
}

pub(super) fn settings_error(e: settings::EditError) -> ApiError {
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
    let config = Arc::clone(&state.config);
    let updated =
        blocking(move || settings::update(&config, &req.items).map_err(settings_error)).await?;
    state.restart_required.store(true, Ordering::SeqCst);
    let dto = dto::config_dto(&updated, true);
    state.set_display_config(updated).await;
    Ok(Json(dto))
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
    use nostr_sdk::prelude::{Client, EventBuilder, FinalizeEvent, Keys, Kind, Tag};
    use std::sync::Arc;

    use crate::shutdown::ExitRequest;

    #[tokio::test]
    async fn internal_errors_do_not_reach_the_client() {
        use axum::response::IntoResponse;
        let resp = super::internal(
            "saving the config file failed",
            "/home/someone/.config/swing/swing.toml: Permission denied",
        )
        .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(!body.contains("someone"), "{body}");
        assert!(!body.contains("Permission denied"), "{body}");
    }

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
    async fn activity_reads_the_newest_stored_at_without_the_agent() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
        config.agent.state_dir = dir.path().join("state");
        let state_path = config.agent.state_dir.join("state.json");
        let state = build_state(config, test_exit(), None, TEST_TOKEN);

        let (status, json) =
            send_json(router(Arc::clone(&state)), "GET", "/api/activity", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["latest_stored_at"].is_null());
        assert!(json["latest_published_at"].is_null());
        assert!(json["latest_replica_report_at"].is_null());

        let mut saved = crate::state::State::default();
        for (d, stored_at) in [("x.example", 20), ("y.example", 50)] {
            saved.apply_store(
                &crate::state::site_key("aa", d),
                crate::state::VersionRecord {
                    cid: format!("bafy{d}"),
                    size: 1,
                    created_at: 1,
                    stored_at,
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
}
