use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

use crate::api_client::{ApiClient, ApiClientError};
use crate::auth;
use crate::config::Config;
use crate::dashboard::dto::LoginCodeDto;

fn browser_command(url: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new(crate::service::windows_system_tool("rundll32.exe"));
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

pub fn open_browser(url: &str) -> Result<()> {
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

pub struct LoginLink {
    pub url: String,
    pub code: String,
    pub expires_in: u64,
}

pub async fn request_link(config: &Config) -> Result<LoginLink> {
    if !config.dashboard.ui {
        bail!("the dashboard UI is disabled ([dashboard].ui = false)");
    }
    let client = ApiClient::for_config(config)?;
    let dto = client
        .post::<LoginCodeDto>("/api/login-code")
        .await
        .map_err(|e| anyhow!("{e}"))?;
    if !auth::is_login_code(&dto.code) {
        bail!(
            "the dashboard at {} returned a malformed login code",
            client.addr()
        );
    }
    let base = config
        .dashboard
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}", client.addr()));
    Ok(LoginLink {
        url: format!("{base}/login?code={}", dto.code),
        code: dto.code,
        expires_in: dto.expires_in,
    })
}

pub async fn open(config: &Config, no_browser: bool) -> Result<()> {
    let link = request_link(config).await?;
    println!("{}", link.url);
    println!(
        "login code (single use, valid for {} minutes): {}",
        link.expires_in / 60,
        link.code
    );
    if !no_browser && let Err(e) = open_browser(&link.url) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::serve_router;
    use axum::Json;

    async fn link_from(code: &'static str) -> Result<LoginLink> {
        let router = axum::Router::new().route(
            "/api/login-code",
            axum::routing::post(move || async move {
                Json(serde_json::json!({ "code": code, "expires_in": 300 }))
            }),
        );
        let addr = serve_router(router).await;
        let dir = tempfile::tempdir().unwrap();
        let toml = format!(
            "[nostr]\nrelays = [\"wss://relay.example\"]\n[agent]\nstate_dir = {:?}\n[dashboard]\nlisten = \"{addr}\"\n",
            dir.path().display().to_string()
        );
        let config = crate::config::build_config_from_str(&toml, |_| None).unwrap();
        request_link(&config).await
    }

    #[tokio::test]
    async fn malformed_login_codes_are_neither_printed_nor_opened() {
        let good = "0123456789abcdef0123456789abcdef";
        let link = link_from(good).await.unwrap();
        assert!(link.url.ends_with(&format!("/login?code={good}")));
        for bad in [
            "x&next=//evil",
            "0123456789ABCDEF0123456789ABCDEF",
            "\u{1b}]0;x\u{7}",
        ] {
            let Err(err) = link_from(bad).await else {
                panic!("accepted {bad:?}");
            };
            assert!(err.to_string().contains("malformed login code"), "{err}");
        }
    }
}
