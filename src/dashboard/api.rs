use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRequest, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nostr_sdk::prelude::*;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config;
use crate::health;
use crate::mfs::MfsLayout;
use crate::mirror;
use crate::nip05;
use crate::publish;
use crate::replicas;
use crate::webring;

use super::AppState;
use super::dto;

const MAX_KEYS: usize = 100;

pub enum ApiError {
    BadRequest(String),
    PayloadTooLarge(String),
    RelayUnavailable,
    Upstream(String),
    Conflict(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::PayloadTooLarge(msg) => (StatusCode::PAYLOAD_TOO_LARGE, msg),
            ApiError::RelayUnavailable => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "relay is not available".to_string(),
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
        pubkey: pubkey.to_hex(),
        npub: mirror::npub(&pubkey),
        relays: state.config.nostr.relays.clone(),
        mirror_set: state.config.nostr.mirror_set.clone(),
        gateway: state.config.dashboard.gateway.clone(),
        started_at: state.started_at,
        max_upload: state.config.dashboard.max_upload,
    }))
}

pub async fn sites(State(state): State<Arc<AppState>>) -> Result<Json<dto::SitesDto>, ApiError> {
    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let view = mirror::collect_sites(relay, &state.config)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::sites_dto(
        &view,
        state.config.dashboard.gateway.as_deref(),
    )))
}

pub async fn status(State(state): State<Arc<AppState>>) -> Result<Json<dto::StatusDto>, ApiError> {
    let report = health::collect_status(&state.ipfs, &state.config)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::status_dto(&report)))
}

pub async fn mirror_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::MirrorListDto>, ApiError> {
    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let view = mirror::collect_mirror_list(relay, &state.config)
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

pub async fn mirror_add(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<MirrorKeysRequest>,
) -> Result<Json<dto::MirrorChangeDto>, ApiError> {
    validate_keys(&req.keys)?;
    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let change = mirror::apply_add(relay, &state.config, &req.keys)
        .await
        .map_err(upstream)?;
    finish_mirror_change(&state, change)
}

pub async fn mirror_remove(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<MirrorKeysRequest>,
) -> Result<Json<dto::MirrorChangeDto>, ApiError> {
    validate_keys(&req.keys)?;
    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let change = mirror::apply_remove(relay, &state.config, &req.keys)
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
    let roots = if root_inputs.is_empty() {
        vec![state.own_pubkey]
    } else {
        mirror::parse_pubkey_inputs(&root_inputs)
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?
    };

    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let view = webring::collect(relay, &state.config, &roots, depth)
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
    let authors = if key_inputs.is_empty() {
        vec![state.own_pubkey]
    } else {
        mirror::parse_pubkey_inputs(&key_inputs)
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?
    };

    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let authors_data = replicas::collect(relay, &state.config, &authors)
        .await
        .map_err(upstream)?;
    Ok(Json(dto::replicas_dto(&authors_data)))
}

pub(super) struct PublishFields {
    pub site: String,
    pub url: Option<String>,
    pub message: Option<String>,
    pub nip05: Option<String>,
}

pub(super) enum PublishOutcome {
    Success(dto::PublishResultDto),
    Nip05Failed(Response),
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
    let nip05_mode =
        publish::resolve_nip05_mode(fields.nip05.as_deref(), state.config.publish.nip05)
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;

    let Ok(_permit) = state.publish_lock.try_lock() else {
        return Err(ApiError::Conflict(
            "a publish is already running".to_string(),
        ));
    };

    let pubkey_hex = state.own_pubkey.to_hex();

    let nip05_dto = if nip05_mode != config::Nip05Mode::Off {
        let verifier = nip05::HttpNip05Verifier::public_only();
        let outcome = publish::check_nip05(&verifier, nip05_mode, &fields.site, &pubkey_hex).await;
        if let Some(abort) = outcome.abort {
            let body = serde_json::json!({
                "error": abort,
                "nip05": dto::nip05_result_dto(&outcome.result),
            });
            return Ok(PublishOutcome::Nip05Failed(
                (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response(),
            ));
        }
        dto::nip05_result_dto(&outcome.result)
    } else {
        dto::nip05_off_dto()
    };

    let layout = MfsLayout::new(state.config.ipfs.mfs_root.clone());
    let created_at = Timestamp::now();
    let stage = publish::add_and_measure(
        &state.ipfs,
        &layout,
        &pubkey_hex,
        &fields.site,
        created_at.as_secs(),
        dir,
    )
    .await
    .map_err(upstream)?;

    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let relay_results = publish::sign_and_send(
        relay,
        &publish::SiteAnnouncement {
            site_event_kind: state.config.nostr.site_event_kind,
            d: &fields.site,
            cid: &stage.cid,
            url: fields.url.as_deref(),
            size: stage.size,
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
    let prune = publish::prune_old_versions_collect(
        &state.ipfs,
        &site_path,
        state.config.publish.keep_versions,
    )
    .await;

    let gateway_url = dto::gateway_url(state.config.dashboard.gateway.as_deref(), &stage.cid, true);

    Ok(PublishOutcome::Success(dto::PublishResultDto {
        site: fields.site,
        url: fields.url,
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
    let relay = state.relay.as_ref().ok_or(ApiError::RelayUnavailable)?;
    let events = relay
        .fetch_site_events(state.config.nostr.site_event_kind, &[state.own_pubkey])
        .await
        .map_err(upstream)?;
    let parsed: Vec<crate::nostr::SiteEvent> = events
        .iter()
        .filter_map(|e| crate::nostr::parse_site_event(e, state.config.nostr.site_event_kind).ok())
        .collect();
    let latest = crate::nostr::select_latest(&parsed);
    let mut sites: Vec<&crate::nostr::SiteEvent> = latest.values().collect();
    sites.sort_by(|a, b| a.d.cmp(&b.d));
    Ok(Json(dto::publish_sites_dto(
        &sites,
        state.config.dashboard.gateway.as_deref(),
    )))
}

pub async fn config(State(state): State<Arc<AppState>>) -> Json<dto::ConfigDto> {
    Json(dto::config_dto(&state.config))
}
