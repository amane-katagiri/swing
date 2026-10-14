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
mod tests;
