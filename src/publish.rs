use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::{self, CheckMode, Config};
use crate::ipfs::{IpfsClient, SiteEntry, SiteListing};
use crate::mfs::MfsLayout;
use crate::nip05::{self, Nip05Verify};
use crate::nostr::{self, RelayClient, RelaySendResult, build_site_event_builder};
use crate::signer::Signer;

mod checks;
mod new_files;
mod staged;

pub use checks::{
    DASHBOARD_UPLOAD_DIR, LISTED_DOTFILES, LocalChecks, SIZE_GUIDELINE, UnchangedOutcome,
    UnchangedStatus, find_dotfiles, refuse_protected_paths,
};
pub use new_files::PreviousFiles;
use new_files::count_new_files;
pub use staged::StagedVersion;

fn versions_to_prune(names: &[String], current: u64, keep: usize) -> Vec<String> {
    let mut versions: Vec<(u64, &String)> = names
        .iter()
        .filter_map(|name| name.parse::<u64>().ok().map(|t| (t, name)))
        .filter(|&(t, _)| t != current)
        .collect();
    versions.sort_by(|a, b| b.cmp(a));
    versions
        .into_iter()
        .skip(keep.saturating_sub(1))
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
    current: u64,
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
    for name in versions_to_prune(&names, current, keep) {
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
    mode: CheckMode,
    result: &nip05::VerificationResult,
    d: &str,
    pubkey_hex: &str,
) -> Option<String> {
    if mode != CheckMode::Require || result.is_verified() {
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
    mode: CheckMode,
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

pub struct IpfsStage {
    pub cid: String,
    pub size: u64,
    pub version: StagedVersion,
}

pub async fn add_and_measure(
    ipfs: &IpfsClient,
    layout: &MfsLayout,
    pubkey_hex: &str,
    d: &str,
    created_at: u64,
    site: SiteListing,
) -> Result<IpfsStage> {
    let path = layout.publish_version(pubkey_hex, d, created_at);
    let added = ipfs.add_site(site, &path).await?;
    let version = StagedVersion::new(ipfs, path);
    let cid = match nostr::canonical_cid(&added).context("Kubo returned an invalid cid") {
        Ok(cid) => cid,
        Err(e) => return Err(version.fail(e).await),
    };
    // add with pin=false doesn't hold Kubo's GC lock, so this verifies nothing was dropped before MFS linked it.
    let size = match ipfs
        .dag_size_local(&[cid.as_str()])
        .await
        .context("added content is not complete in Kubo")
    {
        Ok(size) => size,
        Err(e) => return Err(version.fail(e).await),
    };
    Ok(IpfsStage { cid, size, version })
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
        &nostr::SiteFields {
            d: announcement.d,
            cid: announcement.cid,
            url: announcement.url,
            size: Some(announcement.size),
            title: announcement.title,
            message: announcement.message,
        },
    )
    .custom_created_at(announcement.created_at);
    let event = relay.sign(builder).await.context("signing site event")?;
    let output = relay.publish_to_relays(&event).await?;
    Ok(nostr::relay_send_results(relay.relays(), &output))
}

#[derive(Debug)]
pub enum SiteFieldError {
    InvalidD(anyhow::Error),
    InvalidUrl(String),
}

#[derive(Debug)]
pub struct InvalidTitle;

pub fn validate_site_fields(d: &str, url: Option<&str>) -> Result<(), SiteFieldError> {
    nostr::validate_d_tag(d).map_err(SiteFieldError::InvalidD)?;
    if let Some(url) = url
        && !nostr::valid_http_url(url)
    {
        return Err(SiteFieldError::InvalidUrl(url.to_string()));
    }
    Ok(())
}

pub fn normalize_title(title: Option<&str>) -> Result<Option<&str>, InvalidTitle> {
    let Some(title) = title else {
        return Ok(None);
    };
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !nostr::valid_title(trimmed) {
        return Err(InvalidTitle);
    }
    Ok(Some(trimmed))
}

pub fn resolve_mode(mode_override: Option<&str>, default: CheckMode) -> Result<CheckMode> {
    match mode_override {
        Some(v) => config::parse_check_mode(v),
        None => Ok(default),
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModeOverrides {
    pub nip05: Option<String>,
    pub check_dotfiles: Option<String>,
    pub check_size: Option<String>,
    pub check_unchanged: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    pub nip05: CheckMode,
    pub check_dotfiles: CheckMode,
    pub check_size: CheckMode,
    pub check_unchanged: CheckMode,
}

pub fn resolve_modes(
    overrides: &ModeOverrides,
    defaults: &config::PublishConfig,
) -> Result<Modes, (&'static str, anyhow::Error)> {
    let one = |name: &'static str, value: &Option<String>, default: CheckMode| {
        resolve_mode(value.as_deref(), default).map_err(|e| (name, e))
    };
    Ok(Modes {
        nip05: one("nip05", &overrides.nip05, defaults.nip05)?,
        check_dotfiles: one(
            "check-dotfiles",
            &overrides.check_dotfiles,
            defaults.check_dotfiles,
        )?,
        check_size: one("check-size", &overrides.check_size, defaults.check_size)?,
        check_unchanged: one(
            "check-unchanged",
            &overrides.check_unchanged,
            defaults.check_unchanged,
        )?,
    })
}

pub async fn check_unchanged(
    relay: &RelayClient,
    mode: CheckMode,
    site_event_kind: u16,
    d: &str,
    cid: &str,
) -> UnchangedOutcome {
    if mode == CheckMode::Off {
        return UnchangedOutcome::off();
    }
    let previous = relay.fetch_own_latest_site(site_event_kind, d).await;
    UnchangedOutcome::decide(mode, previous, cid)
}

pub fn validate_message(message: &str) -> Result<(), String> {
    let max = nostr::budget::MAX_CONTENT_BYTES;
    if message.len() > max {
        return Err(format!(
            "must not exceed {max} bytes (got {} bytes)",
            message.len()
        ));
    }
    Ok(())
}

fn check_arguments<'a>(
    d: &str,
    url: Option<&str>,
    title: Option<&'a str>,
    message: Option<&str>,
) -> Result<Option<&'a str>> {
    if let Err(e) = validate_site_fields(d, url) {
        return Err(match e {
            SiteFieldError::InvalidD(err) => err.context("invalid --site"),
            SiteFieldError::InvalidUrl(url) => {
                anyhow::anyhow!("invalid --url: {url} is not an http or https URL")
            }
        });
    }
    let Ok(title) = normalize_title(title) else {
        anyhow::bail!(
            "invalid --title: must not exceed 256 bytes and must not contain control characters"
        )
    };
    if let Some(message) = message {
        validate_message(message).map_err(|e| anyhow::anyhow!("invalid --message: {e}"))?;
    }
    Ok(title)
}

fn print_header(d: &str, url: Option<&str>, title: Option<&str>, message: Option<&str>) {
    println!("Site: {d}");
    if let Some(url) = url {
        println!("URL: {url}");
    }
    if let Some(title) = title {
        println!("Title: {title}");
    }
    if let Some(message) = message {
        println!("Message: {message}");
    }
}

async fn run_nip05_check(mode: CheckMode, d: &str, pubkey_hex: &str) -> Result<()> {
    if mode == CheckMode::Off {
        return Ok(());
    }
    let verifier = nip05::HttpNip05Verifier::new();
    let outcome = check_nip05(&verifier, mode, d, pubkey_hex).await;

    println!();
    println!("NIP-05");
    println!("  {}", outcome.line);

    if let Some(msg) = outcome.abort {
        anyhow::bail!(msg);
    }
    Ok(())
}

fn run_local_checks(entries: &[SiteEntry], modes: &Modes, dotfiles_allow: &[String]) -> Result<()> {
    let local = LocalChecks::evaluate(
        entries,
        modes.check_dotfiles,
        modes.check_size,
        dotfiles_allow,
    );
    if local.all_off() {
        return Ok(());
    }
    println!();
    println!("Checks");
    for line in local.lines() {
        println!("  {line}");
    }
    if let Some(msg) = local.abort_message() {
        anyhow::bail!(msg);
    }
    Ok(())
}

async fn ask_to_publish(new_files: usize) -> Result<bool> {
    print!("  Publish with {}? [y/N] ", count_new_files(new_files));
    std::io::stdout().flush()?;
    let answer = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|_| line)
    })
    .await
    .context("prompt task panicked")??;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

async fn confirm_new_files(
    ipfs: &IpfsClient,
    previous: &Result<Option<nostr::SiteEvent>, String>,
    entries: &[SiteEntry],
    yes: bool,
) -> Result<()> {
    let previous_files = PreviousFiles::load(ipfs, previous).await;
    let new_files = previous_files.new_files(entries);
    println!();
    println!("New files");
    for line in previous_files.lines(&new_files) {
        println!("  {line}");
    }
    if new_files.is_empty() || yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "confirmation needed for {}: review them above and rerun with --yes",
            count_new_files(new_files.len())
        );
    }
    if !ask_to_publish(new_files.len()).await? {
        anyhow::bail!("cancelled; nothing was added or published");
    }
    Ok(())
}

fn report_unchanged(
    mode: CheckMode,
    previous: &Result<Option<nostr::SiteEvent>, String>,
    cid: &str,
) -> bool {
    if mode == CheckMode::Off {
        return false;
    }
    let previous = previous.clone().map_err(anyhow::Error::msg);
    let unchanged = UnchangedOutcome::decide(mode, previous, cid);
    println!();
    println!("Previous version");
    println!("  {}", unchanged.line());
    unchanged.stops_publish()
}

async fn announce(
    relay: RelayClient,
    remote_signer: bool,
    version: StagedVersion,
    announcement: &SiteAnnouncement<'_>,
) -> Result<()> {
    println!();
    println!("Nostr");

    if remote_signer {
        println!("  waiting for the signer app to sign the site event...");
    }
    let sent = sign_and_send(&relay, announcement).await;
    relay.shutdown().await;
    let results = match sent {
        Ok(results) => results,
        Err(e) => return Err(version.fail(e).await),
    };

    nostr::print_relay_send_result_lines(&results);
    if !results.iter().any(|r| r.ok) {
        return Err(version.fail(anyhow::anyhow!(NO_RELAY_ACCEPTED)).await);
    }
    version.keep();
    Ok(())
}

pub const NO_RELAY_ACCEPTED: &str = "no relay accepted the site event; old versions were kept";

pub struct Request {
    pub d: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub modes: ModeOverrides,
    pub yes: bool,
}

pub async fn run(config: Config, dir: &Path, request: Request) -> Result<()> {
    let Request {
        d,
        url,
        title,
        message,
        modes: overrides,
        yes,
    } = request;
    let url = url.as_deref();
    let message = message.as_deref();
    let title = check_arguments(&d, url, title.as_deref(), message)?;
    let modes = resolve_modes(&overrides, &config.publish)
        .map_err(|(name, e)| e.context(format!("invalid --{name}")))?;
    refuse_protected_paths(dir, &config)?;

    print_header(&d, url, title, message);

    let signer = Signer::require(&config)?;
    let pubkey_hex = signer.public_key().to_hex();

    run_nip05_check(modes.nip05, &d, &pubkey_hex).await?;
    let site = SiteListing::read_async(dir).await?;
    let entries = site.entries();
    run_local_checks(&entries, &modes, &config.publish.dotfiles_allow)?;

    let ipfs = config.ipfs_client().await?;
    let remote_signer = signer.is_remote();
    let relay = RelayClient::connect(signer, &config.nostr.relays).await?;
    let site_event_kind = config.nostr.site_event_kind;
    let previous = relay
        .fetch_own_latest_site(site_event_kind, &d)
        .await
        .map_err(|e| format!("{e:#}"));
    if let Err(e) = confirm_new_files(&ipfs, &previous, &entries, yes).await {
        relay.shutdown().await;
        return Err(e);
    }

    println!();
    println!("IPFS");

    let layout = MfsLayout::new(config.ipfs.mfs_root.clone());
    let created_at = Timestamp::now();
    let stage =
        match add_and_measure(&ipfs, &layout, &pubkey_hex, &d, created_at.as_secs(), site).await {
            Ok(stage) => stage,
            Err(e) => {
                relay.shutdown().await;
                return Err(e);
            }
        };
    println!("  CID: {}", stage.cid);
    println!("  \u{2713} added to {}", stage.version.path());
    println!("  Size: {} bytes", stage.size);

    if report_unchanged(modes.check_unchanged, &previous, &stage.cid) {
        relay.shutdown().await;
        let path = stage.version.path().to_string();
        stage.version.withdraw().await?;
        println!("  \u{2713} removed {path}");
        println!();
        println!("Unchanged; not published.");
        return Ok(());
    }

    announce(
        relay,
        remote_signer,
        stage.version,
        &SiteAnnouncement {
            site_event_kind,
            d: &d,
            cid: &stage.cid,
            url,
            size: stage.size,
            title,
            message,
            created_at,
        },
    )
    .await?;

    println!();
    println!("Old versions (keeping {})", config.publish.keep_versions);
    let site_path = layout.publish_site(&pubkey_hex, &d);
    let prune = prune_old_versions_collect(
        &ipfs,
        &site_path,
        created_at.as_secs(),
        config.publish.keep_versions,
    )
    .await;
    print_prune_lines(&site_path, &prune);

    println!();
    println!("Published.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_to_prune_keeps_the_current_and_the_newest_others() {
        let names: Vec<String> = ["100", "300", "junk", "200", "50"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(versions_to_prune(&names, 300, 2), vec!["100", "50"]);
        assert!(versions_to_prune(&names, 300, 4).is_empty());
    }

    #[test]
    fn versions_to_prune_never_removes_the_current_version() {
        let names: Vec<String> = ["100", "999999", "200"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(versions_to_prune(&names, 200, 1), vec!["999999", "100"]);
        assert_eq!(versions_to_prune(&names, 200, 2), vec!["100"]);
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
        assert!(matches!(normalize_title(Some(&long)), Err(InvalidTitle)));
        assert!(matches!(
            normalize_title(Some("bad\ntitle")),
            Err(InvalidTitle)
        ));
    }

    #[test]
    fn validate_message_allows_up_to_the_content_limit() {
        let at_limit = "a".repeat(nostr::budget::MAX_CONTENT_BYTES);
        assert!(validate_message(&at_limit).is_ok());
        let over = "あ".repeat(nostr::budget::MAX_CONTENT_BYTES / 3 + 1);
        assert!(validate_message(&over).unwrap_err().contains("4096 bytes"));
    }

    #[test]
    fn check_arguments_names_the_message_flag() {
        let over = "a".repeat(nostr::budget::MAX_CONTENT_BYTES + 1);
        let err = check_arguments("example.com", None, None, Some(&over)).unwrap_err();
        assert!(err.to_string().starts_with("invalid --message: "), "{err}");
        assert_eq!(
            check_arguments("example.com", None, Some(" t "), Some("note")).unwrap(),
            Some("t")
        );
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
            let outcome = check_nip05(&fake, CheckMode::Warn, "example.com", "abc123").await;
            assert!(outcome.abort.is_none());
        }
    }

    #[tokio::test]
    async fn require_mode_proceeds_only_when_verified() {
        let fake = FakeNip05(nip05::VerificationResult::Verified);
        let outcome = check_nip05(&fake, CheckMode::Require, "example.com", "abc123").await;
        assert!(outcome.abort.is_none());
        assert!(outcome.line.contains("verified"));
    }

    #[tokio::test]
    async fn require_mode_aborts_on_mismatch() {
        let fake = FakeNip05(nip05::VerificationResult::Mismatch);
        let outcome = check_nip05(&fake, CheckMode::Require, "example.com", "abc123").await;
        assert!(outcome.abort.is_some());
        assert!(outcome.abort.unwrap().contains("abc123"));
    }

    #[tokio::test]
    async fn require_mode_aborts_on_not_applicable_with_domain_specific_message() {
        let fake = FakeNip05(nip05::VerificationResult::NotApplicable);
        let outcome = check_nip05(&fake, CheckMode::Require, "not-a-domain", "abc123").await;
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
        let outcome = check_nip05(&fake, CheckMode::Require, "example.com", "abc123").await;
        assert!(outcome.abort.is_some());
    }

    #[tokio::test]
    async fn off_mode_never_aborts_even_on_mismatch() {
        let fake = FakeNip05(nip05::VerificationResult::Mismatch);
        let outcome = check_nip05(&fake, CheckMode::Off, "example.com", "abc123").await;
        assert!(outcome.abort.is_none());
    }

    #[test]
    fn resolve_modes_prefers_overrides_and_names_the_bad_one() {
        let defaults = config::build_config_from_str("", |_| None).unwrap().publish;
        let modes = resolve_modes(&ModeOverrides::default(), &defaults).unwrap();
        assert_eq!(
            modes,
            Modes {
                nip05: CheckMode::Warn,
                check_dotfiles: CheckMode::Require,
                check_size: CheckMode::Warn,
                check_unchanged: CheckMode::Require,
            }
        );
        let overrides = ModeOverrides {
            check_dotfiles: Some("off".into()),
            check_unchanged: Some(" WARN ".into()),
            ..Default::default()
        };
        let modes = resolve_modes(&overrides, &defaults).unwrap();
        assert_eq!(modes.check_dotfiles, CheckMode::Off);
        assert_eq!(modes.check_unchanged, CheckMode::Warn);
        let bad = ModeOverrides {
            check_size: Some("loud".into()),
            ..Default::default()
        };
        let (name, _) = resolve_modes(&bad, &defaults).unwrap_err();
        assert_eq!(name, "check-size");
    }

    #[tokio::test]
    async fn run_refuses_a_site_holding_the_state_dir_before_anything_else() {
        let site = tempfile::tempdir().unwrap();
        std::fs::write(site.path().join("index.html"), b"hi").unwrap();
        let mut config = config::build_config_from_str("", |_| None).unwrap();
        config.agent.state_dir = site.path().join("data");
        std::fs::create_dir(&config.agent.state_dir).unwrap();
        let request = Request {
            d: "example.com".into(),
            url: None,
            title: None,
            message: None,
            modes: ModeOverrides {
                nip05: Some("off".into()),
                check_dotfiles: Some("off".into()),
                check_size: Some("off".into()),
                check_unchanged: Some("off".into()),
            },
            yes: false,
        };
        let err = run(config, site.path(), request).await.unwrap_err();
        assert!(err.to_string().contains("[agent].state_dir"), "{err}");
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
