use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::api_client::{ApiClient, ApiClientError};
use crate::config::Config;

async fn post_dashboard_action(client: &ApiClient, restart: bool) -> Result<(), ApiClientError> {
    let action = if restart { "restart" } else { "shutdown" };
    client
        .post::<serde_json::Value>(&format!("/api/{action}"))
        .await?;
    Ok(())
}

fn instance(overview: &serde_json::Value) -> Option<&str> {
    overview.get("instance").and_then(|v| v.as_str())
}

pub async fn run(config: &Config, restart: bool, timeout: Duration) -> Result<()> {
    let client = ApiClient::for_config(config)?;

    let before = if restart {
        match client.get::<serde_json::Value>("/api/overview").await {
            Ok(overview) => instance(&overview).map(str::to_owned),
            Err(ApiClientError::Unreachable(_)) => {
                println!("not running");
                return Ok(());
            }
            Err(other) => bail!("{other}"),
        }
    } else {
        None
    };

    if let Err(e) = post_dashboard_action(&client, restart).await {
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
        match client.get::<serde_json::Value>("/api/overview").await {
            Err(ApiClientError::Unreachable(_)) if !restart => {
                println!("stopped");
                return Ok(());
            }
            Ok(overview) if restart && instance(&overview) != before.as_deref() => {
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
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
