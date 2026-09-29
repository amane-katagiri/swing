use std::time::{Duration, Instant};

use anyhow::{Result, bail};

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
    let before = match client.identity().await {
        Ok(instance) => instance,
        Err(ApiClientError::Unreachable(_)) => {
            println!("not running");
            return Ok(());
        }
        Err(other) => bail!("{other}"),
    };

    if let Err(e) = post_dashboard_action(client, restart).await {
        match e {
            ApiClientError::Unreachable(_) => {
                println!("not running");
                return Ok(());
            }
            other => bail!("{other}"),
        }
    }

    let deadline = Instant::now() + timeout;
    loop {
        match client.identity().await {
            Err(ApiClientError::Unreachable(_)) if !restart => {
                println!("stopped");
                return Ok(());
            }
            Ok(instance) if restart && instance != before => {
                println!("restarted");
                return Ok(());
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            if restart {
                bail!("swing did not come back within {}s", timeout.as_secs());
            }
            bail!("swing did not stop within {}s", timeout.as_secs());
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth;
    use crate::dashboard::dto::{IdentityDto, IdentityRequestDto};
    use axum::Json;
    use axum::http::HeaderMap;
    use axum::routing::{any, post};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const TOKEN: &str = "tok";
    const FAST: Duration = Duration::from_millis(20);

    fn identity_route(instance: Arc<Mutex<String>>) -> axum::routing::MethodRouter {
        post(move |Json(req): Json<IdentityRequestDto>| {
            let instance = Arc::clone(&instance);
            async move {
                Json(IdentityDto {
                    proof: auth::identity_proof(TOKEN, &req.nonce),
                    instance: instance.lock().unwrap().clone(),
                })
            }
        })
    }

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
                post(move |Json(req): Json<IdentityRequestDto>| {
                    let answer = Arc::clone(&answer);
                    async move {
                        let token = if answer.load(Ordering::SeqCst) {
                            "other"
                        } else {
                            TOKEN
                        };
                        Json(IdentityDto {
                            proof: auth::identity_proof(token, &req.nonce),
                            instance: "inst".to_string(),
                        })
                    }
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

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
            .route("/api/identity", identity_route(Arc::clone(&instance)))
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let client = ApiClient::new(addr, Some(TOKEN.to_string()));
        wait(&client, true, Duration::from_secs(5), FAST)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn stop_against_nothing_is_not_running() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let client = ApiClient::new(addr, Some(TOKEN.to_string()));
        wait(&client, false, Duration::from_secs(1), FAST)
            .await
            .unwrap();
    }
}
