use axum::Json;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use super::AppState;
use crate::auth;
use crate::host::{extract_host, split_host_port};

const DASHBOARD_MARKER_HEADER: &str = "x-swing-dashboard";
const SESSION_COOKIE_PREFIX: &str = "swing_session";
const UNAUTHENTICATED_API_PATHS: &[&str] = &["/api/login", "/api/identity"];
const PROTECTED_UI_PATHS: &[&str] = &["/desktop-page.html", "/desktop-page.css", "/desktop-banner"];
const PROTECTED_UI_PREFIX: &str = "/mascots/";

fn needs_auth(path: &str) -> bool {
    if path.starts_with("/api/") {
        return !UNAUTHENTICATED_API_PATHS.contains(&path);
    }
    PROTECTED_UI_PATHS.contains(&path) || path.starts_with(PROTECTED_UI_PREFIX)
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    Bearer,
    Session,
}

pub fn authenticate(headers: &HeaderMap, host_header: &str, token: &str) -> Option<AuthMethod> {
    if let Some(presented) = bearer_token(headers) {
        return auth::token_matches(token, presented.trim()).then_some(AuthMethod::Bearer);
    }
    let name = session_cookie_name(host_header);
    cookie_values(headers, &name)
        .any(|v| auth::verify_session(token, auth::DASHBOARD_SESSION, v))
        .then_some(AuthMethod::Session)
}

pub async fn require_bearer(req: Request<Body>, next: Next) -> Response {
    if req.extensions().get::<AuthMethod>() != Some(&AuthMethod::Bearer) {
        return guarded_error(
            StatusCode::FORBIDDEN,
            "this endpoint needs the dashboard token, not a browser session",
            true,
        );
    }
    next.run(req).await
}

// DNS rebinding sends the attacker's hostname, so a Host naming the bound IP literally cannot come from it.
fn is_listen_ip(host: &str, listen: SocketAddr) -> bool {
    !listen.ip().is_unspecified() && host.parse::<IpAddr>().is_ok_and(|ip| ip == listen.ip())
}

pub fn host_allowed(host_header: &str, allowed_hosts: &[String], listen: SocketAddr) -> bool {
    let host = extract_host(host_header);
    crate::host::is_loopback_name(&host)
        || is_listen_ip(&host, listen)
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
    mut req: Request<Body>,
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

    if !host_allowed(
        &host_header,
        &state.config.dashboard.allowed_hosts,
        state.config.dashboard.listen,
    ) {
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

    let protected = needs_auth(req.uri().path());
    if protected {
        let Some(method) = authenticate(req.headers(), &host_header, &state.token()) else {
            return guarded_error(
                StatusCode::UNAUTHORIZED,
                "missing or invalid dashboard token or session",
                is_api,
            );
        };
        req.extensions_mut().insert(method);
    }

    let mut response = next.run(req).await;
    apply_security_headers(response.headers_mut(), is_api);
    // Stops same-site pages on other ports from embedding authenticated files, which the Strict cookie would otherwise allow.
    if protected && !is_api {
        response.headers_mut().insert(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOPBACK: SocketAddr = SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 8082);

    #[test]
    fn host_allowed_accepts_the_literal_listen_ip() {
        let v4: SocketAddr = "192.168.1.5:8082".parse().unwrap();
        assert!(host_allowed("192.168.1.5:8082", &[], v4));
        assert!(host_allowed("192.168.1.5", &[], v4));
        assert!(!host_allowed("192.168.1.6:8082", &[], v4));
        assert!(!host_allowed("evil.example:8082", &[], v4));

        let v6: SocketAddr = "[fd00::1]:8082".parse().unwrap();
        assert!(host_allowed("[fd00::1]:8082", &[], v6));
        assert!(host_allowed("[FD00:0::1]", &[], v6));
        assert!(!host_allowed("[fd00::2]:8082", &[], v6));
        assert!(!host_allowed("[fd00::1]junk", &[], v6));
    }

    #[test]
    fn host_allowed_does_not_accept_the_unspecified_address() {
        let any: SocketAddr = "0.0.0.0:8082".parse().unwrap();
        assert!(!host_allowed("0.0.0.0:8082", &[], any));
        let any6: SocketAddr = "[::]:8082".parse().unwrap();
        assert!(!host_allowed("[::]:8082", &[], any6));
    }

    #[test]
    fn host_allowed_rejects_bracketed_host_with_trailing_junk() {
        assert!(!host_allowed("[::1]xyz", &[], LOOPBACK));
        assert!(!host_allowed("[::1]:8082xyz", &[], LOOPBACK));
    }

    #[test]
    fn host_allowed_rejects_non_digit_ports() {
        assert!(!host_allowed("localhost:abc", &[], LOOPBACK));
    }

    #[test]
    fn host_allowed_accepts_loopback_forms() {
        assert!(host_allowed("localhost:8082", &[], LOOPBACK));
        assert!(host_allowed("127.0.0.1:8082", &[], LOOPBACK));
        assert!(host_allowed("127.0.0.1", &[], LOOPBACK));
        assert!(host_allowed("[::1]:8082", &[], LOOPBACK));
        assert!(host_allowed("[::1]", &[], LOOPBACK));
    }

    #[test]
    fn host_allowed_checks_allowed_hosts_case_insensitively() {
        let allowed = vec!["My-Site.example".to_string()];
        assert!(host_allowed("my-site.example:8082", &allowed, LOOPBACK));
        assert!(!host_allowed("other.example:8082", &allowed, LOOPBACK));
    }

    #[test]
    fn host_allowed_rejects_unknown_hosts() {
        assert!(!host_allowed("evil.example", &[], LOOPBACK));
        assert!(!host_allowed("evil.example:8082", &[], LOOPBACK));
    }

    #[test]
    fn host_allowed_rejects_junk_disguised_as_localhost() {
        assert!(!host_allowed("evil.com:8082@localhost", &[], LOOPBACK));
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
    fn authenticate_accepts_bearer_token() {
        let h = headers(&[(header::AUTHORIZATION, "Bearer tok")]);
        assert_eq!(
            authenticate(&h, "127.0.0.1:8082", "tok"),
            Some(AuthMethod::Bearer)
        );
        let h = headers(&[(header::AUTHORIZATION, "Bearer nope")]);
        assert_eq!(authenticate(&h, "127.0.0.1:8082", "tok"), None);
        assert_eq!(
            authenticate(&HeaderMap::new(), "127.0.0.1:8082", "tok"),
            None
        );
    }

    #[test]
    fn authenticate_accepts_session_cookie_for_this_port_only() {
        let session = auth::new_session("tok", auth::DASHBOARD_SESSION);
        let cookie = format!("theme=dark; swing_session_8082={session}");
        let h = headers(&[(header::COOKIE, &cookie)]);
        assert_eq!(
            authenticate(&h, "127.0.0.1:8082", "tok"),
            Some(AuthMethod::Session)
        );
        assert_eq!(authenticate(&h, "127.0.0.1:18082", "tok"), None);
        assert_eq!(authenticate(&h, "127.0.0.1:8082", "rotated"), None);
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
