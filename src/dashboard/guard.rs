use axum::Json;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

use super::AppState;
use crate::auth;
use crate::host::{extract_host, split_host_port};

const DASHBOARD_MARKER_HEADER: &str = "x-swing-dashboard";
const SESSION_COOKIE_PREFIX: &str = "swing_session";
const UNAUTHENTICATED_API_PATHS: &[&str] = &["/api/login", "/api/identity"];

// Cookies ignore the port, so instances on one host would otherwise overwrite each other's session.
pub fn session_cookie_name(host_header: &str) -> String {
    match split_host_port(host_header).1 {
        Some(port) => format!("{SESSION_COOKIE_PREFIX}_{port}"),
        None => SESSION_COOKIE_PREFIX.to_string(),
    }
}

fn cookie_values<'a>(headers: &'a HeaderMap, name: &'a str) -> impl Iterator<Item = &'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(move |pair| {
            let (k, v) = pair.trim().split_once('=')?;
            (k == name).then_some(v)
        })
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

pub fn authorized(headers: &HeaderMap, host_header: &str, token: &str) -> bool {
    if let Some(presented) = bearer_token(headers) {
        return auth::token_matches(token, presented.trim());
    }
    let name = session_cookie_name(host_header);
    cookie_values(headers, &name).any(|v| auth::verify_session(token, auth::DASHBOARD_SESSION, v))
}

pub fn host_allowed(host_header: &str, allowed_hosts: &[String]) -> bool {
    let host = extract_host(host_header);
    crate::host::is_loopback_name(&host)
        || allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host))
}

pub fn origin_matches_host(origin_header: &str, host_header: &str) -> bool {
    let authority = origin_header
        .split_once("://")
        .map_or(origin_header, |(_, rest)| rest);
    let authority = authority.trim_end_matches('/');
    authority.eq_ignore_ascii_case(host_header.trim())
}

fn guarded_error(status: StatusCode, message: &str, is_api: bool) -> Response {
    let mut resp = (status, Json(serde_json::json!({ "error": message }))).into_response();
    apply_security_headers(resp.headers_mut(), is_api);
    resp
}

fn apply_security_headers(headers: &mut HeaderMap, is_api: bool) {
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'self'; base-uri 'none'; form-action 'self'; object-src 'none'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("SAMEORIGIN"),
    );
    if is_api {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

pub async fn security_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let is_api = req.uri().path().starts_with("/api/");

    let Some(host_header) = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
    else {
        return guarded_error(StatusCode::FORBIDDEN, "missing Host header", is_api);
    };

    if !host_allowed(&host_header, &state.config.dashboard.allowed_hosts) {
        return guarded_error(StatusCode::FORBIDDEN, "host not allowed", is_api);
    }

    let safe_method = req.method() == Method::GET || req.method() == Method::HEAD;
    // Same-site pages still carry the SameSite=Strict cookie, so cookie-authenticated reads need the marker too.
    let needs_marker = !safe_method || (is_api && bearer_token(req.headers()).is_none());
    if needs_marker {
        let has_marker = req
            .headers()
            .get(DASHBOARD_MARKER_HEADER)
            .and_then(|v| v.to_str().ok())
            == Some("1");
        if !has_marker {
            return guarded_error(
                StatusCode::FORBIDDEN,
                "missing X-Swing-Dashboard header",
                is_api,
            );
        }
        if let Some(origin) = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            && !origin_matches_host(origin, &host_header)
        {
            return guarded_error(StatusCode::FORBIDDEN, "origin does not match host", is_api);
        }
    }

    if is_api
        && !UNAUTHENTICATED_API_PATHS.contains(&req.uri().path())
        && !authorized(req.headers(), &host_header, &state.token())
    {
        return guarded_error(
            StatusCode::UNAUTHORIZED,
            "missing or invalid dashboard token or session",
            is_api,
        );
    }

    let mut response = next.run(req).await;
    apply_security_headers(response.headers_mut(), is_api);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowed_rejects_bracketed_host_with_trailing_junk() {
        assert!(!host_allowed("[::1]xyz", &[]));
        assert!(!host_allowed("[::1]:8082xyz", &[]));
    }

    #[test]
    fn host_allowed_rejects_non_digit_ports() {
        assert!(!host_allowed("localhost:abc", &[]));
    }

    #[test]
    fn host_allowed_accepts_loopback_forms() {
        assert!(host_allowed("localhost:8082", &[]));
        assert!(host_allowed("127.0.0.1:8082", &[]));
        assert!(host_allowed("127.0.0.1", &[]));
        assert!(host_allowed("[::1]:8082", &[]));
        assert!(host_allowed("[::1]", &[]));
    }

    #[test]
    fn host_allowed_checks_allowed_hosts_case_insensitively() {
        let allowed = vec!["My-Site.example".to_string()];
        assert!(host_allowed("my-site.example:8082", &allowed));
        assert!(!host_allowed("other.example:8082", &allowed));
    }

    #[test]
    fn host_allowed_rejects_unknown_hosts() {
        assert!(!host_allowed("evil.example", &[]));
        assert!(!host_allowed("evil.example:8082", &[]));
    }

    #[test]
    fn host_allowed_rejects_junk_disguised_as_localhost() {
        assert!(!host_allowed("evil.com:8082@localhost", &[]));
    }

    #[test]
    fn session_cookie_name_includes_the_port() {
        assert_eq!(session_cookie_name("127.0.0.1:8082"), "swing_session_8082");
        assert_eq!(session_cookie_name("[::1]:18082"), "swing_session_18082");
        assert_eq!(session_cookie_name("localhost"), "swing_session");
        assert_eq!(session_cookie_name("[::1]"), "swing_session");
    }

    fn headers(pairs: &[(HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(k.clone(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn authorized_accepts_bearer_token() {
        let h = headers(&[(header::AUTHORIZATION, "Bearer tok")]);
        assert!(authorized(&h, "127.0.0.1:8082", "tok"));
        let h = headers(&[(header::AUTHORIZATION, "Bearer nope")]);
        assert!(!authorized(&h, "127.0.0.1:8082", "tok"));
        assert!(!authorized(&HeaderMap::new(), "127.0.0.1:8082", "tok"));
    }

    #[test]
    fn authorized_accepts_session_cookie_for_this_port_only() {
        let session = auth::new_session("tok", auth::DASHBOARD_SESSION);
        let cookie = format!("theme=dark; swing_session_8082={session}");
        let h = headers(&[(header::COOKIE, &cookie)]);
        assert!(authorized(&h, "127.0.0.1:8082", "tok"));
        assert!(!authorized(&h, "127.0.0.1:18082", "tok"));
        assert!(!authorized(&h, "127.0.0.1:8082", "rotated"));
    }

    #[test]
    fn origin_matches_host_compares_authority_only() {
        assert!(origin_matches_host(
            "http://127.0.0.1:8082",
            "127.0.0.1:8082"
        ));
        assert!(origin_matches_host(
            "https://127.0.0.1:8082/",
            "127.0.0.1:8082"
        ));
        assert!(!origin_matches_host(
            "http://evil.example",
            "127.0.0.1:8082"
        ));
        assert!(!origin_matches_host(
            "http://127.0.0.1:9999",
            "127.0.0.1:8082"
        ));
    }
}
