use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth;

use super::AppState;
use super::api::{ApiError, AppJson, internal};
use super::dto;
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
mod tests {
    use super::super::router;
    use super::super::test_support::*;
    use super::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;

    fn get(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap()
    }

    fn dashboard_get(uri: &str) -> Request<Body> {
        let mut req = get(uri);
        req.headers_mut()
            .insert("x-swing-dashboard", "1".parse().unwrap());
        req
    }

    fn post_json(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn set_cookie(resp: &axum::http::Response<Body>) -> Option<String> {
        resp.headers()
            .get("set-cookie")
            .map(|v| v.to_str().unwrap().to_string())
    }

    fn cookie_pair(set_cookie: &str) -> String {
        set_cookie.split(';').next().unwrap().to_string()
    }

    async fn issue_code(state: &Arc<AppState>) -> String {
        let resp = call(router(Arc::clone(state)), post_json("/api/login-code", "")).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["expires_in"], 300);
        json["code"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn api_requires_authentication_but_static_ui_does_not() {
        let state = test_state();
        let resp = call_anonymous(router(Arc::clone(&state)), dashboard_get("/api/overview")).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
        let resp = call_anonymous(router(Arc::clone(&state)), get("/")).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let mut req = get("/api/overview");
        req.headers_mut()
            .insert("authorization", "Bearer wrong".parse().unwrap());
        let resp = call_anonymous(router(state), req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn cookie_authenticated_reads_need_the_dashboard_marker() {
        let state = test_state();
        let session = crate::auth::new_session(TEST_TOKEN, crate::auth::DASHBOARD_SESSION);
        let cookie = format!("swing_session_8082={session}");

        for method in ["GET", "HEAD"] {
            let mut req = get("/api/overview");
            *req.method_mut() = method.parse().unwrap();
            req.headers_mut().insert("cookie", cookie.parse().unwrap());
            let resp = call_anonymous(router(Arc::clone(&state)), req).await;
            assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method}");
        }

        let mut req = dashboard_get("/api/overview");
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        req.headers_mut()
            .insert("origin", "http://127.0.0.1:8080".parse().unwrap());
        let resp = call_anonymous(router(Arc::clone(&state)), req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        let mut req = dashboard_get("/api/overview");
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let resp = call_anonymous(router(Arc::clone(&state)), req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = call(router(Arc::clone(&state)), get("/api/overview")).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = call_anonymous(router(state), get("/")).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn login_code_exchanges_for_a_session_cookie_once() {
        let state = test_state();
        let code = issue_code(&state).await;
        let body = format!("{{\"code\":\"{code}\"}}");

        let resp = call_anonymous(router(Arc::clone(&state)), post_json("/api/login", &body)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = set_cookie(&resp).unwrap();
        assert!(cookie.starts_with("swing_session_8082="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Max-Age=2592000"));

        let mut req = dashboard_get("/api/overview");
        req.headers_mut()
            .insert("cookie", cookie_pair(&cookie).parse().unwrap());
        let resp = call_anonymous(router(Arc::clone(&state)), req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = call_anonymous(router(state), post_json("/api/login", &body)).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(set_cookie(&resp).is_none());
    }

    #[tokio::test]
    async fn login_link_hands_the_code_to_the_page_without_redeeming_it() {
        let state = test_state();
        let code = issue_code(&state).await;
        let resp = call_anonymous(
            router(Arc::clone(&state)),
            get(&format!("/login?code={code}")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            resp.headers().get("location").unwrap(),
            &format!("/#/login/code/{code}")
        );
        assert!(set_cookie(&resp).is_none());

        let resp = call_anonymous(
            router(Arc::clone(&state)),
            post_json("/api/login", &format!("{{\"code\":\"{code}\"}}")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = call_anonymous(router(state), get("/login?code=%2F%2Fevil")).await;
        assert_eq!(resp.headers().get("location").unwrap(), "/#/login/invalid");
    }

    #[tokio::test]
    async fn identity_proves_knowledge_of_the_token_without_authentication() {
        let state = test_state();
        let nonce = crate::auth::new_identity_nonce();
        let resp = call_anonymous(
            router(Arc::clone(&state)),
            post_json("/api/identity", &format!("{{\"nonce\":\"{nonce}\"}}")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(crate::auth::verify_identity_proof(
            TEST_TOKEN,
            &nonce,
            json["proof"].as_str().unwrap()
        ));
        assert_eq!(json["instance"], state.instance.as_str());

        let resp = call_anonymous(
            router(state),
            post_json("/api/identity", "{\"nonce\":\"abc\"}"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rotating_the_token_invalidates_the_old_token_and_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 1 << 20);
        let session = crate::auth::new_session(TEST_TOKEN, crate::auth::DASHBOARD_SESSION);

        let resp = call(
            router(Arc::clone(&state)),
            post_json("/api/token/rotate", ""),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let rotated = crate::auth::read_token(dir.path()).unwrap().unwrap();
        assert_ne!(rotated, TEST_TOKEN);

        let resp = call(router(Arc::clone(&state)), get("/api/overview")).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let mut req = dashboard_get("/api/overview");
        req.headers_mut().insert(
            "cookie",
            format!("swing_session_8082={session}").parse().unwrap(),
        );
        let resp = call_anonymous(router(Arc::clone(&state)), req).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let mut req = get("/api/overview");
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {rotated}").parse().unwrap(),
        );
        let resp = call_anonymous(router(state), req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    async fn login_cookie(state: &Arc<AppState>, forwarded_proto: Option<&str>) -> String {
        let code = issue_code(state).await;
        let mut req = post_json("/api/login", &format!("{{\"code\":\"{code}\"}}"));
        if let Some(proto) = forwarded_proto {
            req.headers_mut()
                .insert("x-forwarded-proto", proto.parse().unwrap());
        }
        let resp = call_anonymous(router(Arc::clone(state)), req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        set_cookie(&resp).unwrap()
    }

    #[tokio::test]
    async fn session_cookie_is_secure_behind_an_https_proxy() {
        let state = test_state();
        assert!(!login_cookie(&state, None).await.contains("Secure"));
        assert!(!login_cookie(&state, Some("http")).await.contains("Secure"));
        assert!(
            login_cookie(&state, Some("HTTPS"))
                .await
                .ends_with("; Secure")
        );
        assert!(
            login_cookie(&state, Some("https, http"))
                .await
                .ends_with("; Secure")
        );
    }

    #[tokio::test]
    async fn session_cookie_is_secure_when_public_url_is_https() {
        let (mut config, secret_hex) = test_config(true);
        config.dashboard.public_url = Some("https://dash.example".to_string());
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);
        assert!(login_cookie(&state, None).await.ends_with("; Secure"));
    }
}
