use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::{self, Config, Nip05Mode};
use crate::ipfs::IpfsClient;
use crate::mfs::MfsLayout;
use crate::nip05::{self, Nip05Verify};
use crate::nostr::{self, RelayClient, RelaySendResult, build_site_event_builder};
use crate::signer::Signer;

fn versions_to_prune(names: &[String], keep: usize) -> Vec<String> {
    let mut versions: Vec<(u64, &String)> = names
        .iter()
        .filter_map(|name| name.parse::<u64>().ok().map(|t| (t, name)))
        .collect();
    versions.sort_by(|a, b| b.cmp(a));
    versions
        .into_iter()
        .skip(keep)
        .map(|(_, name)| name.clone())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneAttempt {
    Removed(String),
    Failed(String, String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneOutcome {
    pub attempts: Vec<PruneAttempt>,
    pub list_error: Option<String>,
}

impl PruneOutcome {
    pub fn pruned(&self) -> Vec<&str> {
        self.attempts
            .iter()
            .filter_map(|a| match a {
                PruneAttempt::Removed(name) => Some(name.as_str()),
                PruneAttempt::Failed(..) => None,
            })
            .collect()
    }

    pub fn error_summary(&self) -> Option<String> {
        if let Some(err) = &self.list_error {
            return Some(format!("could not list old versions: {err}"));
        }
        let failed: Vec<String> = self
            .attempts
            .iter()
            .filter_map(|a| match a {
                PruneAttempt::Failed(name, err) => Some(format!("{name}: {err}")),
                PruneAttempt::Removed(_) => None,
            })
            .collect();
        (!failed.is_empty()).then(|| failed.join("; "))
    }
}

pub async fn prune_old_versions_collect(
    ipfs: &IpfsClient,
    site_path: &str,
    keep: usize,
) -> PruneOutcome {
    let names = match ipfs.mfs_list(site_path).await {
        Ok(entries) => entries.into_iter().map(|e| e.name).collect::<Vec<_>>(),
        Err(e) => {
            return PruneOutcome {
                attempts: Vec::new(),
                list_error: Some(format!("{e}")),
            };
        }
    };
    let mut attempts = Vec::new();
    for name in versions_to_prune(&names, keep) {
        let path = format!("{site_path}/{name}");
        match ipfs.mfs_remove(&path).await {
            Ok(()) => attempts.push(PruneAttempt::Removed(name)),
            Err(e) => attempts.push(PruneAttempt::Failed(name, format!("{e}"))),
        }
    }
    PruneOutcome {
        attempts,
        list_error: None,
    }
}

fn print_prune_lines(site_path: &str, outcome: &PruneOutcome) {
    if let Some(err) = &outcome.list_error {
        println!("  ! could not list old versions: {err}");
        return;
    }
    for attempt in &outcome.attempts {
        match attempt {
            PruneAttempt::Removed(name) => println!("  \u{2713} removed {site_path}/{name}"),
            PruneAttempt::Failed(name, err) => {
                println!("  ! could not remove {site_path}/{name}: {err}")
            }
        }
    }
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
        nip05::VerificationResult::Error(msg, _) => format!("! error: {msg}"),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nip05Outcome {
    pub result: nip05::VerificationResult,
    pub line: String,
    pub abort: Option<String>,
}

pub async fn check_nip05<V: Nip05Verify>(
    verifier: &V,
    mode: Nip05Mode,
    d: &str,
    pubkey_hex: &str,
) -> Nip05Outcome {
    let result = verifier.verify(d, pubkey_hex).await;
    let line = format_nip05_line(&result);
    let abort = nip05_abort_message(mode, &result, d, pubkey_hex);
    Nip05Outcome {
        result,
        line,
        abort,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpfsStage {
    pub cid: String,
    pub size: u64,
    pub path: String,
}

pub async fn add_and_measure(
    ipfs: &IpfsClient,
    layout: &MfsLayout,
    pubkey_hex: &str,
    d: &str,
    created_at: u64,
    dir: &Path,
) -> Result<IpfsStage> {
    let path = layout.publish_version(pubkey_hex, d, created_at);
    let cid = nostr::canonical_cid(&ipfs.add_dir(dir, &path).await?)
        .context("Kubo returned an invalid cid")?;
    // add with pin=false does not hold Kubo's GC lock, so a GC during the add
    // could drop blocks before they were linked into MFS.
    let size = ipfs
        .dag_size_local(&[cid.as_str()])
        .await
        .context("added content is not complete in Kubo")?;
    Ok(IpfsStage { cid, size, path })
}

pub struct SiteAnnouncement<'a> {
    pub site_event_kind: u16,
    pub d: &'a str,
    pub cid: &'a str,
    pub url: Option<&'a str>,
    pub size: u64,
    pub title: Option<&'a str>,
    pub message: Option<&'a str>,
    pub created_at: Timestamp,
}

pub async fn sign_and_send(
    relay: &RelayClient,
    announcement: &SiteAnnouncement<'_>,
) -> Result<Vec<RelaySendResult>> {
    let builder = build_site_event_builder(
        announcement.site_event_kind,
        announcement.d,
        announcement.cid,
        announcement.url,
        Some(announcement.size),
        announcement.title,
        announcement.message,
    )
    .custom_created_at(announcement.created_at);
    let event = relay.sign(builder).await.context("signing site event")?;
    let output = relay.publish_to_relays(&event).await?;
    Ok(nostr::relay_send_results(relay.relays(), &output))
}

/// Site `d`/`url`/`title` validation shared by the CLI and the dashboard API,
/// which report the same checks under different flag/field names and error
/// types.
#[derive(Debug)]
pub enum SiteFieldError {
    InvalidD(anyhow::Error),
    InvalidUrl(String),
    InvalidTitle,
}

pub fn validate_site_fields(d: &str, url: Option<&str>) -> Result<(), SiteFieldError> {
    nostr::validate_d_tag(d).map_err(SiteFieldError::InvalidD)?;
    if let Some(url) = url
        && !nostr::valid_http_url(url)
    {
        return Err(SiteFieldError::InvalidUrl(url.to_string()));
    }
    Ok(())
}

/// An empty or whitespace-only title is treated as absent, not invalid.
pub fn normalize_title(title: Option<&str>) -> Result<Option<&str>, SiteFieldError> {
    let Some(title) = title else {
        return Ok(None);
    };
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !nostr::valid_title(trimmed) {
        return Err(SiteFieldError::InvalidTitle);
    }
    Ok(Some(trimmed))
}

pub fn resolve_nip05_mode(nip05_override: Option<&str>, default: Nip05Mode) -> Result<Nip05Mode> {
    match nip05_override {
        Some(v) => config::parse_nip05_mode(v),
        None => Ok(default),
    }
}

pub async fn run(
    config: Config,
    d: String,
    url: Option<String>,
    dir: &Path,
    nip05_override: Option<String>,
    title: Option<String>,
    message: Option<String>,
) -> Result<()> {
    if let Err(e) = validate_site_fields(&d, url.as_deref()) {
        return Err(match e {
            SiteFieldError::InvalidD(err) => err.context("invalid --site"),
            SiteFieldError::InvalidUrl(url) => {
                anyhow::anyhow!("invalid --url: {url} is not an http or https URL")
            }
            SiteFieldError::InvalidTitle => unreachable!(),
        });
    }
    let title = match normalize_title(title.as_deref()) {
        Ok(title) => title,
        Err(SiteFieldError::InvalidTitle) => {
            anyhow::bail!(
                "invalid --title: must not exceed 256 bytes and must not contain control characters"
            )
        }
        Err(_) => unreachable!(),
    };
    let nip05_mode = resolve_nip05_mode(nip05_override.as_deref(), config.publish.nip05)
        .context("invalid --nip05")?;

    println!("Site: {d}");
    if let Some(url) = &url {
        println!("URL: {url}");
    }
    if let Some(title) = &title {
        println!("Title: {title}");
    }
    if let Some(message) = &message {
        println!("Message: {message}");
    }

    let signer = Signer::require(&config)?;
    let pubkey_hex = signer.public_key().to_hex();

    if nip05_mode != Nip05Mode::Off {
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

    let ipfs = IpfsClient::new(config.ipfs_api_url()?);
    let layout = MfsLayout::new(config.ipfs.mfs_root.clone());
    let created_at = Timestamp::now();
    let stage = add_and_measure(&ipfs, &layout, &pubkey_hex, &d, created_at.as_secs(), dir).await?;
    println!("  CID: {}", stage.cid);
    println!("  \u{2713} added to {}", stage.path);
    println!("  Size: {} bytes", stage.size);

    println!();
    println!("Nostr");

    if signer.is_remote() {
        println!("  waiting for the signer app to sign the site event...");
    }
    let relay = RelayClient::connect(signer, &config.nostr.relays).await?;
    let send_result = sign_and_send(
        &relay,
        &SiteAnnouncement {
            site_event_kind: config.nostr.site_event_kind,
            d: &d,
            cid: &stage.cid,
            url: url.as_deref(),
            size: stage.size,
            title,
            message: message.as_deref(),
            created_at,
        },
    )
    .await;
    let results = match send_result {
        Ok(results) => results,
        Err(e) => {
            relay.shutdown().await;
            return Err(e);
        }
    };

    nostr::print_relay_send_result_lines(&results);
    relay.shutdown().await;
    if !results.iter().any(|r| r.ok) {
        anyhow::bail!("no relay accepted the site event; old versions were kept");
    }

    println!();
    println!("Old versions (keeping {})", config.publish.keep_versions);
    let site_path = layout.publish_site(&pubkey_hex, &d);
    let prune = prune_old_versions_collect(&ipfs, &site_path, config.publish.keep_versions).await;
    print_prune_lines(&site_path, &prune);

    println!();
    println!("Published.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_to_prune_keeps_the_newest_numeric_entries() {
        let names: Vec<String> = ["100", "300", "junk", "200", "50"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(versions_to_prune(&names, 2), vec!["100", "50"]);
        assert!(versions_to_prune(&names, 4).is_empty());
    }

    #[test]
    fn normalize_title_treats_absent_and_whitespace_only_as_none() {
        assert!(normalize_title(None).unwrap().is_none());
        assert!(normalize_title(Some("")).unwrap().is_none());
        assert!(normalize_title(Some("   \t  ")).unwrap().is_none());
    }

    #[test]
    fn normalize_title_trims_and_keeps_a_valid_title() {
        assert_eq!(
            normalize_title(Some("  My Site  ")).unwrap(),
            Some("My Site")
        );
    }

    #[test]
    fn normalize_title_rejects_oversized_or_control_titles() {
        let long = "a".repeat(257);
        assert!(matches!(
            normalize_title(Some(&long)),
            Err(SiteFieldError::InvalidTitle)
        ));
        assert!(matches!(
            normalize_title(Some("bad\ntitle")),
            Err(SiteFieldError::InvalidTitle)
        ));
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
            nip05::VerificationResult::Error(
                "boom".to_string(),
                nip05::ErrorCategory::InvalidResponse,
            ),
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
        let fake = FakeNip05(nip05::VerificationResult::Error(
            "timeout".to_string(),
            nip05::ErrorCategory::Timeout,
        ));
        let outcome = check_nip05(&fake, Nip05Mode::Require, "example.com", "abc123").await;
        assert!(outcome.abort.is_some());
    }

    #[tokio::test]
    async fn off_mode_never_aborts_even_on_mismatch() {
        let fake = FakeNip05(nip05::VerificationResult::Mismatch);
        let outcome = check_nip05(&fake, Nip05Mode::Off, "example.com", "abc123").await;
        assert!(outcome.abort.is_none());
    }

    #[test]
    fn prune_outcome_summarizes_list_and_removal_failures() {
        let list_failed = PruneOutcome {
            attempts: Vec::new(),
            list_error: Some("boom".to_string()),
        };
        assert_eq!(
            list_failed.error_summary().as_deref(),
            Some("could not list old versions: boom")
        );
        assert!(list_failed.pruned().is_empty());

        let mixed = PruneOutcome {
            attempts: vec![
                PruneAttempt::Removed("100".to_string()),
                PruneAttempt::Failed("50".to_string(), "denied".to_string()),
            ],
            list_error: None,
        };
        assert_eq!(mixed.pruned(), vec!["100"]);
        assert_eq!(mixed.error_summary().as_deref(), Some("50: denied"));

        let clean = PruneOutcome {
            attempts: vec![PruneAttempt::Removed("100".to_string())],
            list_error: None,
        };
        assert!(clean.error_summary().is_none());
    }
}
