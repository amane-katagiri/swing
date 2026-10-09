use std::time::Duration;

use tokio::time::Instant;

use anyhow::{Result, anyhow, bail};

use crate::api_client::{ApiClient, ApiClientError};
use crate::config::Config;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

async fn post_dashboard_action(client: &ApiClient, restart: bool) -> Result<(), ApiClientError> {
    let action = if restart { "restart" } else { "shutdown" };
    client
        .post::<serde_json::Value>(&format!("/api/{action}"))
        .await?;
    Ok(())
}

pub async fn run(config: &Config, restart: bool, timeout: Duration) -> Result<()> {
    let client = ApiClient::for_config(config)?;
    wait(&client, restart, timeout, POLL_INTERVAL).await
}

// Polling uses /api/identity, which never carries the token, so whoever takes the port after shutdown learns nothing.
async fn wait(
    client: &ApiClient,
    restart: bool,
    timeout: Duration,
    interval: Duration,
) -> Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow!("--timeout is too large"))?;
    let expired = || {
        if restart {
            anyhow!("swing did not come back within {}s", timeout.as_secs())
        } else {
            anyhow!("swing did not stop within {}s", timeout.as_secs())
        }
    };

    let before = match tokio::time::timeout_at(deadline, client.identity())
        .await
        .map_err(|_| anyhow!("swing did not answer within {}s", timeout.as_secs()))?
    {
        Ok(instance) => instance,
        Err(ApiClientError::Unreachable(_)) => {
            println!("not running");
            return Ok(());
        }
        Err(other) => bail!("{other}"),
    };

    match tokio::time::timeout_at(deadline, post_dashboard_action(client, restart))
        .await
        .map_err(|_| expired())?
    {
        Ok(()) => {}
        Err(ApiClientError::Unreachable(_)) => {
            println!("not running");
            return Ok(());
        }
        Err(other) => bail!("{other}"),
    }

    loop {
        let polled = tokio::time::timeout_at(deadline, client.identity()).await;
        match polled {
            Ok(Err(ApiClientError::Unreachable(_))) if !restart => {
                println!("stopped");
                return Ok(());
            }
            Ok(Ok(instance)) if restart && instance != before => {
                println!("restarted");
                return Ok(());
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(expired());
        }
        tokio::time::sleep_until((Instant::now() + interval).min(deadline)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{identity_route, serve_router};
    use axum::Json;
    use axum::http::HeaderMap;
    use axum::routing::{any, post};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const TOKEN: &str = "tok";
    const FAST: Duration = Duration::from_millis(20);

    #[tokio::test]
    async fn stop_never_sends_the_token_to_whoever_answers_after_shutdown() {
        let squatting = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let (flag, answer, record) = (
            Arc::clone(&squatting),
            Arc::clone(&squatting),
            Arc::clone(&seen),
        );
        let router = axum::Router::new()
            .route(
                "/api/identity",
                identity_route(move || {
                    let token = if answer.load(Ordering::SeqCst) {
                        "other"
                    } else {
                        TOKEN
                    };
                    (token, "inst".to_string())
                }),
            )
            .route(
                "/api/shutdown",
                post(move || {
                    let flag = Arc::clone(&flag);
                    async move {
                        flag.store(true, Ordering::SeqCst);
                        Json(serde_json::json!({ "ok": true }))
                    }
                }),
            )
            .fallback(any(move |headers: HeaderMap| {
                let record = Arc::clone(&record);
                async move {
                    if let Some(v) = headers.get("authorization") {
                        record.lock().unwrap().push(v.to_str().unwrap().to_string());
                    }
                    Json(serde_json::json!({}))
                }
            }));
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, Some(TOKEN.to_string()));
        let err = wait(&client, false, Duration::from_millis(300), FAST)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did not stop"), "{err}");
        assert!(
            client
                .get::<serde_json::Value>("/api/overview")
                .await
                .is_err()
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "{:?}",
            seen.lock().unwrap()
        );
    }

    #[tokio::test]
    async fn restart_waits_for_a_new_instance() {
        let instance = Arc::new(Mutex::new("old".to_string()));
        let flip = Arc::clone(&instance);
        let router = axum::Router::new()
            .route(
                "/api/identity",
                identity_route(move || (TOKEN, instance.lock().unwrap().clone())),
            )
            .route(
                "/api/restart",
                post(move || {
                    let flip = Arc::clone(&flip);
                    async move {
                        *flip.lock().unwrap() = "new".to_string();
                        Json(serde_json::json!({ "ok": true }))
                    }
                }),
            );
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, Some(TOKEN.to_string()));
        wait(&client, true, Duration::from_secs(5), FAST)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unresponsive_server_does_not_outlast_the_timeout() {
        let router = axum::Router::new().route(
            "/api/identity",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Json(serde_json::json!({}))
            }),
        );
        let addr = serve_router(router).await;
        let client = ApiClient::new(addr, Some(TOKEN.to_string()));
        let started = std::time::Instant::now();
        let err = wait(&client, false, Duration::from_millis(300), FAST)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did not answer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn huge_timeout_is_an_error_not_a_panic() {
        let client = ApiClient::new("127.0.0.1:1".parse().unwrap(), None);
        let err = wait(&client, false, Duration::MAX, FAST).await.unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[tokio::test]
    async fn stop_against_nothing_is_not_running() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let client = ApiClient::new(addr, Some(TOKEN.to_string()));
        // Windows retries a refused connection for about 2s before reporting it.
        wait(&client, false, Duration::from_secs(10), FAST)
            .await
            .unwrap();
    }
}
