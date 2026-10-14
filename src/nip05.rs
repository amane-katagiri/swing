use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::Value;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const STATE_VERIFIED: &str = "verified";
pub const STATE_ERROR: &str = "error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Unreachable,
    Timeout,
    InvalidResponse,
}

impl ErrorCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCategory::Unreachable => "unreachable",
            ErrorCategory::Timeout => "timeout",
            ErrorCategory::InvalidResponse => "invalid_response",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    NotApplicable,
    Verified,
    Mismatch,
    Error(String, ErrorCategory),
}

impl VerificationResult {
    pub fn as_state_str(&self) -> &'static str {
        match self {
            VerificationResult::Verified => STATE_VERIFIED,
            VerificationResult::Mismatch => "mismatch",
            VerificationResult::NotApplicable => "not_applicable",
            VerificationResult::Error(..) => STATE_ERROR,
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            VerificationResult::Error(msg, _) => Some(msg.clone()),
            _ => None,
        }
    }

    pub fn coarse_detail(&self) -> Option<&'static str> {
        match self {
            VerificationResult::Error(_, category) => Some(category.as_str()),
            _ => None,
        }
    }

    pub fn is_verified(&self) -> bool {
        matches!(self, VerificationResult::Verified)
    }
}

pub trait Nip05Verify {
    fn verify(&self, d: &str, pubkey_hex: &str) -> impl Future<Output = VerificationResult> + Send;
}

fn normalize_hostname(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    if lower.is_empty() || lower.len() > 253 {
        return None;
    }
    if lower.contains(['/', ':', '@', '?', '#']) {
        return None;
    }
    let labels: Vec<&str> = lower.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    let valid_label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if !labels.iter().all(|l| valid_label(l)) {
        return None;
    }
    // The WHATWG host parser reads forms like "0x7f.1" as IPv4, so defer to it.
    let url = reqwest::Url::parse(&format!("https://{lower}/")).ok()?;
    if url.domain() != Some(lower.as_str()) {
        return None;
    }
    Some(lower)
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 198 && (b == 18 || b == 19))
        || (a == 192 && b == 0 && c == 0))
}

fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return Some(v4);
    }
    let seg = ip.segments();
    let low = || Ipv4Addr::from((u32::from(seg[6]) << 16) | u32::from(seg[7]));
    match seg {
        [0x0064, 0xff9b, 0, 0, 0, 0, _, _] => Some(low()),
        [0x2002, hi, lo, ..] => Some(Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo))),
        [0, 0, 0, 0, 0, 0, _, _] if !ip.is_unspecified() && !ip.is_loopback() => Some(low()),
        _ => None,
    }
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let [s0, s1, s2, ..] = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || (s0 & 0xffc0) == 0xfec0
        || (s0 == 0x2001 && s1 == 0)
        || (s0 == 0x0064 && s1 == 0xff9b && s2 == 1)
        || (s0 == 0x2001 && s1 == 0x0db8))
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let public: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| is_public_ip(addr.ip()))
                .collect();
            if public.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            Ok(Box::new(public.into_iter()) as Addrs)
        })
    }
}

fn evaluate_body(body: &str, pubkey_hex: &str) -> VerificationResult {
    let json: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return VerificationResult::Error(
                format!("invalid JSON: {e}"),
                ErrorCategory::InvalidResponse,
            );
        }
    };
    let found = json
        .get("names")
        .and_then(|names| names.get("_"))
        .and_then(|v| v.as_str());
    match found {
        Some(hex) if hex.eq_ignore_ascii_case(pubkey_hex) => VerificationResult::Verified,
        _ => VerificationResult::Mismatch,
    }
}

pub struct HttpNip05Verifier {
    http: reqwest::Client,
}

impl HttpNip05Verifier {
    pub fn new() -> Self {
        Self::build(reqwest::Client::builder())
    }

    pub fn public_only() -> Self {
        // A proxy would resolve the name itself and bypass the address filter.
        Self::build(
            reqwest::Client::builder()
                .no_proxy()
                .dns_resolver(Arc::new(PublicOnlyResolver)),
        )
    }

    fn build(builder: reqwest::ClientBuilder) -> Self {
        Self {
            http: builder
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("reqwest client uses only built-in TLS/redirect options"),
        }
    }
}

impl Default for HttpNip05Verifier {
    fn default() -> Self {
        Self::new()
    }
}

fn classify_from_flags(timeout: bool, connect: bool) -> ErrorCategory {
    if timeout {
        ErrorCategory::Timeout
    } else if connect {
        ErrorCategory::Unreachable
    } else {
        ErrorCategory::InvalidResponse
    }
}

fn classify_reqwest_error(e: &reqwest::Error) -> ErrorCategory {
    classify_from_flags(e.is_timeout(), e.is_connect())
}

impl Nip05Verify for HttpNip05Verifier {
    async fn verify(&self, d: &str, pubkey_hex: &str) -> VerificationResult {
        let Some(domain) = normalize_hostname(d) else {
            return VerificationResult::NotApplicable;
        };
        let url = format!("https://{domain}/.well-known/nostr.json?name=_");
        let resp = match self.http.get(&url).timeout(FETCH_TIMEOUT).send().await {
            Ok(r) => r,
            Err(e) => {
                let category = classify_reqwest_error(&e);
                return VerificationResult::Error(e.to_string(), category);
            }
        };
        read_response(resp, pubkey_hex).await
    }
}

async fn read_response(resp: reqwest::Response, pubkey_hex: &str) -> VerificationResult {
    if !resp.status().is_success() {
        return VerificationResult::Error(
            format!("HTTP status {}", resp.status()),
            ErrorCategory::InvalidResponse,
        );
    }

    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let category = classify_reqwest_error(&e);
                return VerificationResult::Error(e.to_string(), category);
            }
        };
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return VerificationResult::Error(
                "response body exceeds the size cap".to_string(),
                ErrorCategory::InvalidResponse,
            );
        }
        body.extend_from_slice(&chunk);
    }

    match String::from_utf8(body) {
        Ok(text) => evaluate_body(&text, pubkey_hex),
        Err(_) => VerificationResult::Error(
            "response body is not valid UTF-8".to_string(),
            ErrorCategory::InvalidResponse,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_hostname_accepts_dns_names_only() {
        let long_label = "a".repeat(63);
        let accepted = format!("{long_label}.com");
        for (input, want) in [
            ("Ama.NE.jp", "ama.ne.jp"),
            ("example.com", "example.com"),
            ("sub.example.com", "sub.example.com"),
            (accepted.as_str(), accepted.as_str()),
            ("xn--eckwd4c7c.xn--zckzah", "xn--eckwd4c7c.xn--zckzah"),
        ] {
            assert_eq!(normalize_hostname(input).as_deref(), Some(want), "{input}");
        }
        let too_long_label = format!("{}.com", "a".repeat(64));
        let too_long_name = format!("{}.com", "a".repeat(250));
        for (case, input) in [
            ("no dot", "localhost"),
            ("scheme", "https://example.com"),
            ("path", "example.com/path"),
            ("port", "example.com:8080"),
            ("empty", ""),
            ("space", "exa mple.com"),
            ("underscore", "exa_mple.com"),
            ("empty label", "example..com"),
            ("ipv4 loopback", "127.0.0.1"),
            ("ipv4 private", "10.0.0.5"),
            ("ipv4 link-local", "169.254.169.254"),
            ("hex ipv4", "0x7f.1"),
            ("octal ipv4", "0177.0.0.1"),
            ("numeric tld", "example.123"),
            ("leading hyphen", "-example.com"),
            ("trailing hyphen", "example-.com"),
            ("label over 63", too_long_label.as_str()),
            ("name over 253", too_long_name.as_str()),
        ] {
            assert_eq!(normalize_hostname(input), None, "{case}: {input}");
        }
    }

    #[test]
    fn public_ip_filter() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "198.18.0.1",
            "192.0.0.8",
            "224.0.0.1",
            "::1",
            "::",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b:1::1",
            "2002:7f00:1::1",
            "2002:c0a8:101::1",
            "::127.0.0.1",
            "::10.0.0.1",
            "fec0::1",
            "feff::1",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} should be blocked");
        }
        for ip in [
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
            "::8.8.8.8",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    #[tokio::test]
    async fn public_only_resolver_refuses_loopback_names() {
        let name: Name = "localhost".parse().unwrap();
        let Err(err) = PublicOnlyResolver.resolve(name).await else {
            panic!("localhost resolved to a public address");
        };
        assert_eq!(err.to_string(), "localhost has no public address");
    }

    #[tokio::test]
    async fn public_only_resolver_resolves_only_public_ip_literals() {
        for (literal, public) in [
            ("10.0.0.5", false),
            ("192.168.1.1", false),
            ("169.254.169.254", false),
            ("::1", false),
            ("fc00::1", false),
            ("1.1.1.1", true),
            ("8.8.8.8", true),
        ] {
            let name: Name = literal.parse().unwrap();
            assert_eq!(
                PublicOnlyResolver.resolve(name).await.is_ok(),
                public,
                "{literal}"
            );
        }
    }

    #[test]
    fn evaluate_body_matches_the_underscore_name() {
        for (case, body, pubkey, want) in [
            (
                "hex case ignored",
                r#"{"names":{"_":"ABCDEF0123"}}"#,
                "abcdef0123",
                "verified",
            ),
            (
                "different hex",
                r#"{"names":{"_":"abcdef0123"}}"#,
                "0000000000",
                "mismatch",
            ),
            (
                "no underscore name",
                r#"{"names":{"someone_else":"abcdef0123"}}"#,
                "abcdef0123",
                "mismatch",
            ),
            ("invalid json", "not json", "abcdef0123", "error"),
        ] {
            assert_eq!(evaluate_body(body, pubkey).as_state_str(), want, "{case}");
        }
    }

    #[test]
    fn state_str_and_detail_roundtrip() {
        assert_eq!(VerificationResult::Verified.as_state_str(), "verified");
        assert_eq!(VerificationResult::Mismatch.as_state_str(), "mismatch");
        assert_eq!(
            VerificationResult::NotApplicable.as_state_str(),
            "not_applicable"
        );
        let err = VerificationResult::Error("boom".to_string(), ErrorCategory::InvalidResponse);
        assert_eq!(err.as_state_str(), "error");
        assert_eq!(err.detail(), Some("boom".to_string()));
        assert_eq!(VerificationResult::Verified.detail(), None);
    }

    #[test]
    fn coarse_detail_hides_the_raw_message() {
        let err = VerificationResult::Error(
            "connection refused to 10.0.0.5:443".to_string(),
            ErrorCategory::Unreachable,
        );
        assert_eq!(err.coarse_detail(), Some("unreachable"));
        assert_eq!(
            VerificationResult::Error("deadline exceeded".to_string(), ErrorCategory::Timeout)
                .coarse_detail(),
            Some("timeout")
        );
        assert_eq!(VerificationResult::Verified.coarse_detail(), None);
        assert_eq!(VerificationResult::Mismatch.coarse_detail(), None);
        assert_eq!(VerificationResult::NotApplicable.coarse_detail(), None);
    }

    #[test]
    fn classify_from_flags_prioritizes_timeout_over_connect() {
        assert_eq!(classify_from_flags(true, true), ErrorCategory::Timeout);
        assert_eq!(classify_from_flags(true, false), ErrorCategory::Timeout);
        assert_eq!(classify_from_flags(false, true), ErrorCategory::Unreachable);
        assert_eq!(
            classify_from_flags(false, false),
            ErrorCategory::InvalidResponse
        );
    }

    #[tokio::test]
    async fn a_d_that_is_not_a_hostname_is_not_looked_up() {
        assert_eq!(
            HttpNip05Verifier::new().verify("my-blog", "abcdef").await,
            VerificationResult::NotApplicable
        );
    }

    async fn served(
        status: u16,
        headers: &'static [(&'static str, &'static str)],
        body: Vec<u8>,
        pubkey_hex: &str,
    ) -> VerificationResult {
        let router = axum::Router::new().route(
            "/.well-known/nostr.json",
            axum::routing::get(move || async move {
                let mut resp = axum::response::Response::new(axum::body::Body::from(body));
                *resp.status_mut() = axum::http::StatusCode::from_u16(status).unwrap();
                for (name, value) in headers {
                    resp.headers_mut().insert(*name, value.parse().unwrap());
                }
                resp
            }),
        );
        let addr = crate::test_support::serve_router(router).await;
        let resp = HttpNip05Verifier::new()
            .http
            .get(format!("http://{addr}/.well-known/nostr.json?name=_"))
            .send()
            .await
            .unwrap();
        read_response(resp, pubkey_hex).await
    }

    fn invalid(result: VerificationResult) -> String {
        match result {
            VerificationResult::Error(message, ErrorCategory::InvalidResponse) => message,
            other => panic!("expected an invalid response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_served_nostr_json_is_read_and_evaluated() {
        let body = br#"{"names":{"_":"abcdef"}}"#.to_vec();
        assert_eq!(
            served(200, &[], body.clone(), "abcdef").await,
            VerificationResult::Verified
        );
        assert_eq!(
            served(200, &[], body, "012345").await,
            VerificationResult::Mismatch
        );
    }

    #[tokio::test]
    async fn non_success_and_redirects_are_not_followed() {
        let body = br#"{"names":{"_":"abcdef"}}"#.to_vec();
        assert!(invalid(served(404, &[], body.clone(), "abcdef").await).contains("404"));
        let redirected = served(
            302,
            &[("location", "https://example.com/.well-known/nostr.json")],
            body,
            "abcdef",
        )
        .await;
        assert!(invalid(redirected).contains("302"));
    }

    #[tokio::test]
    async fn bodies_over_the_cap_or_not_utf8_are_refused() {
        let mut big = br#"{"names":{"_":"abcdef"}, "pad":""#.to_vec();
        big.resize(MAX_BODY_BYTES + 1, b' ');
        assert!(invalid(served(200, &[], big, "abcdef").await).contains("size cap"));

        let mut exact = br#"{"names":{"_":"abcdef"}}"#.to_vec();
        exact.resize(MAX_BODY_BYTES, b' ');
        assert_eq!(
            served(200, &[], exact, "abcdef").await,
            VerificationResult::Verified
        );

        let latin1 = b"{\"names\":{\"_\":\"abcdef\"},\"x\":\"\xff\"}".to_vec();
        assert!(invalid(served(200, &[], latin1, "abcdef").await).contains("UTF-8"));
    }
}
