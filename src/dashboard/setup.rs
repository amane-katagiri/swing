use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::mirror;
use crate::settings;
use crate::signer::{self, Pairing, PairingRequest, PairingState, Signer};

use super::AppState;
use super::dto;
use super::error::{ApiError, AppJson, blocking, internal, settings_error};

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    secret_key: Option<Zeroizing<String>>,
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
    let mut writes = state.locks.config_writes().await;
    if writes.setup_done {
        return Err(ApiError::Conflict(
            "setup is already done; swing is restarting".to_string(),
        ));
    }
    let config = Arc::clone(&state.config);
    let items = req.items;
    let pubkey = if req.remote_signer {
        let paired = ready_pairing(&state)?;
        let user = blocking(move || {
            settings::setup(&config, None, &items).map_err(settings_error)?;
            paired
                .file
                .save(&config.agent.state_dir)
                .map_err(|e| internal("saving the signer app connection failed", e))?;
            Ok(paired.user)
        })
        .await?;
        *state.pairing.lock().expect("pairing lock") = None;
        user
    } else {
        let keys = settings::setup_keys(req.secret_key.as_ref().map(|s| s.as_str()))
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
        blocking(move || {
            settings::setup(&config, Some(&keys), &items).map_err(settings_error)?;
            Ok(keys.public_key())
        })
        .await?
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
    let writes = state.locks.config_writes().await;
    let state_dir = state.config.agent.state_dir.clone();
    blocking(move || {
        paired
            .file
            .save(&state_dir)
            .map_err(|e| internal("saving the signer app connection failed", e))
    })
    .await?;
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
    let qr_svg = signer::qr_svg(&uri).map_err(|e| internal("drawing the QR code failed", e))?;
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

#[cfg(test)]
mod tests;
