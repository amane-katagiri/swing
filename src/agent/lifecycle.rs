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
use crate::nip05::HttpNip05Verifier;
use crate::nostr::{self, RelayClient};
use crate::state::State;

use super::Agent;

const GATEWAY_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub async fn run_until(
    config: Config,
    shutdown: CancellationToken,
    dashboard: Arc<dashboard::AppState>,
    notify: Arc<Notify>,
) -> Result<()> {
    let signer = dashboard
        .signer
        .clone()
        .context("the agent started without a Nostr key")?;
    let relay = Arc::new(RelayClient::connect(signer, &config.nostr.relays).await?);
    info!(relays = ?relay.relays(), "connected to relays");

    let ipfs = config.ipfs_client().await?;
    let dashboard_ipfs = ipfs.clone();
    let state_path = config.agent.state_dir.join("state.json");
    let state = State::load(&state_path).await?;
    info!(path = %state_path.display(), sites = state.sites.len(), "loaded state");

    let gateway_listener = bind_gateway(&config).await?;

    let site_event_kind = config.nostr.site_event_kind;
    let mut poll_timer = tokio::time::interval(config.agent.poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let gateway_hosts = config.gateway.hosts.clone();
    let gateway_upstream = config.gateway.upstream.clone();
    let agent = Arc::new(Agent::new(
        config,
        ipfs,
        HttpNip05Verifier::public_only(),
        Arc::clone(&relay),
        state,
        state_path,
        Arc::clone(&dashboard.activity),
    ));
    // A select! arm body is not polled against the other arms, so a signal arriving during it would wait for the body to finish.
    if race_with_shutdown(agent.reconcile(), &shutdown).await {
        info!("shutdown requested during startup reconciliation");
        relay.client.shutdown().await;
        return Ok(());
    }

    dashboard
        .set_ready(Arc::clone(&relay), dashboard_ipfs)
        .await;

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

    let result = 'outer: loop {
        tokio::select! {
            maybe_note = notifications.next() => {
                match maybe_note {
                    Some(ClientNotification::Event { event, subscription_id, .. }) => {
                        if let Some(ev) = site_event_of(&event, &subscription_id, site_event_kind) {
                            agent.submit(ev, &mut tasks);
                        }
                    }
                    Some(ClientNotification::Shutdown) | None => {
                        break 'outer Err(anyhow::anyhow!("relay notification stream ended"));
                    }
                    Some(_) => {}
                }
            }
            _ = poll_timer.tick() => {
                if race_with_shutdown(super::poll_once(&relay, &agent, &mut tasks), &shutdown).await {
                    info!("shutdown requested during poll");
                    break 'outer Ok(());
                }
            }
            _ = notify.notified() => {
                if race_with_shutdown(super::poll_once(&relay, &agent, &mut tasks), &shutdown).await {
                    info!("shutdown requested during poll");
                    break 'outer Ok(());
                }
            }
            Some(joined) = tasks.join_next() => {
                if let Err(e) = joined {
                    error!(error = %e, "site task panicked");
                }
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
                break 'outer Ok(());
            }
        }
    };
    shutdown_gateway(&gateway_token, &mut gateway_task).await;
    relay.client.shutdown().await;
    dashboard.set_not_ready().await;
    result
}

async fn bind_gateway(config: &Config) -> Result<Option<tokio::net::TcpListener>> {
    let Listen::Addr(addr) = &config.gateway.listen else {
        return Ok(None);
    };
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding gateway listener on {addr}"))?;
    info!(%addr, hosts = ?config.gateway.hosts, upstream = %config.gateway.upstream, "gateway will listen");
    Ok(Some(listener))
}

fn site_event_of(
    event: &Event,
    subscription_id: &SubscriptionId,
    site_event_kind: u16,
) -> Option<nostr::SiteEvent> {
    if subscription_id.as_str() != nostr::SITE_SUBSCRIPTION_ID
        || event.kind != Kind::Custom(site_event_kind)
    {
        debug!(
            kind = %event.kind,
            subscription_id = %subscription_id,
            "ignoring notification outside the site subscription"
        );
        return None;
    }
    nostr::parse_site_event(event, site_event_kind)
        .inspect_err(|e| warn!(error = %e, "skipping invalid site event"))
        .ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CID_A, keys};

    #[test]
    fn only_site_events_on_the_site_subscription_are_taken() {
        let k = keys();
        let event = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("a.example"))
            .tag(Tag::custom("cid", [CID_A.to_string()]))
            .finalize(&k)
            .unwrap();
        let id = SubscriptionId::new(nostr::SITE_SUBSCRIPTION_ID);
        assert!(site_event_of(&event, &id, 35980).is_some());
        assert!(site_event_of(&event, &id, 35981).is_none());
        assert!(site_event_of(&event, &SubscriptionId::new("other"), 35980).is_none());
    }
}
