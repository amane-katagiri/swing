use std::future::Future;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    NotApplicable,
    Verified,
    Mismatch,
    Error(String),
}

impl VerificationResult {
    pub fn as_state_str(&self) -> &'static str {
        match self {
            VerificationResult::Verified => "verified",
            VerificationResult::Mismatch => "mismatch",
            VerificationResult::NotApplicable => "not_applicable",
            VerificationResult::Error(_) => "error",
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            VerificationResult::Error(msg) => Some(msg.clone()),
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

pub fn normalize_hostname(input: &str) -> Option<String> {
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
    let valid_label =
        |l: &str| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !labels.iter().all(|l| valid_label(l)) {
        return None;
    }
    Some(lower)
}

fn evaluate_body(body: &str, pubkey_hex: &str) -> VerificationResult {
    let json: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return VerificationResult::Error(format!("invalid JSON: {e}")),
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
        Self {
            http: reqwest::Client::builder()
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

impl Nip05Verify for HttpNip05Verifier {
    async fn verify(&self, d: &str, pubkey_hex: &str) -> VerificationResult {
        let Some(domain) = normalize_hostname(d) else {
            return VerificationResult::NotApplicable;
        };
        let url = format!("https://{domain}/.well-known/nostr.json?name=_");
        let resp = match self.http.get(&url).timeout(FETCH_TIMEOUT).send().await {
            Ok(r) => r,
            Err(e) => return VerificationResult::Error(e.to_string()),
        };
        if !resp.status().is_success() {
            return VerificationResult::Error(format!("http status {}", resp.status()));
        }

        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => return VerificationResult::Error(e.to_string()),
            };
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return VerificationResult::Error("response body exceeds size cap".to_string());
            }
            body.extend_from_slice(&chunk);
        }

        match String::from_utf8(body) {
            Ok(text) => evaluate_body(&text, pubkey_hex),
            Err(_) => VerificationResult::Error("response body is not valid UTF-8".to_string()),
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
        assert!(matches!(result, VerificationResult::Error(_)));
    }

    #[test]
    fn state_str_and_detail_roundtrip() {
        assert_eq!(VerificationResult::Verified.as_state_str(), "verified");
        assert_eq!(VerificationResult::Mismatch.as_state_str(), "mismatch");
        assert_eq!(
            VerificationResult::NotApplicable.as_state_str(),
            "not_applicable"
        );
        let err = VerificationResult::Error("boom".to_string());
        assert_eq!(err.as_state_str(), "error");
        assert_eq!(err.detail(), Some("boom".to_string()));
        assert_eq!(VerificationResult::Verified.detail(), None);
    }
}
