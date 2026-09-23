use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

const FORCE_EXIT_GRACE_PERIOD: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Stop,
    Restart,
}

#[derive(Clone)]
pub struct ExitRequest {
    token: CancellationToken,
    restart: Arc<AtomicBool>,
}

impl ExitRequest {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            restart: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn stop(&self) {
        self.token.cancel();
    }

    pub fn restart(&self) {
        self.restart.store(true, Ordering::SeqCst);
        self.token.cancel();
    }

    pub fn restart_requested(&self) -> bool {
        self.restart.load(Ordering::SeqCst)
    }

    pub fn exit(&self) -> Exit {
        if self.restart_requested() {
            Exit::Restart
        } else {
            Exit::Stop
        }
    }
}

pub fn cancel_on_signal() -> Result<CancellationToken> {
    let token = CancellationToken::new();
    spawn_watcher(token.clone())?;
    Ok(token)
}

#[cfg(unix)]
fn spawn_watcher(token: CancellationToken) -> Result<()> {
    use anyhow::Context;
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).context("registering SIGTERM handler")?;
    tokio::spawn(async move {
        let signal = tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        };
        on_signal(token, signal).await;
    });
    Ok(())
}

#[cfg(not(unix))]
fn spawn_watcher(token: CancellationToken) -> Result<()> {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        on_signal(token, "ctrl-c").await;
    });
    Ok(())
}

async fn on_signal(token: CancellationToken, signal: &'static str) {
    info!(signal, "shutdown requested");
    token.cancel();
    tokio::time::sleep(FORCE_EXIT_GRACE_PERIOD).await;
    error!(
        grace_period = ?FORCE_EXIT_GRACE_PERIOD,
        "graceful shutdown did not finish within the grace period; forcing exit"
    );
    std::process::exit(1);
}
