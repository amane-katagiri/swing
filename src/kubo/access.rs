use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ipfs::IpfsClient;

use super::remove_if_present;

const API_ACCESS_FILE: &str = "kubo-api.json";
const API_SECRET_BYTES: usize = 32;

#[derive(Clone, PartialEq, Eq)]
pub struct ApiSecret(pub(super) String);

impl ApiSecret {
    pub fn generate() -> Self {
        Self(crate::auth::random_hex(API_SECRET_BYTES))
    }

    pub(super) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(text.to_string()))
    }

    pub fn authorization(&self) -> reqwest::header::HeaderValue {
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.0))
            .expect("a hex secret is a valid header value");
        value.set_sensitive(true);
        value
    }

    pub(super) fn kubo_authorizations(&self) -> serde_json::Value {
        json!({
            "swing": {
                "AuthSecret": format!("bearer:{}", self.0),
                "AllowedPaths": ["/api/v0"],
            }
        })
    }
}

impl std::fmt::Debug for ApiSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiAccess {
    pub port: u16,
    pub secret: ApiSecret,
}

#[derive(Serialize, Deserialize)]
struct ApiAccessFile {
    port: u16,
    secret: String,
}

impl ApiAccess {
    pub fn generate(port: u16) -> Self {
        Self {
            port,
            secret: ApiSecret::generate(),
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub(super) fn multiaddr(&self) -> String {
        format!("/ip4/127.0.0.1/tcp/{}", self.port)
    }

    pub fn client(&self) -> IpfsClient {
        IpfsClient::with_secret(self.url(), Some(&self.secret))
    }
}

pub(super) fn api_access_path(state_dir: &Path) -> PathBuf {
    state_dir.join(API_ACCESS_FILE)
}

pub fn write_api_access(state_dir: &Path, access: &ApiAccess) -> Result<()> {
    crate::auth::create_private_dir_all(state_dir)?;
    let file = ApiAccessFile {
        port: access.port,
        secret: access.secret.0.clone(),
    };
    let text = serde_json::to_string(&file).context("serializing the Kubo API access")?;
    crate::auth::write_private_file(&api_access_path(state_dir), &format!("{text}\n"))
}

pub(super) fn read_api_access(state_dir: &Path) -> Result<Option<ApiAccess>> {
    let path = api_access_path(state_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let file: ApiAccessFile =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let secret = ApiSecret::parse(&file.secret)
        .with_context(|| format!("{} does not hold a hex secret", path.display()))?;
    Ok(Some(ApiAccess {
        port: file.port,
        secret,
    }))
}

pub fn remove_api_access(state_dir: &Path) -> Result<()> {
    remove_if_present(&api_access_path(state_dir))
}

pub async fn managed_client(state_dir: &Path, repo: &Path) -> Result<IpfsClient> {
    let Some(access) = read_api_access(state_dir)? else {
        bail!(
            "Kubo is not running: {} does not exist (start `swing up`, or set [kubo].managed = false and [ipfs].api to use an external Kubo)",
            api_access_path(state_dir).display()
        );
    };
    let client = access.client();
    ensure_own_daemon(&client, repo).await?;
    Ok(client)
}

pub async fn ensure_own_daemon(ipfs: &IpfsClient, repo: &Path) -> Result<()> {
    let expected = read_peer_id(repo)?;
    let answered = ipfs
        .peer_id()
        .await
        .context("asking the Kubo API for its peer ID")?;
    if answered != expected {
        bail!(
            "the Kubo API at {} answered with peer ID {answered}, not this repo's {expected}; it is not the Kubo `swing up` started (is `swing up` running?)",
            ipfs.api_url()
        );
    }
    Ok(())
}

pub(super) fn read_peer_id(repo: &Path) -> Result<String> {
    let path = repo.join("config");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let config: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    config["Identity"]["PeerID"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .with_context(|| format!("{} has no Identity.PeerID", path.display()))
}
