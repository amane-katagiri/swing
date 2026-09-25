use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Ja,
    En,
}

impl Lang {
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(l) if l.to_ascii_lowercase().starts_with("ja") => Self::Ja,
            _ => Self::En,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running {
        setup: bool,
        signer_failed: bool,
        instance: Option<String>,
    },
    Stopped,
    Error(String),
}

impl Status {
    pub fn from_overview(overview: &Value) -> Self {
        let setup = overview
            .get("setup")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let signer_failed = overview
            .pointer("/signer/last_failure")
            .is_some_and(|v| !v.is_null());
        let instance = overview
            .get("instance")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Self::Running {
            setup,
            signer_failed,
            instance,
        }
    }

    fn instance(&self) -> Option<&str> {
        match self {
            Self::Running { instance, .. } => instance.as_deref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: Status,
    pub ui: bool,
    pub service_installed: Option<bool>,
}

pub fn is_running(snapshot: Option<&Snapshot>) -> bool {
    snapshot.is_some_and(|s| matches!(s.status, Status::Running { .. }))
}

#[derive(Default)]
pub struct RegistrationWatch {
    seen: bool,
}

impl RegistrationWatch {
    pub fn removed(&mut self, installed: Option<bool>) -> bool {
        match installed {
            Some(true) => {
                self.seen = true;
                false
            }
            Some(false) => self.seen,
            None => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    Starting,
    Stopping,
    Restarting { from: Option<String> },
    Quitting,
}

impl Pending {
    pub fn restarting(snapshot: Option<&Snapshot>) -> Self {
        Self::Restarting {
            from: snapshot
                .and_then(|s| s.status.instance())
                .map(str::to_owned),
        }
    }

    // Quitting never settles from a poll: the tray exits once the stop it asked for has finished.
    pub fn settled_by(&self, status: &Status) -> bool {
        match self {
            Self::Starting => matches!(status, Status::Running { .. }),
            Self::Stopping => *status == Status::Stopped,
            Self::Restarting { from } => {
                matches!(status, Status::Running { .. }) && status.instance() != from.as_deref()
            }
            Self::Quitting => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct MenuState {
    pub status_text: String,
    pub open: bool,
    pub restart: bool,
    pub stop: bool,
    pub start: bool,
    pub quit: bool,
    pub active_icon: bool,
}

const MAX_MESSAGE_CHARS: usize = 80;

fn shorten(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    if line.chars().count() <= MAX_MESSAGE_CHARS {
        return line.to_owned();
    }
    let head: String = line.chars().take(MAX_MESSAGE_CHARS - 1).collect();
    format!("{head}…")
}

pub fn menu_state(
    snapshot: &Snapshot,
    pending: Option<&Pending>,
    action_error: Option<&str>,
    lang: Lang,
) -> MenuState {
    let l = labels(lang);
    let running = matches!(snapshot.status, Status::Running { .. });
    let stopped = snapshot.status == Status::Stopped;
    let status_text = match (action_error, pending) {
        (Some(e), _) => format!("{}{}", l.failed, shorten(e)),
        (None, Some(Pending::Starting)) => l.starting.to_owned(),
        (None, Some(Pending::Stopping | Pending::Quitting)) => l.stopping.to_owned(),
        (None, Some(Pending::Restarting { .. })) => l.restarting.to_owned(),
        (None, None) => status_text(snapshot, lang),
    };
    let idle = pending.is_none();
    MenuState {
        status_text,
        open: idle && running && snapshot.ui,
        restart: idle && running,
        stop: idle && running,
        start: idle && stopped && snapshot.service_installed != Some(false),
        quit: pending != Some(&Pending::Quitting),
        active_icon: running,
    }
}

fn status_text(snapshot: &Snapshot, lang: Lang) -> String {
    let l = labels(lang);
    match &snapshot.status {
        Status::Running { setup: true, .. } => l.setup.to_owned(),
        Status::Running {
            signer_failed: true,
            ..
        } => l.signer_failed.to_owned(),
        Status::Running { .. } => l.running.to_owned(),
        Status::Stopped if snapshot.service_installed == Some(false) => {
            l.stopped_not_installed.to_owned()
        }
        Status::Stopped => l.stopped.to_owned(),
        Status::Error(e) => format!("{}{}", l.error, shorten(e)),
    }
}

pub struct Labels {
    pub running: &'static str,
    pub setup: &'static str,
    pub signer_failed: &'static str,
    pub stopped: &'static str,
    pub stopped_not_installed: &'static str,
    pub error: &'static str,
    pub starting: &'static str,
    pub stopping: &'static str,
    pub restarting: &'static str,
    pub failed: &'static str,
    pub open: &'static str,
    pub restart: &'static str,
    pub stop: &'static str,
    pub start: &'static str,
    pub quit: &'static str,
    pub quit_confirm: &'static str,
    pub stop_confirm: &'static str,
}

pub fn labels(lang: Lang) -> &'static Labels {
    match lang {
        Lang::Ja => &Labels {
            running: "SWING: 動作中",
            setup: "SWING: セットアップ待ち",
            signer_failed: "SWING: 動作中（前回の署名に失敗しました）",
            stopped: "SWING: 停止中",
            stopped_not_installed: "SWING: 停止中（サービス未登録）",
            error: "SWING: エラー: ",
            starting: "SWING: 起動しています…",
            stopping: "SWING: 停止しています…",
            restarting: "SWING: 再起動しています…",
            failed: "操作に失敗しました: ",
            open: "ダッシュボードを開く",
            restart: "再起動",
            stop: "停止",
            start: "起動",
            quit: "終了",
            stop_confirm: "SWING を停止しますか？\n\nミラーしているサイトの取得と配信が止まります。",
            quit_confirm: "SWING を停止してからトレイを閉じますか？\n\n「いいえ」を選ぶと、SWING は動かしたままトレイだけを閉じます。",
        },
        Lang::En => &Labels {
            running: "SWING: Running",
            setup: "SWING: Waiting for setup",
            signer_failed: "SWING: Running (the last signing request failed)",
            stopped: "SWING: Stopped",
            stopped_not_installed: "SWING: Stopped (not registered as a service)",
            error: "SWING: Error: ",
            starting: "SWING: Starting…",
            stopping: "SWING: Stopping…",
            restarting: "SWING: Restarting…",
            failed: "Action failed: ",
            open: "Open dashboard",
            restart: "Restart",
            stop: "Stop",
            start: "Start",
            quit: "Quit",
            stop_confirm: "Stop SWING?\n\nMirrored sites will no longer be fetched or served.",
            quit_confirm: "Stop SWING before closing the tray?\n\nChoose No to close only the tray and keep SWING running.",
        },
    }
}

#[cfg(test)]
mod tests {
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
}
