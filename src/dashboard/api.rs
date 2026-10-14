use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nostr_sdk::prelude::*;
use serde::Deserialize;

use crate::fingerprint::fingerprint;
use crate::health;
use crate::mirror;
use crate::publish;
use crate::replicas;
use crate::settings;
use crate::webring;

use super::AppState;
use super::dto;
use super::error::{ApiError, AppJson, blocking, internal, settings_error, upstream};

mod network;

pub use network::network;

const MAX_KEYS: usize = 100;
const RELAY_QUERY_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

fn not_ready(state: &AppState) -> ApiError {
    if state.setup_mode() {
        ApiError::NotConfigured
    } else {
        ApiError::NotReady
    }
}

impl AppState {
    pub(super) async fn require_relay(&self) -> Result<Arc<crate::nostr::RelayClient>, ApiError> {
        self.relay().await.ok_or_else(|| not_ready(self))
    }

    pub(super) async fn relay_query_permit(
        &self,
    ) -> Result<tokio::sync::SemaphorePermit<'_>, ApiError> {
        acquire_within(&self.relay_queries, RELAY_QUERY_WAIT).await
    }

    pub(super) async fn require_ipfs(&self) -> Result<crate::ipfs::IpfsClient, ApiError> {
        self.ipfs().await.ok_or_else(|| not_ready(self))
    }

    pub(super) fn require_own_pubkey(&self) -> Result<PublicKey, ApiError> {
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

fn query_all<'a>(
    params: &'a [(String, String)],
    key: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    params
        .iter()
        .filter(move |(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn parse_query_number<T: std::str::FromStr>(
    params: &[(String, String)],
    key: &str,
    default: T,
) -> Result<T, ApiError> {
    query_all(params, key).try_fold(default, |_, value| {
        value
            .parse()
            .map_err(|_| ApiError::BadRequest(format!("invalid {key}: {value}")))
    })
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
        fingerprint: pubkey.map(|pk| fingerprint(&pk)),
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
    let since = parse_query_number(&params, "since", 0)?;
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

fn validate_key_count(keys: &[String]) -> Result<(), ApiError> {
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
    Ok(())
}

fn validate_keys(keys: &[String]) -> Result<(), ApiError> {
    validate_key_count(keys)?;
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
    let _writes = state.locks.mirror_writes().await;
    let relay = state.require_relay().await?;
    let change = mirror::apply_add(&relay, &state.config, &req.keys)
        .await
        .map_err(mirror_add_error)?;
    finish_mirror_change(&state, change)
}

pub async fn mirror_preview(
    AppJson(req): AppJson<MirrorKeysRequest>,
) -> Result<Json<dto::KeyPreviewsDto>, ApiError> {
    validate_key_count(&req.keys)?;
    Ok(Json(dto::key_previews_dto(&req.keys)))
}

pub async fn mirror_remove(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<MirrorKeysRequest>,
) -> Result<Json<dto::MirrorChangeDto>, ApiError> {
    validate_keys(&req.keys)?;
    let _writes = state.locks.mirror_writes().await;
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
    let depth = parse_query_number(&params, "depth", DEFAULT_WEBRING_DEPTH)?;
    let root_inputs: Vec<String> = query_all(&params, "root").map(str::to_string).collect();
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
    let key_inputs: Vec<String> = query_all(&params, "key").map(str::to_string).collect();
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

pub async fn publish_previous_files(
    State(state): State<Arc<AppState>>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<dto::PreviousFilesDto>, ApiError> {
    let site = query_all(&params, "site")
        .next()
        .ok_or_else(|| ApiError::BadRequest("missing site".to_string()))?
        .to_string();
    crate::nostr::validate_d_tag(&site)
        .map_err(|e| ApiError::BadRequest(format!("invalid site: {e:#}")))?;
    let previous = {
        let _permit = state.relay_query_permit().await?;
        let relay = state.require_relay().await?;
        relay
            .fetch_own_latest_site(state.config.nostr.site_event_kind, &site)
            .await
            .map_err(|e| format!("{e:#}"))
    };
    let ipfs = state.require_ipfs().await?;
    let files = publish::PreviousFiles::load(&ipfs, &previous).await;
    Ok(Json(dto::previous_files_dto(files)))
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

pub async fn update_config(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<UpdateConfigRequest>,
) -> Result<Json<dto::ConfigDto>, ApiError> {
    let _writes = state.locks.config_writes().await;
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
mod tests;
