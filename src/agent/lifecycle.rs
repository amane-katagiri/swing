use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use nostr_sdk::prelude::*;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::{Config, Listen};
use crate::dashboard;
use crate::gateway;
use crate::ipfs::IpfsClient;
use crate::nip05::HttpNip05Verifier;
use crate::nostr::{self, RelayClient};
use crate::shutdown::{Exit, ExitRequest};
use crate::state::State;

use super::Agent;

const DASHBOARD_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const GATEWAY_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub async fn run_until(config: Config, shutdown: CancellationToken) -> Result<Exit> {
    let exit = ExitRequest::new(shutdown.clone());
    let relay =
        Arc::new(RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?);
    info!(relays = ?relay.relays(), "connected to relays");

    let ipfs = IpfsClient::new(config.ipfs_api_url()?);
    let state_path = config.agent.state_dir.join("state.json");
    let state = State::load(&state_path).await?;
    info!(path = %state_path.display(), sites = state.sites.len(), "loaded state");

    let dashboard_listener = match &config.dashboard.listen {
        Listen::Off => None,
        Listen::Addr(addr) => {
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

    let gateway_listener = match &config.gateway.listen {
        Listen::Off => None,
        Listen::Addr(addr) => {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("binding gateway listener on {addr}"))?;
            info!(%addr, hosts = ?config.gateway.hosts, upstream = %config.gateway.upstream, "gateway will listen");
            Some(listener)
        }
    };

    let site_event_kind = config.nostr.site_event_kind;
    let mut poll_timer = tokio::time::interval(config.agent.poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let notify = Arc::new(Notify::new());
    let dashboard_config = config.clone();
    let gateway_hosts = config.gateway.hosts.clone();
    let gateway_upstream = config.gateway.upstream.clone();
    let agent = Arc::new(Agent::new(
        config,
        ipfs,
        HttpNip05Verifier::public_only(),
        Arc::clone(&relay),
        state,
        state_path,
    ));
    // A select! arm body is not polled against the other arms, so a signal arriving during it would wait for the body to finish.
    if race_with_shutdown(agent.reconcile(), &shutdown).await {
        info!("shutdown requested during startup reconciliation");
        relay.client.shutdown().await;
        return Ok(exit.exit());
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
            Some(exit.clone()),
        )?);
        let (tx, rx) = tokio::sync::oneshot::channel();
        dashboard_shutdown = Some(tx);
        dashboard_task = Some(tokio::spawn(async move {
            if let Err(e) = dashboard::serve(listener, dashboard_state, rx).await {
                error!(error = %e, "dashboard server stopped");
            }
        }));
    }

    let gateway_token = shutdown.child_token();
    let mut gateway_task = None;
    if let Some(listener) = gateway_listener {
        let token = gateway_token.clone();
        gateway_task = Some(tokio::spawn(async move {
            if let Err(e) = gateway::serve(listener, gateway_hosts, gateway_upstream, token).await {
                error!(error = %e, "gateway server stopped");
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
                if race_with_shutdown(super::poll_once(&relay, &agent, &mut tasks), &shutdown).await {
                    info!("shutdown requested during poll");
                    shutdown_dashboard(&mut dashboard_shutdown, &mut dashboard_task).await;
                    shutdown_gateway(&gateway_token, &mut gateway_task).await;
                    relay.client.shutdown().await;
                    break;
                }
            }
            _ = notify.notified() => {
                if race_with_shutdown(super::poll_once(&relay, &agent, &mut tasks), &shutdown).await {
                    info!("shutdown requested during poll");
                    shutdown_dashboard(&mut dashboard_shutdown, &mut dashboard_task).await;
                    shutdown_gateway(&gateway_token, &mut gateway_task).await;
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
            joined = async { gateway_task.as_mut().unwrap().await }, if gateway_task.is_some() => {
                match joined {
                    Ok(()) => error!("gateway server task exited unexpectedly"),
                    Err(e) => error!(error = %e, "gateway server task panicked"),
                }
                gateway_task = None;
            }
            _ = shutdown.cancelled() => {
                info!("shutdown requested");
                shutdown_dashboard(&mut dashboard_shutdown, &mut dashboard_task).await;
                shutdown_gateway(&gateway_token, &mut gateway_task).await;
                relay.client.shutdown().await;
                break;
            }
        }
    }
    Ok(exit.exit())
}

async fn race_with_shutdown<F: std::future::Future<Output = ()>>(
    fut: F,
    shutdown: &CancellationToken,
) -> bool {
    tokio::select! {
        _ = fut => false,
        _ = shutdown.cancelled() => true,
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

async fn shutdown_gateway(
    gateway_token: &CancellationToken,
    gateway_task: &mut Option<tokio::task::JoinHandle<()>>,
) {
    gateway_token.cancel();
    if let Some(task) = gateway_task.take() {
        match tokio::time::timeout(GATEWAY_SHUTDOWN_TIMEOUT, task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error!(error = %e, "gateway server task panicked during shutdown"),
            Err(_) => warn!(
                timeout = ?GATEWAY_SHUTDOWN_TIMEOUT,
                "gateway server did not shut down in time; leaving it behind"
            ),
        }
    }
}
