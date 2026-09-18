use axum::Json;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

use super::AppState;

const DASHBOARD_MARKER_HEADER: &str = "x-swing-dashboard";

fn split_host_port(host_header: &str) -> &str {
    if let Some(rest) = host_header.strip_prefix('[') {
        return match rest.find(']') {
            Some(end) => &rest[..end],
            None => host_header,
        };
    }
    match host_header.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => host_header,
    }
}

pub fn extract_host(host_header: &str) -> String {
    split_host_port(host_header.trim()).to_ascii_lowercase()
}

pub fn host_allowed(host_header: &str, allowed_hosts: &[String]) -> bool {
    let host = extract_host(host_header);
    host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host))
}

pub fn origin_matches_host(origin_header: &str, host_header: &str) -> bool {
    let authority = origin_header
        .split_once("://")
        .map_or(origin_header, |(_, rest)| rest);
    let authority = authority.trim_end_matches('/');
    authority.eq_ignore_ascii_case(host_header.trim())
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

fn apply_security_headers(headers: &mut HeaderMap, is_api: bool) {
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
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
        let mut resp = error_response(StatusCode::FORBIDDEN, "missing Host header");
        apply_security_headers(resp.headers_mut(), is_api);
        return resp;
    };

    if !host_allowed(&host_header, &state.config.dashboard.allowed_hosts) {
        let mut resp = error_response(StatusCode::FORBIDDEN, "host not allowed");
        apply_security_headers(resp.headers_mut(), is_api);
        return resp;
    }

    if req.method() != Method::GET && req.method() != Method::HEAD {
        let has_marker = req
            .headers()
            .get(DASHBOARD_MARKER_HEADER)
            .and_then(|v| v.to_str().ok())
            == Some("1");
        if !has_marker {
            let mut resp =
                error_response(StatusCode::FORBIDDEN, "missing X-Swing-Dashboard header");
            apply_security_headers(resp.headers_mut(), is_api);
            return resp;
        }
        if let Some(origin) = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            && !origin_matches_host(origin, &host_header)
        {
            let mut resp = error_response(StatusCode::FORBIDDEN, "origin does not match host");
            apply_security_headers(resp.headers_mut(), is_api);
            return resp;
        }
    }

    let mut response = next.run(req).await;
    apply_security_headers(response.headers_mut(), is_api);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_strips_port_when_present() {
        assert_eq!(extract_host("127.0.0.1:8082"), "127.0.0.1");
        assert_eq!(extract_host("127.0.0.1"), "127.0.0.1");
        assert_eq!(extract_host("Localhost:8082"), "localhost");
    }

    #[test]
    fn extract_host_handles_ipv6_brackets() {
        assert_eq!(extract_host("[::1]:8082"), "::1");
        assert_eq!(extract_host("[::1]"), "::1");
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
