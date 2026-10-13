use std::path::Path;

use anyhow::Result;
use nostr_sdk::prelude::*;
use serde::Serialize;

use super::checks::total_size;
use super::links::{LinkReport, SITE_GUIDE_URL};
use super::{
    ClockError, LocalChecks, Modes, Prepared, PreviousFiles, RelayState, Request, UnchangedOutcome,
    UnchangedStatus, check_nip05, date_version, list_newest_version, prepare, print_header,
    print_local_checks, print_section, read_and_check,
};
use crate::config::{CheckMode, Config};
use crate::format::Sanitized;
use crate::ipfs::IpfsClient;
use crate::mfs::MfsLayout;
use crate::nip05::{self, Nip05Verify};
use crate::nostr::{self, RelayClient, SiteEvent};
use crate::signer::Signer;

pub const FORMAT: u32 = 1;

pub async fn run(config: Config, dir: &Path, request: Request, json: bool) -> Result<()> {
    let evaluation = evaluate(&config, dir, &request, &nip05::HttpNip05Verifier::new()).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&evaluation.report())?);
    } else {
        evaluation.print();
    }
    let blockers = evaluation.blockers();
    if blockers.is_empty() {
        return Ok(());
    }
    let reasons: Vec<&str> = blockers.iter().map(|b| b.message.as_str()).collect();
    anyhow::bail!("the publish would stop: {}", reasons.join("; "))
}

enum Nip05State {
    Off,
    Unknown(String),
    Checked(super::Nip05Outcome),
}

struct ClockState {
    mfs_newest: Result<Option<u64>, String>,
    dated: Result<u64, ClockError>,
    relays_answered: usize,
}

struct Evaluation<'a> {
    request: &'a Request,
    title: Option<&'a str>,
    modes: Modes,
    pubkey: Result<PublicKey, String>,
    nip05: Nip05State,
    local: LocalChecks,
    files: usize,
    bytes: u64,
    clock: ClockState,
    previous: Result<Option<SiteEvent>, String>,
    previous_files: PreviousFiles,
    new_files: Vec<String>,
    cid: Result<String, String>,
    unchanged: UnchangedOutcome,
}

const KEY_UNKNOWN: &str = "your public key is unknown";

const MISSING_KEY: &str = "missing Nostr key: set SWING_NOSTR_SECRET_KEY or [nostr].secret_key, or pair a signer app on the dashboard's setup page or with `swing signer pair`";

fn load_signer(config: &Config) -> Result<Signer, String> {
    match Signer::load(config) {
        Ok(Some(signer)) => Ok(signer),
        Ok(None) => Err(MISSING_KEY.to_string()),
        Err(e) => Err(format!("{e:#}")),
    }
}

async fn fetch_relay_state(config: &Config, signer: Result<Signer, String>, d: &str) -> RelayState {
    let relays = &config.nostr.relays;
    let Ok(signer) = signer else {
        return RelayState::clocks_only(relays, KEY_UNKNOWN.to_string()).await;
    };
    match RelayClient::connect(signer, relays).await {
        Ok(relay) => {
            let state = RelayState::fetch(&relay, config.nostr.site_event_kind, d).await;
            relay.shutdown().await;
            state
        }
        Err(e) => RelayState::clocks_only(relays, format!("{e:#}")).await,
    }
}

async fn compute_cid(
    ipfs: &Result<IpfsClient, String>,
    site: crate::ipfs::SiteListing,
) -> Result<String, String> {
    let ipfs = ipfs.as_ref().map_err(String::clone)?;
    let hashed = ipfs.hash_site(site).await.map_err(|e| format!("{e:#}"))?;
    nostr::canonical_cid(&hashed).map_err(|e| format!("Kubo returned an invalid CID: {e:#}"))
}

fn decide_unchanged(
    mode: CheckMode,
    previous: &Result<Option<SiteEvent>, String>,
    cid: &Result<String, String>,
) -> UnchangedOutcome {
    let judged = if mode == CheckMode::Off {
        CheckMode::Warn
    } else {
        mode
    };
    let mut outcome = match cid {
        Ok(cid) => {
            UnchangedOutcome::decide(judged, previous.clone().map_err(anyhow::Error::msg), cid)
        }
        Err(e) => {
            let known = previous.as_ref().ok().and_then(Option::as_ref);
            UnchangedOutcome {
                mode: judged,
                status: UnchangedStatus::Unknown,
                previous_cid: known.map(|p| p.cid.clone()),
                previous_created_at: known.map(|p| p.created_at),
                detail: Some(format!("could not compute the CID: {e}")),
            }
        }
    };
    outcome.mode = mode;
    outcome
}

async fn evaluate<'a, V: Nip05Verify>(
    config: &Config,
    dir: &Path,
    request: &'a Request,
    verifier: &V,
) -> Result<Evaluation<'a>> {
    let Prepared { title, modes } = prepare(config, dir, request)?;
    let d = request.d.as_str();
    let url = request.url.as_deref();

    let signer = load_signer(config);
    let pubkey = signer
        .as_ref()
        .map(Signer::public_key)
        .map_err(String::clone);

    let nip05 = match (&pubkey, modes.nip05) {
        (_, CheckMode::Off) => Nip05State::Off,
        (Err(_), _) => Nip05State::Unknown(KEY_UNKNOWN.to_string()),
        (Ok(pk), mode) => Nip05State::Checked(check_nip05(verifier, mode, d, &pk.to_hex()).await),
    };

    let (site, entries, local) =
        read_and_check(dir, &modes, url, &config.publish.dotfiles_allow).await?;
    let files = entries.iter().filter(|e| e.size.is_some()).count();
    let bytes = total_size(&entries);

    let ipfs = config.ipfs_client().await.map_err(|e| format!("{e:#}"));
    let relays = fetch_relay_state(config, signer, d).await;

    let layout = MfsLayout::new(config.ipfs.mfs_root.clone());
    let site_path = pubkey
        .as_ref()
        .ok()
        .map(|pk| layout.publish_site(&pk.to_hex(), d));
    let mfs_newest = match (&ipfs, &site_path) {
        (_, None) => Err(KEY_UNKNOWN.to_string()),
        (Err(e), _) => Err(e.clone()),
        (Ok(ipfs), Some(path)) => list_newest_version(ipfs, path)
            .await
            .map_err(|e| format!("{e:#}")),
    };
    let dated = date_version(
        site_path.as_deref().unwrap_or_default(),
        mfs_newest.as_ref().ok().copied().flatten(),
        &relays,
    );
    let clock = ClockState {
        mfs_newest,
        dated,
        relays_answered: relays.relays_answered(),
    };

    let previous_files =
        PreviousFiles::load_with(ipfs.as_ref().map_err(String::as_str), &relays.previous).await;
    let new_files = previous_files.new_files(&entries);

    let cid = compute_cid(&ipfs, site).await;
    let unchanged = decide_unchanged(modes.check_unchanged, &relays.previous, &cid);

    Ok(Evaluation {
        request,
        title,
        modes,
        pubkey,
        nip05,
        local,
        files,
        bytes,
        clock,
        previous: relays.previous,
        previous_files,
        new_files,
        cid,
        unchanged,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Blocker {
    pub check: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Publish,
    Unchanged,
    Blocked,
}

impl Evaluation<'_> {
    fn blockers(&self) -> Vec<Blocker> {
        let mut blockers = Vec::new();
        if let Nip05State::Checked(outcome) = &self.nip05
            && let Some(message) = &outcome.abort
        {
            blockers.push(Blocker {
                check: "nip05",
                message: message.clone(),
            });
        }
        for (check, message) in self.local.abort_reasons() {
            blockers.push(Blocker { check, message });
        }
        if let Err(e) = &self.clock.dated {
            blockers.push(Blocker {
                check: "clock",
                message: e.to_string(),
            });
        }
        blockers
    }

    fn outcome(&self) -> Outcome {
        if !self.blockers().is_empty() {
            Outcome::Blocked
        } else if self.unchanged.stops_publish() {
            Outcome::Unchanged
        } else {
            Outcome::Publish
        }
    }

    fn print(&self) {
        let request = self.request;
        print_header(
            &request.d,
            request.url.as_deref(),
            self.title,
            request.message.as_deref(),
        );
        if let Err(e) = &self.pubkey {
            println!("Key: unknown ({})", Sanitized(e));
        }
        match &self.nip05 {
            Nip05State::Off => {}
            Nip05State::Unknown(e) => print_section("NIP-05", [format!("! could not check: {e}")]),
            Nip05State::Checked(outcome) => print_section("NIP-05", [&outcome.line]),
        }
        print_local_checks(&self.local);
        print_section("Clock", self.clock_lines());
        print_section("New files", self.previous_files.lines(&self.new_files));
        print_section(
            "IPFS (CID only; nothing was added)",
            [match &self.cid {
                Ok(cid) => format!("CID: {cid}"),
                Err(e) => format!("! could not compute the CID: {e}"),
            }],
        );
        if self.modes.check_unchanged != CheckMode::Off {
            print_section("Previous version", [self.unchanged.line()]);
        }
        println!();
        println!(
            "{}",
            match self.outcome() {
                Outcome::Publish => "Would be published.",
                Outcome::Unchanged => "Unchanged; would not be published.",
                Outcome::Blocked => "Would stop before publishing; see the error below.",
            }
        );
        println!("Dry run; nothing was published.");
    }

    fn clock_lines(&self) -> Vec<String> {
        let mut lines = vec![match &self.clock.dated {
            Ok(at) => format!("\u{2713} the new version would be dated {at}"),
            Err(e) => format!("! {e}"),
        }];
        if let Err(e) = &self.clock.mfs_newest {
            lines.push(format!("! could not check the versions in MFS: {e}"));
        }
        if self.clock.relays_answered == 0 {
            lines.push("- no relay answered the clock check".to_string());
        }
        lines
    }

    fn report(&self) -> Report<'_> {
        let request = self.request;
        Report {
            format: FORMAT,
            dry_run: true,
            site: &request.d,
            url: request.url.as_deref(),
            title: self.title,
            message: request.message.as_deref(),
            note: request.note,
            pubkey: match &self.pubkey {
                Ok(pk) => Known::Known {
                    hex: pk.to_hex(),
                    npub: pk.to_bech32().unwrap_or_default(),
                },
                Err(e) => Known::Unknown { detail: e.clone() },
            },
            nip05: self.nip05_report(),
            checks: ChecksReport {
                dotfiles: self.dotfiles_report(),
                size: SizeReport {
                    mode: self.modes.check_size.name(),
                    status: match self.local.bytes {
                        None => "off",
                        Some(_) if self.local.size_over() => "over",
                        Some(_) => "ok",
                    },
                    files: self.files,
                    bytes: self.bytes,
                    guideline: super::SIZE_GUIDELINE,
                    stops: self.stops("size"),
                },
                links: self.links_report(),
            },
            clock: ClockReport {
                status: if self.clock.dated.is_ok() {
                    "ok"
                } else {
                    "stops"
                },
                created_at: self.clock.dated.as_ref().ok().copied(),
                detail: self.clock.dated.as_ref().err().map(ToString::to_string),
                mfs_versions: match &self.clock.mfs_newest {
                    Ok(newest) => MfsVersions::Listed { newest: *newest },
                    Err(e) => MfsVersions::Unknown { detail: e.clone() },
                },
                relays_answered: self.clock.relays_answered,
            },
            ipfs: match &self.cid {
                Ok(cid) => Computed::Computed { cid: cid.clone() },
                Err(e) => Computed::Unknown { detail: e.clone() },
            },
            previous: match &self.previous {
                Ok(Some(p)) => Previous::Found {
                    cid: p.cid.clone(),
                    created_at: p.created_at,
                },
                Ok(None) => Previous::None,
                Err(e) => Previous::Unknown { detail: e.clone() },
            },
            new_files: self.new_files_report(),
            unchanged: UnchangedReport {
                mode: self.modes.check_unchanged.name(),
                status: self.unchanged.status.as_str(),
                previous_cid: self.unchanged.previous_cid.clone(),
                detail: self.unchanged.detail.clone(),
                skips_publish: self.unchanged.stops_publish(),
            },
            outcome: self.outcome(),
            blockers: self.blockers(),
            guide_url: SITE_GUIDE_URL,
        }
    }

    fn stops(&self, check: &str) -> bool {
        self.blockers().iter().any(|b| b.check == check)
    }

    fn nip05_report(&self) -> Nip05Report {
        let mode = self.modes.nip05.name();
        let (status, detail) = match &self.nip05 {
            Nip05State::Off => ("off", None),
            Nip05State::Unknown(e) => ("unknown", Some(e.clone())),
            Nip05State::Checked(outcome) => {
                (outcome.result.as_state_str(), outcome.result.detail())
            }
        };
        Nip05Report {
            mode,
            status,
            detail,
            stops: self.stops("nip05"),
        }
    }

    fn dotfiles_report(&self) -> DotfilesReport {
        let paths = self.local.dotfiles.clone().unwrap_or_default();
        DotfilesReport {
            mode: self.modes.check_dotfiles.name(),
            status: match &self.local.dotfiles {
                None => "off",
                Some(found) if found.is_empty() => "ok",
                Some(_) => "found",
            },
            count: paths.len(),
            paths,
            stops: self.stops("dotfiles"),
        }
    }

    fn links_report(&self) -> LinksReport {
        let mode = self.modes.check_links.name();
        let Some(report) = &self.local.links else {
            return LinksReport {
                mode,
                status: "off",
                count: 0,
                breaking: 0,
                redirects: false,
                skipped: Vec::new(),
                findings: Vec::new(),
                stops: false,
            };
        };
        LinksReport {
            mode,
            status: if report.total() == 0 { "ok" } else { "found" },
            count: report.total(),
            breaking: report.blocking(),
            redirects: report.redirects,
            skipped: report.skipped.clone(),
            findings: findings(report),
            stops: self.stops("links"),
        }
    }

    fn new_files_report(&self) -> NewFilesReport {
        let (status, compared_with, detail) = match &self.previous_files {
            PreviousFiles::Listed { cid, .. } => ("listed", Some(cid.clone()), None),
            PreviousFiles::NoPrevious => ("no_previous", None, None),
            PreviousFiles::Unknown(e) => ("unknown", None, Some(e.clone())),
        };
        NewFilesReport {
            status,
            compared_with,
            detail,
            count: self.new_files.len(),
            paths: self.new_files.clone(),
        }
    }
}

fn findings(report: &LinkReport) -> Vec<Finding> {
    report
        .kinds()
        .flat_map(|(kind, found)| {
            found.iter().map(move |f| Finding {
                kind: kind.name(),
                breaks: report.blocks(kind),
                file: f.file.clone(),
                reference: f.reference.clone(),
                files: f.files.max(1),
            })
        })
        .collect()
}

#[derive(Serialize)]
struct Report<'a> {
    format: u32,
    dry_run: bool,
    site: &'a str,
    url: Option<&'a str>,
    title: Option<&'a str>,
    message: Option<&'a str>,
    note: bool,
    pubkey: Known,
    nip05: Nip05Report,
    checks: ChecksReport,
    clock: ClockReport,
    ipfs: Computed,
    previous: Previous,
    new_files: NewFilesReport,
    unchanged: UnchangedReport,
    outcome: Outcome,
    blockers: Vec<Blocker>,
    guide_url: &'static str,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Known {
    Known { hex: String, npub: String },
    Unknown { detail: String },
}

#[derive(Serialize)]
struct Nip05Report {
    mode: &'static str,
    status: &'static str,
    detail: Option<String>,
    stops: bool,
}

#[derive(Serialize)]
struct ChecksReport {
    dotfiles: DotfilesReport,
    size: SizeReport,
    links: LinksReport,
}

#[derive(Serialize)]
struct DotfilesReport {
    mode: &'static str,
    status: &'static str,
    count: usize,
    paths: Vec<String>,
    stops: bool,
}

#[derive(Serialize)]
struct SizeReport {
    mode: &'static str,
    status: &'static str,
    files: usize,
    bytes: u64,
    guideline: u64,
    stops: bool,
}

#[derive(Serialize)]
struct LinksReport {
    mode: &'static str,
    status: &'static str,
    count: usize,
    breaking: usize,
    redirects: bool,
    skipped: Vec<String>,
    findings: Vec<Finding>,
    stops: bool,
}

#[derive(Serialize)]
struct Finding {
    kind: &'static str,
    breaks: bool,
    file: String,
    reference: String,
    files: usize,
}

#[derive(Serialize)]
struct ClockReport {
    status: &'static str,
    created_at: Option<u64>,
    detail: Option<String>,
    mfs_versions: MfsVersions,
    relays_answered: usize,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum MfsVersions {
    Listed { newest: Option<u64> },
    Unknown { detail: String },
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Computed {
    Computed { cid: String },
    Unknown { detail: String },
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Previous {
    Found { cid: String, created_at: u64 },
    None,
    Unknown { detail: String },
}

#[derive(Serialize)]
struct NewFilesReport {
    status: &'static str,
    compared_with: Option<String>,
    detail: Option<String>,
    count: usize,
    paths: Vec<String>,
}

#[derive(Serialize)]
struct UnchangedReport {
    mode: &'static str,
    status: &'static str,
    previous_cid: Option<String>,
    detail: Option<String>,
    skips_publish: bool,
}

#[cfg(test)]
mod tests;
