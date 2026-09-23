use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::Client;

use crate::config::{Config, Listen};
use crate::lock;

enum DashboardOutcome {
    Requested,
    Unreachable,
}

async fn post_dashboard_action(addr: SocketAddr, restart: bool) -> Result<DashboardOutcome> {
    let action = if restart { "restart" } else { "shutdown" };
    let url = format!("http://{addr}/api/{action}");
    let client = Client::new();
    let result = client
        .post(&url)
        .header("X-Swing-Dashboard", "1")
        .header(reqwest::header::HOST, addr.to_string())
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    match result {
        Ok(resp) if resp.status().is_success() => Ok(DashboardOutcome::Requested),
        Ok(resp) => bail!("dashboard returned {} for {url}", resp.status()),
        Err(e) if e.is_connect() || e.is_timeout() => Ok(DashboardOutcome::Unreachable),
        Err(e) => Err(e).with_context(|| format!("calling {url}")),
    }
}

#[cfg(unix)]
fn fallback_stop(config: &Config, restart: bool) -> Result<()> {
    if restart {
        bail!(
            "cannot request a restart without the dashboard: enable [dashboard].listen so `swing stop --restart` can reach it"
        );
    }
    let path = config.agent.state_dir.join("swing.lock");
    let pid = read_lock_pid(&path)?.with_context(|| {
        format!(
            "{} has no pid to signal (is swing running?)",
            path.display()
        )
    })?;
    let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err).context("sending SIGTERM to swing");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn fallback_stop(_config: &Config, _restart: bool) -> Result<()> {
    bail!(
        "the dashboard is off or not reachable; enable [dashboard].listen or end the process from Task Scheduler"
    )
}

#[cfg(unix)]
fn read_lock_pid(path: &Path) -> Result<Option<u32>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(text.trim().parse().ok())
}

fn instance_running(state_dir: &Path) -> bool {
    match lock::acquire(state_dir) {
        Ok(_lock) => false,
        Err(_) => true,
    }
}

pub async fn run(config: &Config, restart: bool, timeout: Duration) -> Result<()> {
    let state_dir = &config.agent.state_dir;
    if !instance_running(state_dir) {
        println!("not running");
        return Ok(());
    }

    let requested_via_dashboard = match &config.dashboard.listen {
        Listen::Addr(addr) => matches!(
            post_dashboard_action(*addr, restart).await?,
            DashboardOutcome::Requested
        ),
        Listen::Off => false,
    };

    if !requested_via_dashboard {
        fallback_stop(config, restart)?;
    }

    let deadline = Instant::now() + timeout;
    loop {
        if !instance_running(state_dir) {
            println!("stopped");
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("swing did not stop within {}s", timeout.as_secs());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
