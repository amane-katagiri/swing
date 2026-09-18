use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use nostr_sdk::prelude::*;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::config::{Config, DashboardListen};
use crate::dashboard;
use crate::ipfs::IpfsClient;
use crate::nip05::HttpNip05Verifier;
use crate::nostr::{self, RelayClient};
use crate::state::State;

use super::Agent;

const DASHBOARD_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const FORCE_EXIT_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn run(config: Config) -> Result<()> {
    let relay =
        Arc::new(RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?);
    info!(relays = ?relay.relays(), "connected to relays");

    let ipfs = IpfsClient::new(config.ipfs.api.clone());
    let state_path = config.agent.state_dir.join("state.json");
    let state = State::load(&state_path).await?;
    info!(path = %state_path.display(), sites = state.sites.len(), "loaded state");

    let dashboard_listener = match &config.dashboard.listen {
        DashboardListen::Off => None,
        DashboardListen::Addr(addr) => {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("binding dashboard listener on {addr}"))?;
            if !addr.ip().is_loopback() || !config.dashboard.allowed_hosts.is_empty() {
                warn!(
                    %addr,
                    allowed_hosts = ?config.dashboard.allowed_hosts,
                    "dashboard has no authentication; binding beyond loopback or widening allowed_hosts exposes full mirror and publish control to anyone who can reach it"
                );
            }
            Some(listener)
        }
    };

    let site_event_kind = config.nostr.site_event_kind;
    let mut poll_timer = tokio::time::interval(config.agent.poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let notify = Arc::new(Notify::new());
    let dashboard_config = config.clone();
    let agent = Arc::new(Agent::new(
        config,
        ipfs,
        HttpNip05Verifier::public_only(),
        Arc::clone(&relay),
        state,
        state_path,
    ));
    let mut sigint = signal(SignalKind::interrupt()).context("registering SIGINT handler")?;
    let mut sigterm = signal(SignalKind::terminate()).context("registering SIGTERM handler")?;

    // Independent of the main loop so it still fires when that loop is stuck.
    let mut watchdog_sigint =
        signal(SignalKind::interrupt()).context("registering watchdog SIGINT handler")?;
    let mut watchdog_sigterm =
        signal(SignalKind::terminate()).context("registering watchdog SIGTERM handler")?;
    tokio::spawn(async move {
        tokio::select! {
            _ = watchdog_sigint.recv() => {}
            _ = watchdog_sigterm.recv() => {}
        }
        tokio::time::sleep(FORCE_EXIT_GRACE_PERIOD).await;
        error!(
            grace_period = ?FORCE_EXIT_GRACE_PERIOD,
            "graceful shutdown did not finish within the grace period; forcing exit"
        );
        std::process::exit(1);
    });

    // A select! arm body is not polled against the other arms, so a signal arriving during it would wait for the body to finish.
    if let Some(signal) = race_with_shutdown(agent.reconcile(), &mut sigint, &mut sigterm).await {
        info!(signal, "shutdown requested during startup reconciliation");
        relay.client.shutdown().await;
        return Ok(());
    }

    let mut dashboard_shutdown = None;
    let mut dashboard_task = None;
    if let Some(listener) = dashboard_listener {
        dashboard::cleanup_upload_dir(&dashboard_config.agent.state_dir)
            .await
            .context("cleaning up leftover dashboard uploads")?;
        let dashboard_state = Arc::new(dashboard::AppState::new(
            Some(Arc::clone(&relay)),
            Arc::new(dashboard_config),
            Arc::clone(&notify),
        )?);
        let (tx, rx) = tokio::sync::oneshot::channel();
        dashboard_shutdown = Some(tx);
        dashboard_task = Some(tokio::spawn(async move {
            if let Err(e) = dashboard::serve(listener, dashboard_state, rx).await {
                error!(error = %e, "dashboard server stopped");
            }
        }));
    }

    let mut tasks = JoinSet::new();
    let mut notifications = relay.notifications();

    loop {
        tokio::select! {
            maybe_note = notifications.next() => {
                match maybe_note {
                    Some(ClientNotification::Event { event, subscription_id, .. }) => {
                        if subscription_id.as_str() == nostr::SITE_SUBSCRIPTION_ID
                            && event.kind == Kind::Custom(site_event_kind)
                        {
                            match nostr::parse_site_event(&event, site_event_kind) {
                                Ok(ev) => agent.submit(ev, &mut tasks),
                                Err(e) => warn!(error = %e, "skipping invalid site event"),
                            }
                        } else {
                            debug!(
                                kind = %event.kind,
                                subscription_id = %subscription_id,
                                "ignoring notification outside the site subscription"
                            );
                        }
                    }
                    Some(ClientNotification::Shutdown) | None => {
                        anyhow::bail!("relay notification stream ended");
                    }
                    Some(_) => {}
                }
            }
            _ = poll_timer.tick() => {
                if let Some(signal) = race_with_shutdown(super::poll_once(&relay, &agent, &mut tasks), &mut sigint, &mut sigterm).await {
                    info!(signal, "shutdown requested during poll");
                    shutdown_dashboard(&mut dashboard_shutdown, &mut dashboard_task).await;
                    relay.client.shutdown().await;
                    break;
                }
            }
            _ = notify.notified() => {
                if let Some(signal) = race_with_shutdown(super::poll_once(&relay, &agent, &mut tasks), &mut sigint, &mut sigterm).await {
                    info!(signal, "shutdown requested during poll");
                    shutdown_dashboard(&mut dashboard_shutdown, &mut dashboard_task).await;
                    relay.client.shutdown().await;
                    break;
                }
            }
            Some(joined) = tasks.join_next() => {
                if let Err(e) = joined {
                    error!(error = %e, "site task panicked");
                }
            }
            joined = async { dashboard_task.as_mut().unwrap().await }, if dashboard_task.is_some() => {
                match joined {
                    Ok(()) => error!("dashboard server task exited unexpectedly"),
                    Err(e) => error!(error = %e, "dashboard server task panicked"),
                }
                dashboard_task = None;
            }
            _ = sigint.recv() => {
                info!(signal = "SIGINT", "shutdown requested");
                shutdown_dashboard(&mut dashboard_shutdown, &mut dashboard_task).await;
                relay.client.shutdown().await;
                break;
            }
            _ = sigterm.recv() => {
                info!(signal = "SIGTERM", "shutdown requested");
                shutdown_dashboard(&mut dashboard_shutdown, &mut dashboard_task).await;
                relay.client.shutdown().await;
                break;
            }
        }
    }
    Ok(())
}

async fn race_with_shutdown<F: std::future::Future<Output = ()>>(
    fut: F,
    sigint: &mut tokio::signal::unix::Signal,
    sigterm: &mut tokio::signal::unix::Signal,
) -> Option<&'static str> {
    tokio::select! {
        _ = fut => None,
        _ = sigint.recv() => Some("SIGINT"),
        _ = sigterm.recv() => Some("SIGTERM"),
    }
}

async fn shutdown_dashboard(
    dashboard_shutdown: &mut Option<tokio::sync::oneshot::Sender<()>>,
    dashboard_task: &mut Option<tokio::task::JoinHandle<()>>,
) {
    if let Some(tx) = dashboard_shutdown.take() {
        let _ = tx.send(());
    }
    if let Some(task) = dashboard_task.take() {
        match tokio::time::timeout(DASHBOARD_SHUTDOWN_TIMEOUT, task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error!(error = %e, "dashboard server task panicked during shutdown"),
            Err(_) => warn!(
                timeout = ?DASHBOARD_SHUTDOWN_TIMEOUT,
                "dashboard server did not shut down in time; leaving it behind"
            ),
        }
    }
}
