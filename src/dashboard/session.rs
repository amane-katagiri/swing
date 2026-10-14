use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth;

use super::AppState;
use super::dto;
use super::error::{ApiError, AppJson, internal};
use super::guard;

#[derive(Deserialize)]
pub struct LoginRequest {
    code: String,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    code: Option<String>,
}

// Trusting a spoofed X-Forwarded-Proto is harmless: it only affects the requester's own cookie.
fn served_over_https(state: &AppState, headers: &HeaderMap) -> bool {
    let forwarded_https = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .is_some_and(|proto| proto.trim().eq_ignore_ascii_case("https"));
    let public_https = state
        .config
        .dashboard
        .public_url
        .as_deref()
        .is_some_and(|url| url.starts_with("https://"));
    forwarded_https || public_https
}

fn session_cookie(state: &AppState, headers: &HeaderMap) -> HeaderValue {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let secure = if served_over_https(state, headers) {
        "; Secure"
    } else {
        ""
    };
    let value = format!(
        "{}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{secure}",
        guard::session_cookie_name(host),
        auth::new_session(&state.token(), auth::DASHBOARD_SESSION),
        auth::SESSION_TTL.as_secs()
    );
    HeaderValue::from_str(&value).expect("session cookie is ASCII")
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AppJson(req): AppJson<LoginRequest>,
) -> Result<Response, ApiError> {
    if !state.login_codes.redeem(&req.code) {
        return Err(ApiError::Unauthorized(
            "invalid or expired login code".to_string(),
        ));
    }
    let cookie = session_cookie(&state, &headers);
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response())
}

// Redeeming on GET would let link previewers burn the code; the page POSTs it to /api/login instead.
pub async fn login_page(Query(query): Query<LoginQuery>) -> Redirect {
    match query.code {
        Some(code) if !code.is_empty() && code.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Redirect::to(&format!("/#/login/code/{code}"))
        }
        Some(_) => Redirect::to("/#/login/invalid"),
        None => Redirect::to("/#/login"),
    }
}

pub async fn identity(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<dto::IdentityRequestDto>,
) -> Result<Json<dto::IdentityDto>, ApiError> {
    if !auth::is_identity_nonce(&req.nonce) {
        return Err(ApiError::BadRequest(format!(
            "nonce must be {} hex characters",
            auth::IDENTITY_NONCE_BYTES * 2
        )));
    }
    Ok(Json(dto::IdentityDto {
        proof: auth::identity_proof(&state.token(), &req.nonce),
        instance: state.instance.clone(),
    }))
}

pub async fn login_code(State(state): State<Arc<AppState>>) -> Json<dto::LoginCodeDto> {
    Json(dto::LoginCodeDto {
        code: state.login_codes.issue(),
        expires_in: auth::LOGIN_CODE_TTL.as_secs(),
    })
}

pub async fn rotate_token(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _rotation = state.locks.token_rotation().await;
    let state_dir = state.config.agent.state_dir.clone();
    let token = tokio::task::spawn_blocking(move || auth::write_new_token(&state_dir))
        .await
        .map_err(|e| internal("rotating the dashboard token failed", e))?
        .map_err(|e| internal("rotating the dashboard token failed", e))?;
    state.set_token(token);
    state.login_codes.clear();
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests;
