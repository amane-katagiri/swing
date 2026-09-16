use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::{self, Config, Nip05Mode};
use crate::ipfs::IpfsClient;
use crate::nip05::{self, Nip05Verify};
use crate::nostr::{self, RelayClient, build_site_event_builder};

fn host_from_url(url: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("invalid URL: {url}"))?;
    parsed
        .host_str()
        .map(|s| s.to_string())
        .with_context(|| format!("URL has no host: {url}"))
}

fn format_nip05_line(result: &nip05::VerificationResult) -> String {
    match result {
        nip05::VerificationResult::Verified => "\u{2713} verified".to_string(),
        nip05::VerificationResult::Mismatch => {
            "! mismatch: this pubkey is not listed under \"_\" in nostr.json".to_string()
        }
        nip05::VerificationResult::NotApplicable => {
            "- not applicable (d is not a domain)".to_string()
        }
        nip05::VerificationResult::Error(msg) => format!("! error: {msg}"),
    }
}

fn nip05_abort_message(
    mode: Nip05Mode,
    result: &nip05::VerificationResult,
    d: &str,
    pubkey_hex: &str,
) -> Option<String> {
    if mode != Nip05Mode::Require || result.is_verified() {
        return None;
    }
    Some(match result {
        nip05::VerificationResult::NotApplicable => {
            format!("NIP-05 verification required, but \"{d}\" is not a valid domain name")
        }
        _ => format!(
            "NIP-05 verification required but not satisfied for {d}: publish {{\"names\": {{\"_\": \"{pubkey_hex}\"}}}} at https://{d}/.well-known/nostr.json"
        ),
    })
}

struct Nip05Outcome {
    line: String,
    abort: Option<String>,
}

async fn check_nip05<V: Nip05Verify>(
    verifier: &V,
    mode: Nip05Mode,
    d: &str,
    pubkey_hex: &str,
) -> Nip05Outcome {
    let result = verifier.verify(d, pubkey_hex).await;
    let line = format_nip05_line(&result);
    let abort = nip05_abort_message(mode, &result, d, pubkey_hex);
    Nip05Outcome { line, abort }
}

pub async fn run(
    config: Config,
    site: Option<String>,
    url: String,
    dir: &Path,
    nip05_override: Option<String>,
) -> Result<()> {
    let d = match site {
        Some(s) => s,
        None => host_from_url(&url)?,
    };
    let nip05_mode = match nip05_override {
        Some(v) => config::parse_nip05_mode(&v).context("invalid --nip05")?,
        None => config.publish.nip05,
    };

    println!("Site: {url}");

    if nip05_mode != Nip05Mode::Off {
        let keys = Keys::parse(&config.nostr.secret_key).context("parsing Nostr secret key")?;
        let pubkey_hex = keys.public_key().to_hex();
        let verifier = nip05::HttpNip05Verifier::new();
        let outcome = check_nip05(&verifier, nip05_mode, &d, &pubkey_hex).await;

        println!();
        println!("NIP-05");
        println!("  {}", outcome.line);

        if let Some(msg) = outcome.abort {
            anyhow::bail!(msg);
        }
    }

    println!();
    println!("IPFS");

    let ipfs = IpfsClient::new(config.ipfs.api.clone());
    let cid = ipfs.add_dir(dir).await?;
    println!("  CID: {cid}");
    println!("  \u{2713} added");
    println!("  \u{2713} pinned");

    let size = match ipfs.files_stat(&cid).await {
        Ok(s) => Some(s),
        Err(e) => {
            println!("  ! size unknown (files/stat failed: {e})");
            None
        }
    };

    println!();
    println!("Nostr");

    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let event = build_site_event_builder(config.nostr.site_event_kind, &d, &cid, Some(&url), size)
        .finalize(&relay.keys)
        .context("signing site event")?;
    let output = relay.publish_to_relays(&event).await?;

    nostr::print_relay_send_results(relay.relays(), &output);

    println!();
    println!("Published.");

    relay.client.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_from_url_extracts_hostname() {
        assert_eq!(host_from_url("https://ama.ne.jp/").unwrap(), "ama.ne.jp");
        assert_eq!(
            host_from_url("https://sub.example.com:8080/path").unwrap(),
            "sub.example.com"
        );
    }

    #[test]
    fn host_from_url_rejects_invalid_url() {
        assert!(host_from_url("not a url").is_err());
    }

    struct FakeNip05(nip05::VerificationResult);

    impl Nip05Verify for FakeNip05 {
        async fn verify(&self, _d: &str, _pubkey_hex: &str) -> nip05::VerificationResult {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn warn_mode_never_aborts() {
        for result in [
            nip05::VerificationResult::Verified,
            nip05::VerificationResult::Mismatch,
            nip05::VerificationResult::NotApplicable,
            nip05::VerificationResult::Error("boom".to_string()),
        ] {
            let fake = FakeNip05(result);
            let outcome = check_nip05(&fake, Nip05Mode::Warn, "example.com", "abc123").await;
            assert!(outcome.abort.is_none());
        }
    }

    #[tokio::test]
    async fn require_mode_proceeds_only_when_verified() {
        let fake = FakeNip05(nip05::VerificationResult::Verified);
        let outcome = check_nip05(&fake, Nip05Mode::Require, "example.com", "abc123").await;
        assert!(outcome.abort.is_none());
        assert!(outcome.line.contains("verified"));
    }

    #[tokio::test]
    async fn require_mode_aborts_on_mismatch() {
        let fake = FakeNip05(nip05::VerificationResult::Mismatch);
        let outcome = check_nip05(&fake, Nip05Mode::Require, "example.com", "abc123").await;
        assert!(outcome.abort.is_some());
        assert!(outcome.abort.unwrap().contains("abc123"));
    }

    #[tokio::test]
    async fn require_mode_aborts_on_not_applicable_with_domain_specific_message() {
        let fake = FakeNip05(nip05::VerificationResult::NotApplicable);
        let outcome = check_nip05(&fake, Nip05Mode::Require, "not-a-domain", "abc123").await;
        let msg = outcome.abort.unwrap();
        assert!(msg.contains("not-a-domain"));
        assert!(msg.contains("not a valid domain"));
    }

    #[tokio::test]
    async fn require_mode_aborts_on_error() {
        let fake = FakeNip05(nip05::VerificationResult::Error("timeout".to_string()));
        let outcome = check_nip05(&fake, Nip05Mode::Require, "example.com", "abc123").await;
        assert!(outcome.abort.is_some());
    }

    #[tokio::test]
    async fn off_mode_never_aborts_even_on_mismatch() {
        let fake = FakeNip05(nip05::VerificationResult::Mismatch);
        let outcome = check_nip05(&fake, Nip05Mode::Off, "example.com", "abc123").await;
        assert!(outcome.abort.is_none());
    }
}
