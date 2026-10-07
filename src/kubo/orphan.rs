use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use tokio::process::Command;

use crate::proc::{process_alive, process_start_marker};

use super::access::{ApiSecret, read_api_access};
use super::{read_optional_json, remove_if_present, wait_until};

const ORPHAN_SHUTDOWN_RPC_TIMEOUT: Duration = Duration::from_secs(3);
const ORPHAN_SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
#[cfg(unix)]
const ORPHAN_SIGTERM_GRACE: Duration = Duration::from_secs(30);
const ORPHAN_KILL_WAIT: Duration = Duration::from_secs(10);

fn pid_file_path(state_dir: &Path) -> PathBuf {
    state_dir.join("kubo.pid")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PidRecord {
    pub(super) pid: u32,
    pub(super) api_port: u16,
    pub(super) started_at: String,
}

pub fn write_pid_file(state_dir: &Path, pid: u32, api_port: u16) -> Result<()> {
    let started_at = process_start_marker(pid)
        .with_context(|| format!("determining the start time of Kubo process {pid}"))?;
    let record = PidRecord {
        pid,
        api_port,
        started_at,
    };
    let text = serde_json::to_string(&record).context("serializing kubo.pid")?;
    crate::auth::write_private_file(&pid_file_path(state_dir), &text)
}

pub(super) fn read_pid_file(state_dir: &Path) -> Result<Option<PidRecord>> {
    read_optional_json(&pid_file_path(state_dir))
}

pub fn remove_pid_file(state_dir: &Path) -> Result<()> {
    remove_if_present(&pid_file_path(state_dir))
}

async fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    wait_until(timeout, Duration::from_millis(500), || async {
        !process_alive(pid)
    })
    .await
}

async fn attempt_graceful_shutdown(api_port: u16, pid: u32, secret: Option<&ApiSecret>) -> bool {
    let url = format!("http://127.0.0.1:{api_port}/api/v0/shutdown");
    let responded = crate::ipfs::kubo_http_client(secret)
        .post(&url)
        .timeout(ORPHAN_SHUTDOWN_RPC_TIMEOUT)
        .send()
        .await
        .is_ok_and(|resp| resp.status().is_success());
    if !responded {
        return false;
    }
    wait_for_exit(pid, ORPHAN_SHUTDOWN_GRACE).await
}

#[cfg(unix)]
async fn terminate_process(pid: u32) -> Result<()> {
    use crate::proc::send_signal;
    send_signal(pid, libc::SIGTERM).context("sending SIGTERM to orphaned Kubo")?;
    if wait_for_exit(pid, ORPHAN_SIGTERM_GRACE).await {
        return Ok(());
    }
    tracing::warn!(
        pid,
        "orphaned Kubo did not exit after SIGTERM, sending SIGKILL"
    );
    send_signal(pid, libc::SIGKILL).context("sending SIGKILL to orphaned Kubo")?;
    if wait_for_exit(pid, ORPHAN_KILL_WAIT).await {
        return Ok(());
    }
    bail!("orphaned Kubo (pid {pid}) did not exit after SIGKILL")
}

#[cfg(windows)]
async fn terminate_process(pid: u32) -> Result<()> {
    let output = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .output()
        .await
        .context("running taskkill")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("taskkill failed for pid {pid}: {}", stderr.trim());
    }
    if wait_for_exit(pid, ORPHAN_KILL_WAIT).await {
        Ok(())
    } else {
        bail!("orphaned Kubo (pid {pid}) did not exit after taskkill /F")
    }
}

pub async fn recover_orphan(state_dir: &Path, repo: &Path) -> Result<()> {
    let record = match read_pid_file(state_dir) {
        Ok(Some(record)) => record,
        Ok(None) => return Ok(()),
        Err(e) => {
            tracing::warn!(
                error = %format!("{e:#}"),
                repo = %repo.display(),
                "cannot read kubo.pid; assuming no Kubo from a previous swing is left running"
            );
            return Ok(());
        }
    };

    let Some(current_started_at) = process_start_marker(record.pid) else {
        tracing::info!(
            pid = record.pid,
            repo = %repo.display(),
            "stale kubo.pid (process is no longer running)"
        );
        remove_pid_file(state_dir)?;
        return Ok(());
    };
    if current_started_at != record.started_at {
        tracing::warn!(
            pid = record.pid,
            repo = %repo.display(),
            "pid in kubo.pid no longer belongs to the recorded Kubo (start time differs); leaving it alone"
        );
        remove_pid_file(state_dir)?;
        return Ok(());
    }

    let secret = read_api_access(state_dir)
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "cannot read the Kubo API access; asking the orphan to shut down without it");
            None
        })
        .filter(|access| access.port == record.api_port)
        .map(|access| access.secret);
    if attempt_graceful_shutdown(record.api_port, record.pid, secret.as_ref()).await {
        tracing::info!(
            pid = record.pid,
            repo = %repo.display(),
            "orphaned Kubo shut down gracefully via its API"
        );
        remove_pid_file(state_dir)?;
        return Ok(());
    }

    tracing::warn!(
        pid = record.pid,
        repo = %repo.display(),
        "terminating orphaned Kubo left by a previous swing"
    );
    terminate_process(record.pid).await?;
    remove_pid_file(state_dir)?;
    Ok(())
}
