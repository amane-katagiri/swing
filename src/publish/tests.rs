mod run;

use super::*;
use crate::test_support::{CID_A, KuboApi, site_event_fixture};

#[test]
fn only_canonical_decimal_names_are_versions() {
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(newest_version(&names(&[])), None);
    assert_eq!(newest_version(&names(&["100", "junk"])), Some(100));
    assert_eq!(newest_version(&names(&["100", "0500", "+600"])), Some(100));
    assert_eq!(newest_version(&names(&["0"])), Some(0));
    assert_eq!(newest_version(&names(&["junk", "00"])), None);
}

async fn kubo_with_versions(site: &str, versions: &[u64]) -> KuboApi {
    let kubo = KuboApi::serve(CID_A).await;
    kubo.with(|s| {
        for v in versions {
            s.mfs.insert(format!("{site}/{v}"), "bafy".to_string());
        }
    });
    kubo
}

#[tokio::test]
async fn list_newest_version_reads_the_site_dir() {
    let kubo = kubo_with_versions("/swing/publish/k/s", &[499, 500]).await;
    assert_eq!(
        list_newest_version(&kubo.client(), "/swing/publish/k/s")
            .await
            .unwrap(),
        Some(500)
    );
    let kubo = kubo_with_versions("/swing/publish/k/s", &[]).await;
    assert_eq!(
        list_newest_version(&kubo.client(), "/swing/publish/k/s")
            .await
            .unwrap(),
        None
    );
}

fn relay_state(previous: Option<u64>, relay_offsets: Vec<(String, i64)>) -> RelayState {
    let previous = previous.map(|created_at| {
        site_event_fixture(
            Keys::generate().public_key(),
            "example.com",
            CID_A,
            created_at,
        )
    });
    RelayState {
        previous: Ok(previous),
        relay_offsets,
    }
}

async fn refused_stage(newest: u64, relays: RelayState) -> (anyhow::Error, bool) {
    let kubo = kubo_with_versions("/swing/publish/k/example.com", &[100, newest]).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
    let site = SiteListing::read_async(dir.path()).await.unwrap();
    let layout = MfsLayout::new("/swing".to_string());
    let Err(err) =
        add_and_measure(&kubo.client(), &layout, "k", "example.com", &relays, site).await
    else {
        panic!("add_and_measure should refuse");
    };
    (err, kubo.calls().iter().any(|c| c != "files/ls"))
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
async fn add_and_measure_moves_past_a_version_path_another_publish_took() {
    let newest = Timestamp::now().as_secs() + 5;
    let taken = newest + 1;
    let site = "/swing/publish/k/example.com";
    let kubo = kubo_with_versions(site, &[newest]).await;
    kubo.with(|s| {
        s.taken.insert(format!("{site}/{taken}"));
    });
    let ipfs = kubo.client();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
    let site = SiteListing::read_async(dir.path()).await.unwrap();
    let layout = MfsLayout::new("/swing".to_string());

    let stage = add_and_measure(
        &ipfs,
        &layout,
        "k",
        "example.com",
        &relay_state(None, Vec::new()),
        site,
    )
    .await
    .unwrap();

    assert_eq!(stage.created_at.as_secs(), taken + 1);
    assert_eq!(stage.size, 42);
    assert_eq!(
        stage.version.keep(),
        format!("/swing/publish/k/example.com/{}", taken + 1)
    );
    let calls = kubo.calls();
    assert_eq!(calls.iter().filter(|c| *c == "files/cp").count(), 2);
    assert!(!calls.contains(&"files/rm".to_string()), "{calls:?}");
}

#[tokio::test]
async fn plan_version_ignores_one_lagging_relay_and_dates_after_the_previous() {
    let kubo = kubo_with_versions("/swing/publish/k/s", &[100]).await;
    let now = Timestamp::now().as_secs();
    let relays = RelayState {
        previous: Err("relay down".to_string()),
        relay_offsets: vec![
            ("wss://a".to_string(), 1_000_000_000),
            ("wss://b".to_string(), 0),
        ],
    };
    let created_at = plan_version(&kubo.client(), "/swing/publish/k/s", &relays)
        .await
        .unwrap();
    assert!(created_at >= now, "{created_at}");
    let relays = relay_state(Some(now + 600), Vec::new());
    assert_eq!(
        plan_version(&kubo.client(), "/swing/publish/k/s", &relays)
            .await
            .unwrap(),
        now + 601
    );
}

#[tokio::test]
async fn add_and_measure_asks_to_wait_for_a_previous_event_at_the_limit_before_adding() {
    let now = Timestamp::now().as_secs();
    let relays = relay_state(Some(now + nostr::MAX_FUTURE_SKEW), Vec::new());
    let (err, added) = refused_stage(100, relays).await;
    assert!(err.downcast_ref::<ClockError>().is_some(), "{err:#}");
    assert!(err.to_string().contains("and publish again"), "{err}");
    assert!(!added);
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
    let error = results[0].error.as_deref().unwrap();
    assert!(
        error.ends_with(&format!(" ({FUTURE_REJECTION_HINT})")),
        "{error}"
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
        nip05::VerificationResult::Error("boom".to_string(), nip05::ErrorCategory::InvalidResponse),
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
            check_links: CheckMode::Warn,
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
    let bad = ModeOverrides {
        check_links: Some("strict".into()),
        ..Default::default()
    };
    let (name, _) = resolve_modes(&bad, &defaults).unwrap_err();
    assert_eq!(name, "check-links");
    let overrides = ModeOverrides {
        check_links: Some("require".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_modes(&overrides, &defaults).unwrap().check_links,
        CheckMode::Require
    );
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
        note: false,
        modes: ModeOverrides {
            nip05: Some("off".into()),
            check_dotfiles: Some("off".into()),
            check_size: Some("off".into()),
            check_links: Some("off".into()),
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
