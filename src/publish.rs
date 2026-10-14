use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::{self, CheckMode, Config};
use crate::format::Sanitized;
use crate::ipfs::{IpfsClient, SiteEntry, SiteListing};
use crate::mfs::MfsLayout;
use crate::nip05::{self, Nip05Verify};
use crate::nostr::{self, RelayClient, RelaySendResult, build_site_event_builder};
use crate::signer::Signer;

mod checks;
mod clock;
pub mod dry_run;
pub mod links;
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
        println!("  ! could not list old versions: {}", Sanitized(err));
        return;
    }
    for attempt in &outcome.attempts {
        match attempt {
            PruneAttempt::Removed(name) => {
                println!("  \u{2713} removed {site_path}/{}", Sanitized(name))
            }
            PruneAttempt::Failed(name, err) => {
                println!(
                    "  ! could not remove {site_path}/{}: {}",
                    Sanitized(name),
                    Sanitized(err)
                )
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

    pub async fn clocks_only(relays: &[String], previous_unknown: String) -> Self {
        Self {
            previous: Err(previous_unknown),
            relay_offsets: clock::probe_relay_offsets(relays).await,
        }
    }

    pub fn relays_answered(&self) -> usize {
        self.relay_offsets.len()
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
    Ok(date_version(site_path, newest, relays)?)
}

fn date_version(
    site_path: &str,
    newest: Option<u64>,
    relays: &RelayState,
) -> Result<u64, ClockError> {
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
    let mut created_at = plan_version(ipfs, &site_path, relays).await?;
    let added = ipfs.add_site(site).await?;
    let cid = nostr::canonical_cid(&added).context("Kubo returned an invalid CID")?;
    let path = loop {
        let path = layout.publish_version(pubkey_hex, d, created_at);
        if ipfs
            .mfs_place(&cid, &path)
            .await
            .with_context(|| format!("placing {cid} at {path}"))?
        {
            break path;
        }
        created_at += 1;
    };
    let version = StagedVersion::new(ipfs, path);
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

pub async fn post_site_note(
    relay: &RelayClient,
    announcement: &SiteAnnouncement<'_>,
) -> Result<Vec<RelaySendResult>> {
    let url = announcement
        .url
        .context("a note needs the URL of the site")?;
    let content = nostr::site_note_content(announcement.title, url, announcement.message);
    let builder = nostr::build_site_note_builder(
        announcement.site_event_kind,
        &relay.public_key(),
        announcement.d,
        &content,
    );
    let event = relay.sign(builder).await.context("signing the note")?;
    send_site_event(relay, &event).await
}

pub const NO_RELAY_ACCEPTED_NOTE: &str = "no relay accepted the note";

pub fn check_note_request(note: bool, url: Option<&str>) -> Result<(), &'static str> {
    if note && url.is_none() {
        return Err("needs a URL");
    }
    Ok(())
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
            *error = nostr::with_rejection_hint(error, FUTURE_REJECTION_HINT);
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
    pub check_links: Option<String>,
    pub check_unchanged: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    pub nip05: CheckMode,
    pub check_dotfiles: CheckMode,
    pub check_size: CheckMode,
    pub check_links: CheckMode,
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
        check_links: one("check-links", &overrides.check_links, defaults.check_links)?,
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

struct Prepared<'a> {
    title: Option<&'a str>,
    modes: Modes,
}

fn prepare<'a>(config: &Config, dir: &Path, request: &'a Request) -> Result<Prepared<'a>> {
    let url = request.url.as_deref();
    let title = check_arguments(
        &request.d,
        url,
        request.title.as_deref(),
        request.message.as_deref(),
    )?;
    check_note_request(request.note, url).map_err(|e| anyhow::anyhow!("invalid --note: {e}"))?;
    let modes = resolve_modes(&request.modes, &config.publish)
        .map_err(|(name, e)| e.context(format!("invalid --{name}")))?;
    refuse_protected_paths(dir, config)?;
    Ok(Prepared { title, modes })
}

fn print_section<S: AsRef<str>>(title: &str, lines: impl IntoIterator<Item = S>) {
    println!();
    println!("{title}");
    for line in lines {
        println!("  {}", Sanitized(line.as_ref()));
    }
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
    print_section("NIP-05", [&outcome.line]);
    if let Some(msg) = outcome.abort {
        anyhow::bail!(msg);
    }
    Ok(())
}

pub async fn scan_links(
    site: SiteListing,
    mode: CheckMode,
    url: Option<&str>,
) -> Result<(SiteListing, Option<links::LinkReport>)> {
    if mode == CheckMode::Off {
        return Ok((site, None));
    }
    let (site, report) = links::scan_async(site, url.map(str::to_string)).await?;
    Ok((site, Some(report)))
}

async fn read_and_check(
    dir: &Path,
    modes: &Modes,
    url: Option<&str>,
    dotfiles_allow: &[String],
) -> Result<(SiteListing, Vec<SiteEntry>, LocalChecks)> {
    let site = SiteListing::read_async(dir).await?;
    let (site, links) = scan_links(site, modes.check_links, url).await?;
    let entries = site.entries();
    let local = LocalChecks::evaluate(
        &entries,
        modes.check_dotfiles,
        modes.check_size,
        modes.check_links,
        links,
        dotfiles_allow,
    );
    Ok((site, entries, local))
}

fn print_local_checks(local: &LocalChecks) {
    if !local.all_off() {
        print_section("Checks", local.lines());
    }
}

fn run_local_checks(local: &LocalChecks) -> Result<()> {
    print_local_checks(local);
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
    print_section("New files", previous_files.lines(&new_files));
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
    print_section("Previous version", [unchanged.line()]);
    unchanged.stops_publish()
}

async fn announce(
    relay: RelayClient,
    remote_signer: bool,
    version: StagedVersion,
    announcement: &SiteAnnouncement<'_>,
    note: bool,
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
    let results = match send_site_event(&relay, &event).await {
        Ok(results) => results,
        Err(e) => {
            relay.shutdown().await;
            return Err(e);
        }
    };

    nostr::print_relay_send_result_lines(&results);
    if !results.iter().any(|r| r.ok) {
        relay.shutdown().await;
        anyhow::bail!(NO_RELAY_ACCEPTED);
    }
    if note {
        announce_note(&relay, remote_signer, announcement).await;
    }
    relay.shutdown().await;
    Ok(())
}

async fn announce_note(
    relay: &RelayClient,
    remote_signer: bool,
    announcement: &SiteAnnouncement<'_>,
) {
    println!();
    println!("Note");
    if remote_signer {
        println!("  waiting for the signer app to sign the note...");
    }
    match post_site_note(relay, announcement).await {
        Ok(results) => {
            nostr::print_relay_send_result_lines(&results);
            if !results.iter().any(|r| r.ok) {
                println!("  ! {NO_RELAY_ACCEPTED_NOTE}");
            }
        }
        Err(e) => println!(
            "  ! could not post the note: {}",
            Sanitized(&format!("{e:#}"))
        ),
    }
}

pub const NO_RELAY_ACCEPTED: &str = "no relay accepted the site event; old versions were kept";

pub struct Request {
    pub d: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub note: bool,
    pub modes: ModeOverrides,
    pub yes: bool,
}

pub async fn run(config: Config, dir: &Path, request: Request) -> Result<()> {
    let Prepared { title, modes } = prepare(&config, dir, &request)?;
    let d = &request.d;
    let url = request.url.as_deref();
    let message = request.message.as_deref();
    let (note, yes) = (request.note, request.yes);

    print_header(d, url, title, message);

    let signer = Signer::require(&config)?;
    let pubkey_hex = signer.public_key().to_hex();

    run_nip05_check(modes.nip05, d, &pubkey_hex).await?;
    let (site, entries, local) =
        read_and_check(dir, &modes, url, &config.publish.dotfiles_allow).await?;
    run_local_checks(&local)?;

    let ipfs = config.ipfs_client().await?;
    let remote_signer = signer.is_remote();
    let relay = RelayClient::connect(signer, &config.nostr.relays).await?;
    let site_event_kind = config.nostr.site_event_kind;
    let relays = RelayState::fetch(&relay, site_event_kind, d).await;
    let layout = MfsLayout::new(config.ipfs.mfs_root.clone());
    let site_path = layout.publish_site(&pubkey_hex, d);
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

    let stage = match add_and_measure(&ipfs, &layout, &pubkey_hex, d, &relays, site).await {
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
            d,
            cid: &stage.cid,
            url,
            size: stage.size,
            title,
            message,
            created_at,
        },
        note,
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
mod tests;
