use super::*;
use serde_json::json;

fn snapshot(status: Status) -> Snapshot {
    Snapshot {
        status,
        ui: true,
        service_installed: Some(true),
    }
}

fn running(instance: &str) -> Status {
    Status::Running {
        setup: false,
        signer_failed: false,
        instance: Some(instance.into()),
    }
}

#[test]
fn overview_maps_setup_signer_failure_and_instance() {
    assert_eq!(
        Status::from_overview(&json!({"setup": true, "signer": null, "instance": "a1"})),
        Status::Running {
            setup: true,
            signer_failed: false,
            instance: Some("a1".into()),
        }
    );
    assert_eq!(
        Status::from_overview(&json!({
            "setup": false,
            "signer": {"remote": true, "relays": [], "last_failure": {"at": 1, "message": "timeout"}}
        })),
        Status::Running {
            setup: false,
            signer_failed: true,
            instance: None,
        }
    );
    assert_eq!(
        Status::from_overview(&json!({
            "setup": false,
            "signer": {"remote": true, "relays": [], "last_failure": null}
        })),
        Status::Running {
            setup: false,
            signer_failed: false,
            instance: None,
        }
    );
}

#[test]
fn running_enables_open_restart_stop_but_not_start() {
    let s = menu_state(&snapshot(running("a")), None, None, Lang::En);
    assert_eq!(
        s,
        MenuState {
            status_text: "SWING: Running".into(),
            open: true,
            restart: true,
            stop: true,
            start: false,
            quit: true,
            active_icon: true,
        }
    );
}

#[test]
fn open_is_disabled_when_the_dashboard_ui_is_off() {
    let mut snap = snapshot(running("a"));
    snap.ui = false;
    assert!(!menu_state(&snap, None, None, Lang::En).open);
}

#[test]
fn stopped_can_start_only_when_registered_as_a_service() {
    let s = menu_state(&snapshot(Status::Stopped), None, None, Lang::Ja);
    assert!(s.start && !s.open && !s.restart && !s.stop && !s.active_icon);
    assert_eq!(s.status_text, "SWING: 停止中");

    let mut snap = snapshot(Status::Stopped);
    snap.service_installed = Some(false);
    let s = menu_state(&snap, None, None, Lang::Ja);
    assert!(!s.start);
    assert_eq!(s.status_text, "SWING: 停止中（サービス未登録）");

    snap.service_installed = None;
    let s = menu_state(&snap, None, None, Lang::Ja);
    assert!(s.start);
    assert_eq!(s.status_text, "SWING: 停止中");
}

#[test]
fn error_disables_every_action_but_quit() {
    let s = menu_state(&snapshot(Status::Error("401".into())), None, None, Lang::En);
    assert!(!s.open && !s.restart && !s.stop && !s.start && !s.active_icon);
    assert!(s.quit);
    assert_eq!(s.status_text, "SWING: Error: 401");
}

#[test]
fn pending_disables_every_action_right_away() {
    for (pending, text) in [
        (Pending::Stopping, "SWING: Stopping…"),
        (Pending::restarting(None), "SWING: Restarting…"),
    ] {
        let s = menu_state(&snapshot(running("a")), Some(&pending), None, Lang::En);
        assert!(!s.open && !s.restart && !s.stop && !s.start, "{pending:?}");
        assert!(s.quit);
        assert!(s.active_icon);
        assert_eq!(s.status_text, text);
    }
    let s = menu_state(
        &snapshot(Status::Stopped),
        Some(&Pending::Starting),
        None,
        Lang::Ja,
    );
    assert!(!s.start && !s.stop);
    assert_eq!(s.status_text, "SWING: 起動しています…");
}

#[test]
fn quitting_also_disables_quit() {
    let s = menu_state(
        &snapshot(running("a")),
        Some(&Pending::Quitting),
        None,
        Lang::En,
    );
    assert!(!s.quit && !s.stop && !s.start);
    assert_eq!(s.status_text, "SWING: Stopping…");
}

#[test]
fn pending_settles_only_on_the_state_it_waits_for() {
    assert!(Pending::Starting.settled_by(&running("a")));
    assert!(!Pending::Starting.settled_by(&Status::Stopped));
    assert!(Pending::Stopping.settled_by(&Status::Stopped));
    assert!(!Pending::Stopping.settled_by(&running("a")));
    assert!(!Pending::Stopping.settled_by(&Status::Error("x".into())));
    assert!(!Pending::Quitting.settled_by(&Status::Stopped));
}

#[test]
fn restart_settles_once_a_new_instance_answers() {
    let pending = Pending::restarting(Some(&snapshot(running("a"))));
    assert_eq!(
        pending,
        Pending::Restarting {
            from: Some("a".into())
        }
    );
    assert!(!pending.settled_by(&running("a")));
    assert!(!pending.settled_by(&Status::Stopped));
    assert!(pending.settled_by(&running("b")));
}

#[test]
fn action_error_replaces_the_status_line_with_its_first_line_shortened() {
    let long = format!("{}\nsecond line", "x".repeat(200));
    let s = menu_state(&snapshot(Status::Stopped), None, Some(&long), Lang::En);
    assert!(s.status_text.starts_with("Action failed: xxx"));
    assert!(s.status_text.ends_with('…'));
    assert!(!s.status_text.contains("second line"));
    assert_eq!(
        s.status_text.chars().count(),
        "Action failed: ".len() + MAX_MESSAGE_CHARS
    );
}

#[test]
fn registration_counts_as_removed_only_after_it_was_seen() {
    let mut watch = RegistrationWatch::default();
    assert!(!watch.removed(Some(false)));
    assert!(!watch.removed(None));
    assert!(!watch.removed(Some(false)));
    assert!(!watch.removed(Some(true)));
    assert!(!watch.removed(Some(true)));
    assert!(watch.removed(Some(false)));
}

#[test]
fn an_unknown_registration_keeps_the_tray_open() {
    let mut watch = RegistrationWatch::default();
    assert!(!watch.removed(Some(true)));
    assert!(!watch.removed(None));
    assert!(!watch.removed(None));
    assert!(!watch.removed(Some(true)));
    assert!(!watch.removed(None));
    assert!(watch.removed(Some(false)));
}

#[test]
fn only_a_running_swing_counts_as_running() {
    assert!(is_running(Some(&snapshot(Status::Running {
        setup: true,
        signer_failed: false,
        instance: None,
    }))));
    assert!(!is_running(Some(&snapshot(Status::Stopped))));
    assert!(!is_running(Some(&snapshot(Status::Error("401".into())))));
    assert!(!is_running(None));
}

#[test]
fn japanese_locale_is_detected_case_insensitively() {
    assert_eq!(Lang::from_locale(Some("ja-JP")), Lang::Ja);
    assert_eq!(Lang::from_locale(Some("JA")), Lang::Ja);
    assert_eq!(Lang::from_locale(Some("en-US")), Lang::En);
    assert_eq!(Lang::from_locale(None), Lang::En);
}

#[test]
fn answers_come_from_the_labels_of_the_same_language() {
    assert_eq!(Answer::from_label(Lang::Ja, "はい"), Answer::Yes);
    assert_eq!(Answer::from_label(Lang::Ja, "いいえ"), Answer::No);
    assert_eq!(Answer::from_label(Lang::Ja, "キャンセル"), Answer::Cancel);
    assert_eq!(Answer::from_label(Lang::En, "Yes"), Answer::Yes);
    assert_eq!(Answer::from_label(Lang::En, "No"), Answer::No);
    assert_eq!(Answer::from_label(Lang::En, "Cancel"), Answer::Cancel);
    assert_eq!(Answer::from_label(Lang::Ja, "Yes"), Answer::Cancel);
    assert_eq!(Answer::from_label(Lang::En, ""), Answer::Cancel);
}
