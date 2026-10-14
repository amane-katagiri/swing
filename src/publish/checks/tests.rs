use super::*;
use crate::test_support::{random_pubkey, site_event_fixture};

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
    let err =
        refuse_paths_inside(site.path(), &[("[agent].state_dir", &state.join("."))]).unwrap_err();
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
fn find_dotfiles_reports_the_topmost_unallowed_dotfile() {
    let cases = [
        (
            "allowed names but not dotfiles beneath them",
            vec![
                "index.html",
                ".well-known",
                ".well-known/nostr.json",
                ".well-known/.secret",
                ".well-known/.git/HEAD",
                ".nojekyll",
                "blog/.gitkeep",
                ".env",
            ],
            allow(),
            vec![".well-known/.secret", ".well-known/.git", ".env"],
        ),
        (
            "a hit directory once and not its children",
            vec![
                ".git",
                ".git/HEAD",
                ".git/objects",
                ".git/objects/ab",
                "assets",
                "assets/.DS_Store",
                "assets/img/.cache/x.png",
            ],
            allow(),
            vec![".git", "assets/.DS_Store", "assets/img/.cache"],
        ),
        (
            "an empty allow list flags .well-known",
            vec![".well-known", ".well-known/nostr.json"],
            Vec::new(),
            vec![".well-known"],
        ),
    ];
    for (case, paths, allow, want) in cases {
        assert_eq!(find_dotfiles(paths, &allow), want, "{case}");
    }
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
    site_event_fixture(random_pubkey(), "example.com", cid, 100)
}

#[test]
fn unchanged_stops_only_under_require_with_an_equal_cid() {
    use UnchangedStatus::*;
    let cases = [
        (
            "require, same cid",
            CheckMode::Require,
            Ok(Some(site("bafy1"))),
            Unchanged,
            true,
        ),
        (
            "warn, same cid",
            CheckMode::Warn,
            Ok(Some(site("bafy1"))),
            Unchanged,
            false,
        ),
        (
            "require, changed",
            CheckMode::Require,
            Ok(Some(site("bafy0"))),
            Changed,
            false,
        ),
        (
            "require, no previous",
            CheckMode::Require,
            Ok(None),
            NoPrevious,
            false,
        ),
        (
            "require, lookup failed",
            CheckMode::Require,
            Err(anyhow::anyhow!("timeout")),
            Unknown,
            false,
        ),
        ("off", CheckMode::Off, Ok(Some(site("bafy1"))), Off, false),
    ];
    for (case, mode, previous, status, stops) in cases {
        let outcome = UnchangedOutcome::decide(mode, previous, "bafy1");
        assert_eq!(outcome.status, status, "{case}");
        assert_eq!(outcome.stops_publish(), stops, "{case}");
        match case {
            "require, same cid" => assert_eq!(outcome.previous_created_at, Some(100)),
            "require, changed" => assert_eq!(outcome.previous_cid.as_deref(), Some("bafy0")),
            "require, lookup failed" => {
                assert_eq!(outcome.detail.as_deref(), Some("timeout"));
                assert!(outcome.line().contains("could not check"));
            }
            "off" => assert_eq!(outcome, UnchangedOutcome::off()),
            _ => {}
        }
    }
}
