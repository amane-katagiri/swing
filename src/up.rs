use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::agent;
use crate::auth;
use crate::config::{Config, IpfsApi};
use crate::dashboard;
use crate::kubo;
use crate::lock;
use crate::shutdown::{Exit, ExitRequest, RUNTIME_SHUTDOWN_TIMEOUT};
use crate::signer::Signer;

const UNMANAGED_HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const MANAGED_HEALTH_TIMEOUT: Duration = Duration::from_secs(120);
const DAEMON_STOP_GRACE: Duration = Duration::from_secs(20);
const AGENT_STOP_TIMEOUT: Duration = Duration::from_secs(15);
const DASHBOARD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const FORCE_EXIT_MARGIN: Duration = Duration::from_secs(5);

const STOP_BUDGET: Duration = AGENT_STOP_TIMEOUT
    .saturating_add(kubo::daemon_stop_budget(DAEMON_STOP_GRACE))
    .saturating_add(DASHBOARD_SHUTDOWN_TIMEOUT);

pub const FORCE_EXIT_GRACE: Duration = STOP_BUDGET
    .saturating_add(RUNTIME_SHUTDOWN_TIMEOUT)
    .saturating_add(FORCE_EXIT_MARGIN);

struct Backoff {
    delay: Duration,
}

impl Backoff {
    const MIN: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(60);

    fn new() -> Self {
        Self { delay: Self::MIN }
    }

    fn next_delay(&mut self, ran_for: Duration) -> Duration {
        if ran_for >= Self::MAX {
            self.delay = Self::MIN;
        }
        let delay = self.delay;
        self.delay = (self.delay * 2).min(Self::MAX);
        delay
    }

    async fn wait(&mut self, ran_for: Duration, token: &CancellationToken) {
        let delay = self.next_delay(ran_for);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = token.cancelled() => {}
        }
    }
}

async fn stop_daemon(daemon: kubo::Daemon, config: &Config, grace: Duration) -> Result<()> {
    let result = daemon.stop(grace).await;
    if let Err(e) = kubo::remove_pid_file(&config.agent.state_dir) {
        warn!(error = %e, "failed to remove kubo.pid");
    }
    result
}

fn spawn_agent(
    config: Config,
    token: CancellationToken,
    dashboard: &Arc<dashboard::AppState>,
    notify: &Arc<Notify>,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::spawn(agent::run_until(
        config,
        token,
        Arc::clone(dashboard),
        Arc::clone(notify),
    ))
}

enum StartOutcome {
    Ready(Box<kubo::Daemon>, String),
    Retry,
    Cancelled,
}

async fn start_kubo(
    bin: &Path,
    config: &Config,
    backoff: &mut Backoff,
    token: &CancellationToken,
) -> Result<StartOutcome> {
    macro_rules! attempt {
        ($result:expr, $msg:literal) => {
            match $result {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, $msg);
                    backoff.wait(Duration::ZERO, token).await;
                    return Ok(StartOutcome::Retry);
                }
            }
        };
    }

    let initialised = attempt!(
        kubo::ensure_repo(bin, &config.kubo.repo).await,
        "failed to prepare Kubo repo"
    );
    if initialised {
        info!(repo = %config.kubo.repo.display(), "initialised Kubo repo");
    }

    let api_port = attempt!(
        kubo::pick_free_port(),
        "failed to pick a free port for the Kubo API"
    );
    let settings = kubo::KuboSettings {
        storage_max: config.kubo.storage_max,
        provide_strategy: config.kubo.provide_strategy.clone(),
        api_port,
        gateway: config.kubo.gateway_listen,
        swarm_port: config.kubo.swarm_port,
        public_gateway_hosts: config.gateway.hosts.clone(),
    };
    attempt!(
        kubo::apply_config(bin, &config.kubo.repo, &settings).await,
        "failed to configure Kubo"
    );

    let api_url = format!("http://127.0.0.1:{api_port}");
    let mut daemon = attempt!(
        kubo::Daemon::spawn(bin, &config.kubo.repo, api_url.clone()).await,
        "failed to spawn the Kubo daemon"
    );
    if let Some(pid) = daemon.pid()
        && let Err(e) = kubo::write_pid_file(&config.agent.state_dir, pid, api_port)
    {
        warn!(error = %e, "failed to write kubo.pid");
    }

    let health = tokio::select! {
        res = kubo::wait_healthy(&api_url, MANAGED_HEALTH_TIMEOUT) => Some(res),
        _ = token.cancelled() => None,
    };
    let health = match health {
        None => {
            let _ = stop_daemon(daemon, config, DAEMON_STOP_GRACE).await;
            return Ok(StartOutcome::Cancelled);
        }
        Some(h) => h,
    };
    if let Err(e) = health {
        warn!(error = %e, "Kubo did not become healthy");
        if matches!(daemon.try_wait(), Ok(Some(_))) && daemon.saw_repo_lock_error() {
            warn!(
                repo = %config.kubo.repo.display(),
                "another ipfs daemon seems to hold the Kubo repo lock; stop it or point [kubo].repo elsewhere"
            );
        }
        let _ = stop_daemon(daemon, config, DAEMON_STOP_GRACE).await;
        backoff.wait(Duration::ZERO, token).await;
        return Ok(StartOutcome::Retry);
    }

    info!(api = %api_url, "kubo is ready");
    Ok(StartOutcome::Ready(Box::new(daemon), api_url))
}

pub async fn run(config: Config, token: CancellationToken) -> Result<Exit> {
    let _lock = lock::acquire(&config.agent.state_dir)?;
    let exit = ExitRequest::new(token.clone());
    let notify = Arc::new(Notify::new());

    dashboard::cleanup_upload_dir(&config.agent.state_dir)
        .await
        .context("cleaning up leftover dashboard uploads")?;

    let signer = Signer::load(&config)?;
    let setup_mode = signer.is_none();

    let dashboard_token = auth::load_or_create_token(&config.agent.state_dir)
        .context("preparing the dashboard token")?;
    let dashboard_state = Arc::new(dashboard::AppState::new(
        Arc::new(config.clone()),
        Arc::clone(&notify),
        exit.clone(),
        signer,
        dashboard_token,
    )?);

    let addr = config.dashboard.listen;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding dashboard listener on {addr}"))?;
    if !addr.ip().is_loopback() {
        warn!(
            %addr,
            "dashboard is served over plain HTTP beyond loopback; without a TLS-terminating HTTP reverse proxy in front, login codes and session cookies travel in cleartext"
        );
    }

    let (dashboard_shutdown_tx, dashboard_shutdown_rx) = tokio::sync::oneshot::channel();
    let dashboard_for_serve = Arc::clone(&dashboard_state);
    let dashboard_task = tokio::spawn(async move {
        if let Err(e) = dashboard::serve(listener, dashboard_for_serve, dashboard_shutdown_rx).await
        {
            error!(error = %e, "dashboard server stopped");
        }
    });

    let result = if setup_mode {
        info!(
            "no Nostr key or signer app configured; running in setup mode (dashboard only, waiting for setup)"
        );
        token.cancelled().await;
        Ok(())
    } else if config.kubo.managed {
        run_managed(
            config,
            token,
            Arc::clone(&dashboard_state),
            Arc::clone(&notify),
        )
        .await
    } else {
        run_unmanaged(
            config,
            token,
            Arc::clone(&dashboard_state),
            Arc::clone(&notify),
        )
        .await
    };

    let _ = dashboard_shutdown_tx.send(());
    if tokio::time::timeout(DASHBOARD_SHUTDOWN_TIMEOUT, dashboard_task)
        .await
        .is_err()
    {
        warn!(
            timeout = ?DASHBOARD_SHUTDOWN_TIMEOUT,
            "dashboard server did not shut down in time; leaving it behind"
        );
    }
    if let Some(signer) = &dashboard_state.signer {
        signer.shutdown().await;
    }

    result?;
    Ok(exit.exit())
}

async fn run_unmanaged(
    config: Config,
    token: CancellationToken,
    dashboard: Arc<dashboard::AppState>,
    notify: Arc<Notify>,
) -> Result<()> {
    let mut backoff = Backoff::new();
    loop {
        let api_url = config.ipfs_api_url()?;
        let health = tokio::select! {
            res = kubo::wait_healthy(&api_url, UNMANAGED_HEALTH_TIMEOUT) => Some(res),
            _ = token.cancelled() => None,
        };
        let Some(health) = health else {
            return Ok(());
        };
        if let Err(e) = health {
            warn!(error = %e, "external Kubo is not healthy yet");
            backoff.wait(Duration::ZERO, &token).await;
            continue;
        }
        info!(api = %api_url, "external Kubo is ready");

        let started = Instant::now();
        let agent = agent::run_until(
            config.clone(),
            token.child_token(),
            Arc::clone(&dashboard),
            Arc::clone(&notify),
        );
        tokio::pin!(agent);
        let result = tokio::select! {
            result = &mut agent => result,
            _ = token.cancelled() => match tokio::time::timeout(AGENT_STOP_TIMEOUT, &mut agent).await {
                Ok(result) => result,
                Err(_) => {
                    warn!(timeout = ?AGENT_STOP_TIMEOUT, "agent did not stop in time during shutdown");
                    return Ok(());
                }
            },
        };
        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!(error = %e, "agent exited with an error; restarting");
                backoff.wait(started.elapsed(), &token).await;
            }
        }
    }
}

async fn run_managed(
    config: Config,
    token: CancellationToken,
    dashboard: Arc<dashboard::AppState>,
    notify: Arc<Notify>,
) -> Result<()> {
    let bin = kubo::locate_binary(config.kubo.binary.as_deref())?;
    let installed_version = kubo::version(&bin).await?;
    if installed_version != kubo::KUBO_VERSION {
        warn!(
            installed = %installed_version,
            expected = %kubo::KUBO_VERSION,
            "Kubo version does not match the version swing was tested with"
        );
    }

    tokio::select! {
        result = kubo::recover_orphan(&config.agent.state_dir, &config.kubo.repo) => result?,
        _ = token.cancelled() => return Ok(()),
    }

    let mut backoff = Backoff::new();

    'daemon: loop {
        if token.is_cancelled() {
            return Ok(());
        }

        let (mut daemon, api_url) = match start_kubo(&bin, &config, &mut backoff, &token).await? {
            StartOutcome::Ready(daemon, api_url) => (*daemon, api_url),
            StartOutcome::Retry => continue 'daemon,
            StartOutcome::Cancelled => return Ok(()),
        };
        let mut managed_config = config.clone();
        managed_config.ipfs.api = IpfsApi::Url(api_url);

        let mut agent_token = token.child_token();
        let mut agent_handle = spawn_agent(
            managed_config.clone(),
            agent_token.clone(),
            &dashboard,
            &notify,
        );
        let mut agent_started = Instant::now();
        let daemon_started = Instant::now();

        loop {
            tokio::select! {
                status = daemon.wait() => {
                    match status {
                        Ok(status) => error!(%status, "kubo daemon exited unexpectedly"),
                        Err(e) => error!(error = %e, "waiting for the kubo daemon failed"),
                    }
                    agent_token.cancel();
                    if tokio::time::timeout(AGENT_STOP_TIMEOUT, &mut agent_handle).await.is_err() {
                        warn!(timeout = ?AGENT_STOP_TIMEOUT, "agent did not stop in time after kubo exited");
                        agent_handle.abort();
                    }
                    if let Err(e) = kubo::remove_pid_file(&config.agent.state_dir) {
                        warn!(error = %e, "failed to remove kubo.pid");
                    }
                    backoff.wait(daemon_started.elapsed(), &token).await;
                    continue 'daemon;
                }
                result = &mut agent_handle => {
                    let ran_for = agent_started.elapsed();
                    match result {
                        Ok(Ok(())) => {
                            agent_token.cancel();
                            let _ = stop_daemon(daemon, &config, DAEMON_STOP_GRACE).await;
                            return Ok(());
                        }
                        Ok(Err(e)) => warn!(error = %e, "agent exited with an error; restarting agent"),
                        Err(e) => error!(error = %e, "agent task panicked; restarting agent"),
                    }
                    backoff.wait(ran_for, &token).await;
                    if token.is_cancelled() {
                        let _ = stop_daemon(daemon, &config, DAEMON_STOP_GRACE).await;
                        return Ok(());
                    }
                    agent_token = token.child_token();
                    agent_started = Instant::now();
                    agent_handle = spawn_agent(
                        managed_config.clone(),
                        agent_token.clone(),
                        &dashboard,
                        &notify,
                    );
                }
                _ = token.cancelled() => {
                    agent_token.cancel();
                    if tokio::time::timeout(AGENT_STOP_TIMEOUT, &mut agent_handle).await.is_err() {
                        warn!(timeout = ?AGENT_STOP_TIMEOUT, "agent did not stop in time during shutdown");
                        agent_handle.abort();
                    }
                    stop_daemon(daemon, &config, DAEMON_STOP_GRACE).await?;
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_budget_fits_within_force_exit_and_service_manager_limits() {
        let unmanaged = AGENT_STOP_TIMEOUT + DASHBOARD_SHUTDOWN_TIMEOUT;
        assert!(unmanaged <= STOP_BUDGET);
        assert_eq!(
            STOP_BUDGET,
            AGENT_STOP_TIMEOUT
                + kubo::daemon_stop_budget(DAEMON_STOP_GRACE)
                + DASHBOARD_SHUTDOWN_TIMEOUT
        );
        assert!(STOP_BUDGET + RUNTIME_SHUTDOWN_TIMEOUT < FORCE_EXIT_GRACE);
        assert!(FORCE_EXIT_GRACE < crate::service::STOP_TIMEOUT);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut b = Backoff::new();
        let seq: Vec<u64> = (0..8)
            .map(|_| b.next_delay(Duration::ZERO).as_secs())
            .collect();
        assert_eq!(seq, vec![1, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn backoff_resets_after_a_long_run() {
        let mut b = Backoff::new();
        for _ in 0..5 {
            b.next_delay(Duration::ZERO);
        }
        assert_eq!(b.delay, Duration::from_secs(32));
        let delay = b.next_delay(Duration::from_secs(120));
        assert_eq!(delay, Duration::from_secs(1));
        assert_eq!(b.delay, Duration::from_secs(2));
    }

    #[test]
    fn backoff_boundary_run_of_exactly_max_resets() {
        let mut b = Backoff::new();
        b.next_delay(Duration::ZERO);
        let delay = b.next_delay(Duration::from_secs(60));
        assert_eq!(delay, Duration::from_secs(1));
    }
}
