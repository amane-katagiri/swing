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

    /// A coarse, oracle-resistant classification of `Error`'s detail, safe to hand to a network caller.
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

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let [s0, s1, ..] = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
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
        if !resp.status().is_success() {
            return VerificationResult::Error(
                format!("http status {}", resp.status()),
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
                    "response body exceeds size cap".to_string(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_hostname_accepts_lowercase_and_normalizes_case() {
        assert_eq!(
            normalize_hostname("Ama.NE.jp").as_deref(),
            Some("ama.ne.jp")
        );
        assert_eq!(
            normalize_hostname("example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            normalize_hostname("sub.example.com").as_deref(),
            Some("sub.example.com")
        );
    }

    #[test]
    fn normalize_hostname_rejects_missing_dot() {
        assert_eq!(normalize_hostname("localhost"), None);
    }

    #[test]
    fn normalize_hostname_rejects_scheme_path_port() {
        assert_eq!(normalize_hostname("https://example.com"), None);
        assert_eq!(normalize_hostname("example.com/path"), None);
        assert_eq!(normalize_hostname("example.com:8080"), None);
    }

    #[test]
    fn normalize_hostname_rejects_invalid_characters_and_empty() {
        assert_eq!(normalize_hostname(""), None);
        assert_eq!(normalize_hostname("exa mple.com"), None);
        assert_eq!(normalize_hostname("exa_mple.com"), None);
        assert_eq!(normalize_hostname("example..com"), None);
    }

    #[test]
    fn normalize_hostname_rejects_ip_addresses() {
        assert_eq!(normalize_hostname("127.0.0.1"), None);
        assert_eq!(normalize_hostname("10.0.0.5"), None);
        assert_eq!(normalize_hostname("169.254.169.254"), None);
        assert_eq!(normalize_hostname("0x7f.1"), None);
        assert_eq!(normalize_hostname("0177.0.0.1"), None);
        assert_eq!(normalize_hostname("example.123"), None);
    }

    #[test]
    fn normalize_hostname_rejects_bad_labels() {
        assert_eq!(normalize_hostname("-example.com"), None);
        assert_eq!(normalize_hostname("example-.com"), None);
        let label = "a".repeat(64);
        assert_eq!(normalize_hostname(&format!("{label}.com")), None);
        let label = "a".repeat(63);
        assert!(normalize_hostname(&format!("{label}.com")).is_some());
        assert!(normalize_hostname("xn--eckwd4c7c.xn--zckzah").is_some());
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
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} should be blocked");
        }
        for ip in [
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    #[tokio::test]
    async fn public_only_resolver_refuses_loopback_names() {
        let name: Name = "localhost".parse().unwrap();
        assert!(PublicOnlyResolver.resolve(name).await.is_err());
    }

    #[tokio::test]
    async fn public_only_resolver_refuses_private_ip_literals() {
        for literal in [
            "10.0.0.5",
            "192.168.1.1",
            "169.254.169.254",
            "::1",
            "fc00::1",
        ] {
            let name: Name = literal.parse().unwrap();
            assert!(
                PublicOnlyResolver.resolve(name).await.is_err(),
                "{literal} should be refused"
            );
        }
    }

    #[tokio::test]
    async fn public_only_resolver_accepts_public_ip_literals() {
        for literal in ["1.1.1.1", "8.8.8.8"] {
            let name: Name = literal.parse().unwrap();
            assert!(
                PublicOnlyResolver.resolve(name).await.is_ok(),
                "{literal} should be accepted"
            );
        }
    }

    #[test]
    fn normalize_hostname_rejects_too_long() {
        let long_label = "a".repeat(250);
        let d = format!("{long_label}.com");
        assert_eq!(normalize_hostname(&d), None);
    }

    #[test]
    fn evaluate_body_verified_on_matching_hex_case_insensitive() {
        let body = r#"{"names":{"_":"ABCDEF0123"}}"#;
        assert_eq!(
            evaluate_body(body, "abcdef0123"),
            VerificationResult::Verified
        );
    }

    #[test]
    fn evaluate_body_mismatch_on_different_hex() {
        let body = r#"{"names":{"_":"abcdef0123"}}"#;
        assert_eq!(
            evaluate_body(body, "0000000000"),
            VerificationResult::Mismatch
        );
    }

    #[test]
    fn evaluate_body_mismatch_when_underscore_name_missing() {
        let body = r#"{"names":{"someone_else":"abcdef0123"}}"#;
        assert_eq!(
            evaluate_body(body, "abcdef0123"),
            VerificationResult::Mismatch
        );
    }

    #[test]
    fn evaluate_body_error_on_invalid_json() {
        let result = evaluate_body("not json", "abcdef0123");
        assert!(matches!(result, VerificationResult::Error(..)));
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
}
