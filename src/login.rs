use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

use crate::api_client::{ApiClient, ApiClientError};
use crate::auth;
use crate::config::Config;
use crate::dashboard::dto::LoginCodeDto;

fn browser_command(url: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("rundll32");
        cmd.args(["url.dll,FileProtocolHandler", url]);
        cmd
    } else if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        cmd
    } else {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd
    }
}

fn open_browser(url: &str) -> Result<()> {
    let mut cmd = browser_command(url);
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} exited with {status}");
    }
    Ok(())
}

pub async fn open(config: &Config, no_browser: bool) -> Result<()> {
    if !config.dashboard.ui {
        bail!("the dashboard UI is disabled ([dashboard].ui = false)");
    }
    let client = ApiClient::for_config(config)?;
    let dto = client
        .post::<LoginCodeDto>("/api/login-code")
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let base = config
        .dashboard
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}", client.addr()));
    let url = format!("{base}/login?code={}", dto.code);
    println!("{url}");
    println!(
        "login code (single use, valid for {} minutes): {}",
        dto.expires_in / 60,
        dto.code
    );
    if !no_browser && let Err(e) = open_browser(&url) {
        eprintln!(
            "could not open a browser ({e:#}); open the URL above or enter the code on the login screen"
        );
    }
    Ok(())
}

pub async fn rotate_token(config: &Config) -> Result<()> {
    let client = ApiClient::for_config(config)?;
    match client.post::<serde_json::Value>("/api/token/rotate").await {
        Ok(_) => {
            println!("rotated the dashboard token; browser sessions have been logged out");
            Ok(())
        }
        Err(ApiClientError::Unreachable(_)) => {
            auth::write_new_token(&config.agent.state_dir)?;
            println!(
                "rotated the dashboard token in {}",
                auth::token_path(&config.agent.state_dir).display()
            );
            Ok(())
        }
        Err(e) => bail!("{e}"),
    }
}
