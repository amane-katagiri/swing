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
                "! size: {} is over the {} guideline; each mirror decides by its own limits (max_update_size, default {})",
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

    pub fn abort_message(&self) -> Option<String> {
        let mut reasons = Vec::new();
        if self.dotfiles_mode == CheckMode::Require && self.dotfiles_found() {
            reasons.push(
                "dotfiles found: remove them, add their names to [publish].dotfiles_allow, or set --check-dotfiles / [publish].check_dotfiles to warn or off"
                    .to_string(),
            );
        }
        if self.size_mode == CheckMode::Require && self.size_over() {
            reasons.push(format!(
                "the site is larger than {}: make it smaller, or set --check-size / [publish].check_size to warn or off",
                format_bytes(SIZE_GUIDELINE)
            ));
        }
        if self.links_mode == CheckMode::Require
            && let Some(reason) = self.links.as_ref().and_then(LinkReport::abort_reason)
        {
            reasons.push(reason);
        }
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
mod tests {
    use super::*;
    use nostr_sdk::prelude::Keys;

    fn allow() -> Vec<String> {
        crate::config::DEFAULT_DOTFILES_ALLOW
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn entries(dir: &Path) -> Vec<SiteEntry> {
        crate::ipfs::SiteListing::read(dir).unwrap().entries()
    }

    #[test]
    fn protected_paths_inside_the_site_are_refused() {
        let site = tempfile::tempdir().unwrap();
        let config = site.path().join("swing.toml");
        std::fs::write(&config, b"").unwrap();
        let state = site.path().join("data");
        std::fs::create_dir(&state).unwrap();

        let err = refuse_paths_inside(site.path(), &[("the config file", &config)]).unwrap_err();
        assert!(err.to_string().contains("the config file"), "{err}");
        let err = refuse_paths_inside(&state, &[("[agent].state_dir", &state)]).unwrap_err();
        assert!(err.to_string().contains("[agent].state_dir"), "{err}");
        let err = refuse_paths_inside(site.path(), &[("[agent].state_dir", &state.join("."))])
            .unwrap_err();
        assert!(err.to_string().contains("[agent].state_dir"), "{err}");
    }

    #[test]
    fn a_site_inside_the_state_dir_and_missing_paths_are_allowed() {
        let state = tempfile::tempdir().unwrap();
        let upload = state.path().join("uploads/site");
        std::fs::create_dir_all(&upload).unwrap();
        std::fs::write(upload.join("index.html"), b"hi").unwrap();
        let config = state.path().join("swing.toml");
        std::fs::write(&config, b"").unwrap();
        refuse_paths_inside(
            &upload,
            &[
                ("the config file", &config),
                ("[agent].state_dir", state.path()),
                ("[kubo].repo", &upload.join("kubo")),
            ],
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_protected_path_reached_through_a_symlink_is_refused() {
        use std::os::unix::fs::symlink;

        let site = tempfile::tempdir().unwrap();
        std::fs::create_dir(site.path().join("data")).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let link = elsewhere.path().join("data");
        symlink(site.path().join("data"), &link).unwrap();
        let err = refuse_paths_inside(site.path(), &[("[agent].state_dir", &link)]).unwrap_err();
        assert!(err.to_string().contains("[agent].state_dir"), "{err}");
    }

    #[test]
    fn find_dotfiles_skips_allowed_names_but_not_dotfiles_beneath_them() {
        let paths = [
            "index.html",
            ".well-known",
            ".well-known/nostr.json",
            ".well-known/.secret",
            ".well-known/.git/HEAD",
            ".nojekyll",
            "blog/.gitkeep",
            ".env",
        ];
        assert_eq!(
            find_dotfiles(paths, &allow()),
            vec![".well-known/.secret", ".well-known/.git", ".env"]
        );
    }

    #[test]
    fn a_site_inside_the_kubo_repo_or_the_state_dir_is_refused() {
        let state = tempfile::tempdir().unwrap();
        let repo = state.path().join("kubo");
        std::fs::create_dir_all(repo.join("blocks")).unwrap();
        let upload = state.path().join(DASHBOARD_UPLOAD_DIR);
        std::fs::create_dir_all(upload.join("abc/sub")).unwrap();
        std::fs::create_dir_all(state.path().join("site")).unwrap();
        let containers = [
            ("[kubo].repo", repo.as_path(), None),
            ("[agent].state_dir", state.path(), Some(upload.as_path())),
        ];

        for site in [repo.join("blocks"), repo.clone()] {
            let err = refuse_site_inside(&site, &containers).unwrap_err();
            assert!(err.to_string().contains("[kubo].repo"), "{err}");
        }
        for site in [
            state.path().join("site"),
            state.path().to_path_buf(),
            upload.clone(),
        ] {
            let err = refuse_site_inside(&site, &containers).unwrap_err();
            assert!(err.to_string().contains("[agent].state_dir"), "{err}");
        }
        refuse_site_inside(&upload.join("abc"), &containers).unwrap();
        refuse_site_inside(&upload.join("abc/sub"), &containers).unwrap();

        let elsewhere = tempfile::tempdir().unwrap();
        refuse_site_inside(elsewhere.path(), &containers).unwrap();
        refuse_site_inside(
            elsewhere.path(),
            &[("[kubo].repo", &elsewhere.path().join("missing"), None)],
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_site_reached_through_a_symlink_into_the_repo_is_refused() {
        use std::os::unix::fs::symlink;

        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join("keystore")).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let link = elsewhere.path().join("site");
        symlink(repo.path().join("keystore"), &link).unwrap();
        let err = refuse_site_inside(&link, &[("[kubo].repo", repo.path(), None)]).unwrap_err();
        assert!(err.to_string().contains("[kubo].repo"), "{err}");
    }

    #[test]
    fn find_dotfiles_reports_a_hit_directory_once_and_not_its_children() {
        let paths = [
            ".git",
            ".git/HEAD",
            ".git/objects",
            ".git/objects/ab",
            "assets",
            "assets/.DS_Store",
            "assets/img/.cache/x.png",
        ];
        assert_eq!(
            find_dotfiles(paths, &allow()),
            vec![".git", "assets/.DS_Store", "assets/img/.cache"]
        );
    }

    #[test]
    fn find_dotfiles_with_an_empty_allow_list_flags_well_known() {
        assert_eq!(
            find_dotfiles([".well-known", ".well-known/nostr.json"], &[]),
            vec![".well-known"]
        );
    }

    #[test]
    fn local_checks_scan_nested_dirs_and_do_not_descend_into_hits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join(".git/objects/aa"), b"x").unwrap();
        std::fs::create_dir_all(dir.path().join("docs/.well-known")).unwrap();
        std::fs::write(dir.path().join("docs/.well-known/.hidden"), b"x").unwrap();
        std::fs::write(dir.path().join("docs/.well-known/ok.json"), b"x").unwrap();
        std::fs::write(dir.path().join("docs/.env"), b"SECRET=1").unwrap();
        let checks = LocalChecks::evaluate(
            &entries(dir.path()),
            CheckMode::Require,
            CheckMode::Warn,
            CheckMode::Off,
            None,
            &allow(),
        );
        assert_eq!(
            checks.dotfiles.as_deref().unwrap(),
            [".git", "docs/.env", "docs/.well-known/.hidden"]
        );
        assert_eq!(checks.bytes, Some(2 + 1 + 1 + 1 + 8));
        assert!(checks.abort_message().unwrap().contains("dotfiles_allow"));
    }

    #[cfg(unix)]
    #[test]
    fn local_checks_follow_symlinked_dirs_like_add_does() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join(".env"), b"SECRET=1").unwrap();
        std::fs::write(target.join("page.html"), b"hello").unwrap();
        symlink("target", dir.path().join("linked")).unwrap();
        let checks = LocalChecks::evaluate(
            &entries(dir.path()),
            CheckMode::Warn,
            CheckMode::Warn,
            CheckMode::Off,
            None,
            &allow(),
        );
        assert_eq!(
            checks.dotfiles.as_deref().unwrap(),
            ["linked/.env", "target/.env"]
        );
        assert_eq!(checks.bytes, Some(26));
        assert!(checks.abort_message().is_none());
    }

    #[test]
    fn local_checks_listing_caps_the_paths_shown() {
        let paths: Vec<String> = (0..12).map(|i| format!(".f{i:02}")).collect();
        let entries: Vec<SiteEntry> = paths
            .iter()
            .map(|p| SiteEntry {
                path: p.clone(),
                size: Some(1),
            })
            .collect();
        let checks = LocalChecks::evaluate(
            &entries,
            CheckMode::Warn,
            CheckMode::Off,
            CheckMode::Off,
            None,
            &[],
        );
        let lines = checks.lines();
        assert_eq!(
            lines[0],
            "! dotfiles: 12 found (not in [publish].dotfiles_allow)"
        );
        assert_eq!(lines.len(), 1 + LISTED_DOTFILES + 1 + 1 + 1);
        assert_eq!(lines[LISTED_DOTFILES + 1], "    \u{2026} and 2 more");
        assert_eq!(lines[LISTED_DOTFILES + 2], "- size: off");
        assert_eq!(lines.last().unwrap(), "- links: off");
    }

    #[test]
    fn size_check_flags_only_strictly_over_the_guideline() {
        let at = [SiteEntry {
            path: "a".into(),
            size: Some(SIZE_GUIDELINE),
        }];
        let over = [
            SiteEntry {
                path: "a".into(),
                size: Some(SIZE_GUIDELINE),
            },
            SiteEntry {
                path: "b".into(),
                size: Some(1),
            },
            SiteEntry {
                path: "d".into(),
                size: None,
            },
        ];
        let checks = LocalChecks::evaluate(
            &at,
            CheckMode::Off,
            CheckMode::Require,
            CheckMode::Off,
            None,
            &[],
        );
        assert!(!checks.size_over());
        assert!(checks.abort_message().is_none());
        let checks = LocalChecks::evaluate(
            &over,
            CheckMode::Off,
            CheckMode::Require,
            CheckMode::Off,
            None,
            &[],
        );
        assert_eq!(checks.bytes, Some(SIZE_GUIDELINE + 1));
        assert!(checks.size_over());
        assert!(checks.abort_message().unwrap().contains("--check-size"));
        let checks = LocalChecks::evaluate(
            &over,
            CheckMode::Off,
            CheckMode::Warn,
            CheckMode::Off,
            None,
            &[],
        );
        assert!(checks.abort_message().is_none());
        assert!(checks.lines()[1].contains("guideline"));
    }

    #[test]
    fn link_check_stops_only_under_require_with_breaking_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), br#"<a href="/about/">a</a>"#).unwrap();
        let site = crate::ipfs::SiteListing::read(dir.path()).unwrap();
        let report = super::super::links::scan(&site, None).unwrap();
        let evaluate = |mode| {
            LocalChecks::evaluate(
                &site.entries(),
                CheckMode::Off,
                CheckMode::Off,
                mode,
                Some(report.clone()),
                &[],
            )
        };
        let require = evaluate(CheckMode::Require);
        assert!(!require.all_off());
        assert!(require.abort_message().unwrap().contains("--check-links"));
        assert!(require.lines()[2].starts_with("! links: 2 found"));
        assert!(evaluate(CheckMode::Warn).abort_message().is_none());
        let off = evaluate(CheckMode::Off);
        assert!(off.all_off());
        assert_eq!(off.links, None);
        assert_eq!(off.lines()[2], "- links: off");
    }

    #[test]
    fn off_modes_skip_the_checks_entirely() {
        let entries = [SiteEntry {
            path: ".env".into(),
            size: Some(SIZE_GUIDELINE * 2),
        }];
        let checks = LocalChecks::evaluate(
            &entries,
            CheckMode::Off,
            CheckMode::Off,
            CheckMode::Off,
            None,
            &[],
        );
        assert!(checks.all_off());
        assert_eq!(checks.dotfiles, None);
        assert_eq!(checks.bytes, None);
        assert!(checks.abort_message().is_none());
    }

    fn site(cid: &str) -> SiteEvent {
        SiteEvent {
            pubkey: Keys::generate().public_key(),
            d: "example.com".into(),
            cid: cid.into(),
            url: None,
            size: None,
            title: None,
            message: None,
            created_at: 100,
            id: nostr_sdk::prelude::EventId::from_byte_array([0; 32]),
        }
    }

    #[test]
    fn unchanged_stops_only_under_require_with_an_equal_cid() {
        let same = UnchangedOutcome::decide(CheckMode::Require, Ok(Some(site("bafy1"))), "bafy1");
        assert_eq!(same.status, UnchangedStatus::Unchanged);
        assert!(same.stops_publish());
        assert_eq!(same.previous_created_at, Some(100));

        let warn = UnchangedOutcome::decide(CheckMode::Warn, Ok(Some(site("bafy1"))), "bafy1");
        assert_eq!(warn.status, UnchangedStatus::Unchanged);
        assert!(!warn.stops_publish());

        let changed =
            UnchangedOutcome::decide(CheckMode::Require, Ok(Some(site("bafy0"))), "bafy1");
        assert_eq!(changed.status, UnchangedStatus::Changed);
        assert_eq!(changed.previous_cid.as_deref(), Some("bafy0"));
        assert!(!changed.stops_publish());
    }

    #[test]
    fn unchanged_never_stops_when_the_relays_cannot_tell() {
        let none = UnchangedOutcome::decide(CheckMode::Require, Ok(None), "bafy1");
        assert_eq!(none.status, UnchangedStatus::NoPrevious);
        assert!(!none.stops_publish());

        let failed =
            UnchangedOutcome::decide(CheckMode::Require, Err(anyhow::anyhow!("timeout")), "bafy1");
        assert_eq!(failed.status, UnchangedStatus::Unknown);
        assert_eq!(failed.detail.as_deref(), Some("timeout"));
        assert!(!failed.stops_publish());
        assert!(failed.line().contains("could not check"));
    }

    #[test]
    fn unchanged_off_ignores_the_lookup() {
        let off = UnchangedOutcome::decide(CheckMode::Off, Ok(Some(site("bafy1"))), "bafy1");
        assert_eq!(off, UnchangedOutcome::off());
        assert!(!off.stops_publish());
    }
}
