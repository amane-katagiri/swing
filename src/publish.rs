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
mod clock;
mod new_files;
mod staged;

pub use checks::{
    DASHBOARD_UPLOAD_DIR, LISTED_DOTFILES, LocalChecks, SIZE_GUIDELINE, UnchangedOutcome,
    UnchangedStatus, find_dotfiles, refuse_protected_paths,
};
pub use clock::ClockError;
pub use new_files::PreviousFiles;
use new_files::count_new_files;
pub use staged::StagedVersion;

fn version_name(name: &str) -> Option<u64> {
    name.parse::<u64>()
        .ok()
        .filter(|version| version.to_string() == name)
}

fn versions_to_prune(names: &[String], current: u64, keep: usize) -> Vec<String> {
    let mut versions: Vec<(u64, &String)> = names
        .iter()
        .filter_map(|name| version_name(name).map(|t| (t, name)))
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

fn newest_version(names: &[String]) -> Option<u64> {
    names.iter().filter_map(|name| version_name(name)).max()
}

async fn list_newest_version(ipfs: &IpfsClient, site_path: &str) -> Result<Option<u64>> {
    let names: Vec<String> = ipfs
        .mfs_list(site_path)
        .await
        .with_context(|| format!("listing {site_path}"))?
        .into_iter()
        .map(|e| e.name)
        .collect();
    Ok(newest_version(&names))
}

pub struct RelayState {
    pub previous: Result<Option<nostr::SiteEvent>, String>,
    relay_offsets: Vec<(String, i64)>,
}

impl RelayState {
    pub async fn fetch(relay: &RelayClient, site_event_kind: u16, d: &str) -> Self {
        let (previous, relay_offsets) = tokio::join!(
            relay.fetch_own_latest_site(site_event_kind, d),
            clock::probe_relay_offsets(relay.relays())
        );
        Self {
            previous: previous.map_err(|e| format!("{e:#}")),
            relay_offsets,
        }
    }

    fn previous_created_at(&self) -> Option<u64> {
        self.previous
            .as_ref()
            .ok()
            .and_then(|previous| previous.as_ref())
            .map(|previous| previous.created_at)
    }
}

pub async fn plan_version(ipfs: &IpfsClient, site_path: &str, relays: &RelayState) -> Result<u64> {
    let newest = list_newest_version(ipfs, site_path).await?;
    let now = Timestamp::now().as_secs();
    let created_at = clock::version_time(site_path, newest, relays.previous_created_at(), now)?;
    clock::refuse_clock_ahead(&relays.relay_offsets)?;
    Ok(created_at)
}

pub struct IpfsStage {
    pub cid: String,
    pub size: u64,
    pub created_at: Timestamp,
    pub version: StagedVersion,
}

pub async fn add_and_measure(
    ipfs: &IpfsClient,
    layout: &MfsLayout,
    pubkey_hex: &str,
    d: &str,
    relays: &RelayState,
    site: SiteListing,
) -> Result<IpfsStage> {
    let site_path = layout.publish_site(pubkey_hex, d);
    let created_at = plan_version(ipfs, &site_path, relays).await?;
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
    Ok(IpfsStage {
        cid,
        size,
        created_at: Timestamp::from_secs(created_at),
        version,
    })
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

pub async fn sign_site_event(
    relay: &RelayClient,
    announcement: &SiteAnnouncement<'_>,
) -> Result<Event> {
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
    relay.sign(builder).await.context("signing site event")
}

pub const FUTURE_REJECTION_HINT: &str =
    "the relay thinks the event is dated in the future; check your clock";

fn rejected_as_future(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    let words: Vec<&str> = m
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let has = |word: &str| words.contains(&word);
    let follows = |first: &str, second: &str| words.windows(2).any(|w| w == [first, second]);
    let too_early = ["early", "old", "past"].into_iter().any(has);
    has("future")
        || follows("too", "late")
        || follows("too", "new")
        || (has("created_at") && has("too") && !too_early)
}

fn with_rejection_hints(mut results: Vec<RelaySendResult>) -> Vec<RelaySendResult> {
    for result in &mut results {
        if let Some(error) = &mut result.error
            && !result.ok
            && rejected_as_future(error)
        {
            *error = format!(
                "{} ({FUTURE_REJECTION_HINT})",
                nostr::cap_rejection_reason(error)
            );
        }
    }
    results
}

pub async fn send_site_event(relay: &RelayClient, event: &Event) -> Result<Vec<RelaySendResult>> {
    let output = relay.publish_to_relays(event).await?;
    Ok(with_rejection_hints(nostr::relay_send_results(
        relay.relays(),
        &output,
    )))
}

pub fn no_relay_accepted(results: &[RelaySendResult]) -> anyhow::Error {
    let reasons: Vec<String> = results
        .iter()
        .filter_map(|r| r.error.as_ref().map(|e| format!("{}: {e}", r.relay)))
        .collect();
    if reasons.is_empty() {
        anyhow::anyhow!(NO_RELAY_ACCEPTED)
    } else {
        anyhow::anyhow!("{NO_RELAY_ACCEPTED} ({})", reasons.join("; "))
    }
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

pub fn check_unchanged(mode: CheckMode, relays: &RelayState, cid: &str) -> UnchangedOutcome {
    let previous = relays.previous.clone().map_err(anyhow::Error::msg);
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

fn report_unchanged(mode: CheckMode, relays: &RelayState, cid: &str) -> bool {
    if mode == CheckMode::Off {
        return false;
    }
    let unchanged = check_unchanged(mode, relays, cid);
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
    let event = match sign_site_event(&relay, announcement).await {
        Ok(event) => event,
        Err(e) => {
            relay.shutdown().await;
            return Err(version.fail(e).await);
        }
    };
    version.keep();
    let sent = send_site_event(&relay, &event).await;
    relay.shutdown().await;
    let results = sent?;

    nostr::print_relay_send_result_lines(&results);
    if !results.iter().any(|r| r.ok) {
        anyhow::bail!(NO_RELAY_ACCEPTED);
    }
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
    let relays = RelayState::fetch(&relay, site_event_kind, &d).await;
    let layout = MfsLayout::new(config.ipfs.mfs_root.clone());
    let site_path = layout.publish_site(&pubkey_hex, &d);
    let checked = match plan_version(&ipfs, &site_path, &relays).await {
        Ok(_) => confirm_new_files(&ipfs, &relays.previous, &entries, yes).await,
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
        relay.shutdown().await;
        return Err(e);
    }

    println!();
    println!("IPFS");

    let stage = match add_and_measure(&ipfs, &layout, &pubkey_hex, &d, &relays, site).await {
        Ok(stage) => stage,
        Err(e) => {
            relay.shutdown().await;
            return Err(e);
        }
    };
    let created_at = stage.created_at;
    println!("  CID: {}", stage.cid);
    println!("  \u{2713} added to {}", stage.version.path());
    println!("  Size: {} bytes", stage.size);

    if report_unchanged(modes.check_unchanged, &relays, &stage.cid) {
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
    fn only_canonical_decimal_names_are_versions() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(newest_version(&names(&[])), None);
        assert_eq!(newest_version(&names(&["100", "junk"])), Some(100));
        assert_eq!(newest_version(&names(&["100", "0500", "+600"])), Some(100));
        assert_eq!(newest_version(&names(&["0"])), Some(0));
        assert_eq!(newest_version(&names(&["junk", "00"])), None);
    }

    struct FakeKubo {
        ipfs: IpfsClient,
        added: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    async fn fake_files_ls(answer: String) -> FakeKubo {
        let added = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let added_flag = added.clone();
        let router = axum::Router::new()
            .route(
                "/api/v0/files/ls",
                axum::routing::post(move || async move {
                    if answer == "missing" {
                        let body = r#"{"Message":"file does not exist","Code":0,"Type":"error"}"#;
                        return (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            body.to_string(),
                        );
                    }
                    (axum::http::StatusCode::OK, answer)
                }),
            )
            .fallback(move || async move {
                added_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            });
        let addr = crate::test_support::serve_router(router).await;
        FakeKubo {
            ipfs: IpfsClient::new(format!("http://{addr}")),
            added,
        }
    }

    fn ls_answer(names: &[u64]) -> String {
        let entries: Vec<String> = names
            .iter()
            .map(|n| format!(r#"{{"Name":"{n}","Type":1,"Hash":"bafy"}}"#))
            .collect();
        format!(r#"{{"Entries":[{}]}}"#, entries.join(","))
    }

    #[tokio::test]
    async fn list_newest_version_reads_the_site_dir() {
        let kubo = fake_files_ls(ls_answer(&[499, 500])).await;
        assert_eq!(
            list_newest_version(&kubo.ipfs, "/swing/publish/k/s")
                .await
                .unwrap(),
            Some(500)
        );
        let kubo = fake_files_ls("missing".to_string()).await;
        assert_eq!(
            list_newest_version(&kubo.ipfs, "/swing/publish/k/s")
                .await
                .unwrap(),
            None
        );
    }

    fn relay_state(previous: Option<u64>, relay_offsets: Vec<(String, i64)>) -> RelayState {
        let previous = previous.map(|created_at| nostr::SiteEvent {
            pubkey: Keys::generate().public_key(),
            d: "example.com".to_string(),
            cid: "bafy".to_string(),
            url: None,
            size: None,
            title: None,
            message: None,
            created_at,
            id: EventId::from_byte_array([0; 32]),
        });
        RelayState {
            previous: Ok(previous),
            relay_offsets,
        }
    }

    async fn refused_stage(newest: u64, relays: RelayState) -> (anyhow::Error, bool) {
        let kubo = fake_files_ls(ls_answer(&[100, newest])).await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
        let site = SiteListing::read_async(dir.path()).await.unwrap();
        let layout = MfsLayout::new("/swing".to_string());
        let Err(err) =
            add_and_measure(&kubo.ipfs, &layout, "k", "example.com", &relays, site).await
        else {
            panic!("add_and_measure should refuse");
        };
        (err, kubo.added.load(std::sync::atomic::Ordering::SeqCst))
    }

    #[tokio::test]
    async fn add_and_measure_refuses_a_future_version_before_adding() {
        let future = Timestamp::now().as_secs() + nostr::MAX_FUTURE_SKEW + 3600;
        let (err, added) = refused_stage(future, relay_state(None, Vec::new())).await;
        assert!(err.downcast_ref::<ClockError>().is_some(), "{err:#}");
        let rm = format!("ipfs files rm -r /swing/publish/k/example.com/{future}");
        assert!(err.to_string().contains(&rm), "{err}");
        assert!(!added);
    }

    #[tokio::test]
    async fn add_and_measure_refuses_a_clock_ahead_of_every_relay_before_adding() {
        let skew = nostr::MAX_FUTURE_SKEW as i64;
        let offsets = vec![
            ("wss://a".to_string(), skew + 3600),
            ("wss://b".to_string(), skew + 60),
        ];
        let (err, added) = refused_stage(100, relay_state(None, offsets)).await;
        assert!(err.downcast_ref::<ClockError>().is_some(), "{err:#}");
        assert!(
            err.to_string()
                .contains("ahead of every relay that answered (even wss://b,"),
            "{err}"
        );
        assert!(!added);
    }

    #[tokio::test]
    async fn plan_version_ignores_one_lagging_relay_and_never_refuses_over_the_previous() {
        let kubo = fake_files_ls(ls_answer(&[100])).await;
        let now = Timestamp::now().as_secs();
        let relays = RelayState {
            previous: Err("relay down".to_string()),
            relay_offsets: vec![
                ("wss://a".to_string(), 1_000_000_000),
                ("wss://b".to_string(), 0),
            ],
        };
        let created_at = plan_version(&kubo.ipfs, "/swing/publish/k/s", &relays)
            .await
            .unwrap();
        assert!(created_at >= now, "{created_at}");
        let relays = relay_state(Some(now + 600), Vec::new());
        assert_eq!(
            plan_version(&kubo.ipfs, "/swing/publish/k/s", &relays)
                .await
                .unwrap(),
            now + 601
        );
        let relays = relay_state(Some(now + 3600), Vec::new());
        let created_at = plan_version(&kubo.ipfs, "/swing/publish/k/s", &relays)
            .await
            .unwrap();
        assert!(created_at < now + 3600, "{created_at}");
    }

    #[test]
    fn the_future_hint_survives_a_long_rejection_reason() {
        let long = format!(
            "invalid: created_at too far in the future {}",
            "x".repeat(2000)
        );
        let results = with_rejection_hints(vec![RelaySendResult {
            relay: "wss://a".to_string(),
            ok: false,
            error: Some(long),
        }]);
        assert!(FUTURE_REJECTION_HINT.chars().count() + 3 <= nostr::MAX_REJECTION_HINT_CHARS);
        let error = results[0].error.as_deref().unwrap();
        assert!(
            error.ends_with(&format!(" ({FUTURE_REJECTION_HINT})")),
            "{error}"
        );
        let line = nostr::rejection_line("wss://a", error);
        assert!(
            line.ends_with(&format!(" ({FUTURE_REJECTION_HINT})")),
            "{line}"
        );
    }

    #[test]
    fn versions_to_prune_keeps_the_current_and_the_newest_others() {
        let names: Vec<String> = ["100", "300", "junk", "200", "50", "0020", "+10"]
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

    #[test]
    fn future_rejections_are_recognized() {
        for message in [
            "invalid: created_at too late",
            "invalid: event creation date is too far in the FUTURE",
            "blocked: created_at is too far ahead",
            "invalid: event is too new",
            "invalid: created_at too far off threshold",
        ] {
            assert!(rejected_as_future(message), "{message}");
        }
        for message in [
            "invalid: created_at too early",
            "invalid: created_at is too old",
            "blocked: pubkey not allowed",
            "rate-limited: slow down",
            "error: the futures tool took too long",
            "invalid: created_at tool mismatch",
            "",
        ] {
            assert!(!rejected_as_future(message), "{message}");
        }
    }

    fn sent(relay: &str, ok: bool, error: Option<&str>) -> RelaySendResult {
        RelaySendResult {
            relay: relay.to_string(),
            ok,
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn only_future_rejections_get_the_clock_hint() {
        let results = with_rejection_hints(vec![
            sent("wss://a", false, Some("invalid: created_at too late")),
            sent("wss://b", false, Some("blocked: not allowed")),
            sent("wss://c", true, None),
            sent("wss://d", false, None),
        ]);
        assert_eq!(
            results[0].error.as_deref(),
            Some(
                "invalid: created_at too late (the relay thinks the event is dated in the future; check your clock)"
            )
        );
        assert_eq!(results[1].error.as_deref(), Some("blocked: not allowed"));
        assert!(results[0..2].iter().all(|r| !r.ok));
        assert_eq!(results[2], sent("wss://c", true, None));
        assert_eq!(results[3], sent("wss://d", false, None));
    }

    #[test]
    fn no_relay_accepted_lists_the_reasons() {
        assert_eq!(
            no_relay_accepted(&[sent("wss://a", false, None)]).to_string(),
            NO_RELAY_ACCEPTED
        );
        assert_eq!(
            no_relay_accepted(&[
                sent("wss://a", false, Some("x")),
                sent("wss://b", false, None),
                sent("wss://c", false, Some("y")),
            ])
            .to_string(),
            format!("{NO_RELAY_ACCEPTED} (wss://a: x; wss://c: y)")
        );
    }
}
