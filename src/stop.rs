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

pub async fn run(config: &Config, restart: bool, timeout: Duration) -> Result<()> {
    let client = ApiClient::new(config.dashboard.listen);

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
        if let Err(ApiClientError::Unreachable(_)) =
            client.get::<serde_json::Value>("/api/overview").await
        {
            println!("stopped");
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("swing did not stop within {}s", timeout.as_secs());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
