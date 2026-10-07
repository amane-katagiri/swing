use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

mod access;
mod binary;
mod config;
mod daemon;
mod orphan;
#[cfg(test)]
mod tests;

pub use access::{
    ApiAccess, ApiSecret, ensure_own_daemon, managed_client, remove_api_access, write_api_access,
};
pub use binary::{ensure_repo, locate_binary, version};
pub use config::{KuboSettings, apply_config, multiaddr_to_http_url};
pub use daemon::{Daemon, daemon_stop_budget, wait_healthy};
pub use orphan::{recover_orphan, remove_pid_file, write_pid_file};

pub const KUBO_VERSION: &str = "0.43.1";

fn read_optional_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    serde_json::from_str(&text)
        .map(Some)
        .with_context(|| format!("parsing {}", path.display()))
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

pub fn pick_free_port() -> Result<u16> {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").context("binding an ephemeral port")?;
    let port = listener
        .local_addr()
        .context("reading ephemeral port")?
        .port();
    Ok(port)
}

async fn wait_until<F, Fut>(timeout: Duration, interval: Duration, mut ready: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if ready().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(interval).await;
    }
}
