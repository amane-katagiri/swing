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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Cancel,
}

impl Answer {
    pub fn from_label(lang: Lang, label: &str) -> Self {
        let l = labels(lang);
        [
            (l.yes, Self::Yes),
            (l.no, Self::No),
            (l.cancel, Self::Cancel),
        ]
        .into_iter()
        .find(|(l, _)| *l == label)
        .map_or(Self::Cancel, |(_, answer)| answer)
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
    pub yes: &'static str,
    pub no: &'static str,
    pub cancel: &'static str,
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
            yes: "はい",
            no: "いいえ",
            cancel: "キャンセル",
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
            yes: "Yes",
            no: "No",
            cancel: "Cancel",
        },
    }
}

#[cfg(test)]
mod tests;
