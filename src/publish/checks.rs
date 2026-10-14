use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::{CheckMode, Config, DEFAULT_MAX_UPDATE_SIZE};
use crate::format::{format_bytes, format_bytes_approx};
use crate::ipfs::SiteEntry;
use crate::nostr::SiteEvent;

use super::links::LinkReport;

pub const SIZE_GUIDELINE: u64 = 512 << 20;
pub const LISTED_DOTFILES: usize = 10;

pub const DASHBOARD_UPLOAD_DIR: &str = "upload";

pub fn find_dotfiles<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    allow: &[String],
) -> Vec<String> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let mut end = 0;
        for segment in path.split('/') {
            end += segment.len();
            if segment.starts_with('.') && !allow.iter().any(|name| name == segment) {
                let hit = &path[..end];
                if seen.insert(hit.to_string()) {
                    hits.push(hit.to_string());
                }
                break;
            }
            end += 1;
        }
    }
    hits
}

pub fn refuse_protected_paths(dir: &Path, config: &Config) -> Result<()> {
    refuse_paths_inside(
        dir,
        &[
            ("the config file", &config.config_path),
            ("[agent].state_dir", &config.agent.state_dir),
            ("[kubo].repo", &config.kubo.repo),
        ],
    )?;
    let upload = config.agent.state_dir.join(DASHBOARD_UPLOAD_DIR);
    refuse_site_inside(
        dir,
        &[
            ("[kubo].repo", &config.kubo.repo, None),
            ("[agent].state_dir", &config.agent.state_dir, Some(&upload)),
        ],
    )
}

fn canonicalize_existing(path: &Path) -> Result<Option<std::path::PathBuf>> {
    match std::fs::canonicalize(path) {
        Ok(canon) => Ok(Some(canon)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("resolving path {}", path.display())),
    }
}

fn refuse_paths_inside(dir: &Path, protected: &[(&str, &Path)]) -> Result<()> {
    let root =
        std::fs::canonicalize(dir).with_context(|| format!("resolving path {}", dir.display()))?;
    for (what, path) in protected {
        let Some(canon) = canonicalize_existing(path)? else {
            continue;
        };
        if canon.starts_with(&root) {
            bail!(
                "{} contains {what} ({}), which holds secrets; publish a directory that does not include swing's config or data",
                dir.display(),
                canon.display()
            );
        }
    }
    Ok(())
}

fn refuse_site_inside(dir: &Path, containers: &[(&str, &Path, Option<&Path>)]) -> Result<()> {
    let root =
        std::fs::canonicalize(dir).with_context(|| format!("resolving path {}", dir.display()))?;
    for (what, path, allowed) in containers {
        let Some(canon) = canonicalize_existing(path)? else {
            continue;
        };
        if !root.starts_with(&canon) {
            continue;
        }
        if let Some(allowed) = allowed
            && let Some(allowed) = canonicalize_existing(allowed)?
            && root != allowed
            && root.starts_with(&allowed)
        {
            continue;
        }
        bail!(
            "{} is inside {what} ({}), which holds swing's data; publish a directory outside swing's config and data",
            dir.display(),
            canon.display()
        );
    }
    Ok(())
}

pub fn total_size(entries: &[SiteEntry]) -> u64 {
    entries.iter().filter_map(|e| e.size).sum()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalChecks {
    pub dotfiles_mode: CheckMode,
    pub dotfiles: Option<Vec<String>>,
    pub size_mode: CheckMode,
    pub bytes: Option<u64>,
    pub links_mode: CheckMode,
    pub links: Option<LinkReport>,
}

impl LocalChecks {
    pub fn evaluate(
        entries: &[SiteEntry],
        dotfiles_mode: CheckMode,
        size_mode: CheckMode,
        links_mode: CheckMode,
        links: Option<LinkReport>,
        allow: &[String],
    ) -> Self {
        let dotfiles = (dotfiles_mode != CheckMode::Off)
            .then(|| find_dotfiles(entries.iter().map(|e| e.path.as_str()), allow));
        let bytes = (size_mode != CheckMode::Off).then(|| total_size(entries));
        Self {
            dotfiles_mode,
            dotfiles,
            size_mode,
            bytes,
            links: links.filter(|_| links_mode != CheckMode::Off),
            links_mode,
        }
    }

    pub fn all_off(&self) -> bool {
        self.dotfiles_mode == CheckMode::Off
            && self.size_mode == CheckMode::Off
            && self.links_mode == CheckMode::Off
    }

    pub fn dotfiles_found(&self) -> bool {
        self.dotfiles.as_ref().is_some_and(|d| !d.is_empty())
    }

    pub fn size_over(&self) -> bool {
        self.bytes.is_some_and(|b| b > SIZE_GUIDELINE)
    }

    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match &self.dotfiles {
            None => lines.push("- dotfiles: off".to_string()),
            Some(found) if found.is_empty() => lines.push("\u{2713} dotfiles: none".to_string()),
            Some(found) => {
                lines.push(format!(
                    "! dotfiles: {} found (not in [publish].dotfiles_allow)",
                    found.len()
                ));
                for path in found.iter().take(LISTED_DOTFILES) {
                    lines.push(format!("    {path}"));
                }
                if found.len() > LISTED_DOTFILES {
                    lines.push(format!(
                        "    \u{2026} and {} more",
                        found.len() - LISTED_DOTFILES
                    ));
                }
            }
        }
        match self.bytes {
            None => lines.push("- size: off".to_string()),
            Some(bytes) if bytes <= SIZE_GUIDELINE => lines.push(format!(
                "\u{2713} size: {} (guideline {})",
                format_bytes_approx(bytes),
                format_bytes(SIZE_GUIDELINE)
            )),
            Some(bytes) => lines.push(format!(
                "! size: {} is over the {} guideline; each mirror decides based on its own limit (max_update_size, default {})",
                format_bytes_approx(bytes),
                format_bytes(SIZE_GUIDELINE),
                format_bytes(DEFAULT_MAX_UPDATE_SIZE)
            )),
        }
        match &self.links {
            None => lines.push("- links: off".to_string()),
            Some(report) => lines.extend(report.lines()),
        }
        lines
    }

    pub fn abort_reasons(&self) -> Vec<(&'static str, String)> {
        let mut reasons = Vec::new();
        if self.dotfiles_mode == CheckMode::Require && self.dotfiles_found() {
            reasons.push((
                "dotfiles",
                "dotfiles found: remove them, add their names to [publish].dotfiles_allow, or set --check-dotfiles / [publish].check_dotfiles to warn or off"
                    .to_string(),
            ));
        }
        if self.size_mode == CheckMode::Require && self.size_over() {
            reasons.push((
                "size",
                format!(
                    "the site is larger than {}: make it smaller, or set --check-size / [publish].check_size to warn or off",
                    format_bytes(SIZE_GUIDELINE)
                ),
            ));
        }
        if self.links_mode == CheckMode::Require
            && let Some(reason) = self.links.as_ref().and_then(LinkReport::abort_reason)
        {
            reasons.push(("links", reason));
        }
        reasons
    }

    pub fn abort_message(&self) -> Option<String> {
        let reasons: Vec<String> = self.abort_reasons().into_iter().map(|(_, r)| r).collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnchangedStatus {
    Off,
    Changed,
    Unchanged,
    NoPrevious,
    Unknown,
}

impl UnchangedStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            UnchangedStatus::Off => "off",
            UnchangedStatus::Changed => "changed",
            UnchangedStatus::Unchanged => "unchanged",
            UnchangedStatus::NoPrevious => "no_previous",
            UnchangedStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnchangedOutcome {
    pub mode: CheckMode,
    pub status: UnchangedStatus,
    pub previous_cid: Option<String>,
    pub previous_created_at: Option<u64>,
    pub detail: Option<String>,
}

impl UnchangedOutcome {
    pub fn off() -> Self {
        Self {
            mode: CheckMode::Off,
            status: UnchangedStatus::Off,
            previous_cid: None,
            previous_created_at: None,
            detail: None,
        }
    }

    pub fn decide(mode: CheckMode, previous: Result<Option<SiteEvent>>, new_cid: &str) -> Self {
        if mode == CheckMode::Off {
            return Self::off();
        }
        let (status, previous_cid, previous_created_at, detail) = match previous {
            Err(e) => (UnchangedStatus::Unknown, None, None, Some(format!("{e:#}"))),
            Ok(None) => (UnchangedStatus::NoPrevious, None, None, None),
            Ok(Some(ev)) => (
                if ev.cid == new_cid {
                    UnchangedStatus::Unchanged
                } else {
                    UnchangedStatus::Changed
                },
                Some(ev.cid),
                Some(ev.created_at),
                None,
            ),
        };
        Self {
            mode,
            status,
            previous_cid,
            previous_created_at,
            detail,
        }
    }

    pub fn stops_publish(&self) -> bool {
        self.mode == CheckMode::Require && self.status == UnchangedStatus::Unchanged
    }

    pub fn line(&self) -> String {
        match self.status {
            UnchangedStatus::Off => "- unchanged: off".to_string(),
            UnchangedStatus::Changed => format!(
                "\u{2713} changed from the latest version on the relays ({})",
                self.previous_cid.as_deref().unwrap_or_default()
            ),
            UnchangedStatus::Unchanged => {
                "! unchanged: the CID equals your latest version on the relays".to_string()
            }
            UnchangedStatus::NoPrevious => "- no previous version on the relays".to_string(),
            UnchangedStatus::Unknown => format!(
                "! could not check: {}",
                self.detail.as_deref().unwrap_or_default()
            ),
        }
    }
}

#[cfg(test)]
mod tests;
