use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;
use swing::api_client::{ApiClient, ApiClientError};
use swing::config::Config;
use swing::{login, service, stop};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

use crate::status::{RegistrationWatch, Snapshot, Status};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const FAST_POLL_INTERVAL: Duration = Duration::from_secs(1);
const FAST_POLL_WINDOW: Duration = Duration::from_secs(90);
const OVERVIEW_TIMEOUT: Duration = Duration::from_secs(5);
const INSTALLED_CACHE: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Restart,
    Stop,
    Start,
    StopAndQuit,
}

#[derive(Debug)]
pub enum Update {
    Snapshot(Snapshot),
    AutoStarting,
    ActionDone,
    ActionFailed(String),
    ReadyToQuit,
    ServiceRemoved,
}

struct Poller {
    installed: Option<(bool, Instant)>,
}

impl Poller {
    async fn snapshot(&mut self, config_path: Option<&Path>) -> Snapshot {
        let service_installed = self.service_installed().await;
        let config = match Config::load(config_path) {
            Ok(config) => config,
            Err(e) => {
                return Snapshot {
                    status: Status::Error(format!("{e:#}")),
                    ui: false,
                    service_installed,
                };
            }
        };
        let status = match ApiClient::for_config(&config) {
            Err(e) => Status::Error(format!("{e:#}")),
            Ok(client) => {
                match tokio::time::timeout(OVERVIEW_TIMEOUT, client.get::<Value>("/api/overview"))
                    .await
                {
                    Err(_) => Status::Error("the dashboard API did not respond".into()),
                    Ok(Ok(overview)) => Status::from_overview(&overview),
                    Ok(Err(ApiClientError::Unreachable(_))) => Status::Stopped,
                    Ok(Err(e)) => Status::Error(e.to_string()),
                }
            }
        };
        Snapshot {
            status,
            ui: config.dashboard.ui,
            service_installed,
        }
    }

    async fn service_installed(&mut self) -> bool {
        if let Some((installed, at)) = self.installed
            && at.elapsed() < INSTALLED_CACHE
        {
            return installed;
        }
        let installed = tokio::task::spawn_blocking(|| service::is_installed(false))
            .await
            .unwrap_or(false);
        self.installed = Some((installed, Instant::now()));
        installed
    }
}

pub async fn run(
    config_path: Option<PathBuf>,
    mut actions: UnboundedReceiver<Action>,
    send: impl Fn(Update),
) {
    let mut poller = Poller { installed: None };
    let mut next_poll = Instant::now();
    let mut fast_until = Instant::now();
    let mut first_poll = true;
    let mut registration = RegistrationWatch::default();
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(next_poll) => {
                let snapshot = poller.snapshot(config_path.as_deref()).await;
                let auto_start = first_poll
                    && snapshot.status == Status::Stopped
                    && snapshot.service_installed;
                first_poll = false;
                let removed = registration.removed(snapshot.service_installed);
                send(Update::Snapshot(snapshot));
                if removed {
                    send(Update::ServiceRemoved);
                    return;
                }
                if auto_start {
                    send(Update::AutoStarting);
                    send(finish(perform(config_path.as_deref(), Action::Start).await));
                    fast_until = Instant::now() + FAST_POLL_WINDOW;
                }
                let interval = if Instant::now() < fast_until {
                    FAST_POLL_INTERVAL
                } else {
                    POLL_INTERVAL
                };
                next_poll = Instant::now() + interval;
            }
            action = actions.recv() => {
                let Some(action) = action else { return };
                let result = perform(config_path.as_deref(), action).await;
                match (action, result) {
                    (Action::StopAndQuit, Ok(())) => send(Update::ReadyToQuit),
                    (_, result) => send(finish(result)),
                }
                fast_until = Instant::now() + FAST_POLL_WINDOW;
                next_poll = Instant::now() + FAST_POLL_INTERVAL;
            }
        }
    }
}

fn finish(result: Result<()>) -> Update {
    match result {
        Ok(()) => Update::ActionDone,
        Err(e) => Update::ActionFailed(format!("{e:#}")),
    }
}

async fn post(config_path: Option<&Path>, path: &str) -> Result<()> {
    let config = Config::load(config_path)?;
    ApiClient::for_config(&config)?
        .post::<Value>(path)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

async fn perform(config_path: Option<&Path>, action: Action) -> Result<()> {
    match action {
        Action::Open => {
            let config = Config::load(config_path)?;
            let link = login::request_link(&config).await?;
            tokio::task::spawn_blocking(move || login::open_browser(&link.url)).await?
        }
        Action::Restart => post(config_path, "/api/restart").await,
        Action::Stop => post(config_path, "/api/shutdown").await,
        Action::Start => tokio::task::spawn_blocking(|| service::start(false)).await?,
        Action::StopAndQuit => {
            let config = Config::load(config_path)?;
            stop::run(&config, false, STOP_TIMEOUT).await
        }
    }
}
