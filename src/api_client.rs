use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::{Client, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::auth;
use crate::config::Config;
use crate::dashboard::dto::{IdentityDto, IdentityRequestDto};

// Above dashboard's own REQUEST_TIMEOUT so a slow call times out server-side, not client-side.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(125);

#[derive(Debug)]
pub enum ApiClientError {
    Unreachable(SocketAddr),
    NotSwing(SocketAddr),
    Http { status: StatusCode, message: String },
    Other(anyhow::Error),
}

const MAX_ERROR_CHARS: usize = 500;
const MAX_UNVERIFIED_BODY_BYTES: usize = 64 * 1024;

impl fmt::Display for ApiClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(addr) => {
                write!(f, "swing up is not running (cannot connect to {addr})")
            }
            Self::Http { message, .. } => f.write_str(&crate::format::sanitize_display_text(
                message,
                MAX_ERROR_CHARS,
            )),
            Self::NotSwing(addr) => write!(
                f,
                "the server at {addr} did not prove it knows this swing's dashboard token; not sending the token (another program may be using the dashboard port, or the token was rotated)"
            ),
            Self::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for ApiClientError {}

#[derive(Debug, serde::Deserialize)]
struct ErrorBody {
    error: String,
}

async fn read_capped(mut resp: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
    if resp.content_length().is_some_and(|len| len > limit as u64) {
        return None;
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if buf.len() + chunk.len() > limit {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    Some(buf)
}

async fn read_small_json<T: DeserializeOwned>(resp: reqwest::Response) -> Option<T> {
    let bytes = read_capped(resp, MAX_UNVERIFIED_BODY_BYTES).await?;
    serde_json::from_slice(&bytes).ok()
}

// An unspecified bind address (0.0.0.0/::) isn't connectable; loopback also satisfies the dashboard's Host guard.
fn loopback_addr(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V4(v4) if v4.ip().is_unspecified() => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), v4.port())
        }
        SocketAddr::V6(v6) if v6.ip().is_unspecified() => {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), v6.port())
        }
        other => other,
    }
}

pub struct ApiClient {
    client: Client,
    addr: SocketAddr,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(listen: SocketAddr, token: Option<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .no_proxy()
                .build()
                .expect("building the dashboard API client"),
            addr: loopback_addr(listen),
            token,
        }
    }

    pub fn for_config(config: &Config) -> anyhow::Result<Self> {
        let token = auth::read_token(&config.agent.state_dir)?;
        Ok(Self::new(config.dashboard.listen, token))
    }

    pub fn matches(&self, listen: SocketAddr, token: Option<&str>) -> bool {
        self.addr == loopback_addr(listen) && self.token.as_deref() == token
    }

    // Anything can listen on the dashboard port while swing is down, so every token-bearing request is preceded by a fresh proof.
    pub async fn identity(&self) -> Result<String, ApiClientError> {
        let nonce = auth::new_identity_nonce();
        let result = self
            .client
            .post(self.url("/api/identity"))
            .header("X-Swing-Dashboard", "1")
            .json(&IdentityRequestDto {
                nonce: nonce.clone(),
            })
            .send()
            .await;
        let resp = self.check_status(result).await?;
        let body = read_small_json::<IdentityDto>(resp)
            .await
            .ok_or(ApiClientError::NotSwing(self.addr))?;
        match &self.token {
            Some(token) if !auth::verify_identity_proof(token, &nonce, &body.proof) => {
                Err(ApiClientError::NotSwing(self.addr))
            }
            _ => Ok(body.instance),
        }
    }

    async fn send(&self, builder: RequestBuilder) -> Result<reqwest::Response, ApiClientError> {
        let builder = builder.header("X-Swing-Dashboard", "1");
        let builder = match &self.token {
            Some(token) => {
                self.identity().await?;
                builder.bearer_auth(token)
            }
            None => builder,
        };
        self.check_status(builder.send().await).await
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn check_status(
        &self,
        result: reqwest::Result<reqwest::Response>,
    ) -> Result<reqwest::Response, ApiClientError> {
        let resp = match result {
            Ok(resp) => resp,
            Err(e) if e.is_connect() => return Err(ApiClientError::Unreachable(self.addr)),
            Err(e) => {
                return Err(ApiClientError::Other(
                    anyhow::Error::new(e)
                        .context(format!("calling the dashboard at {}", self.addr)),
                ));
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let message = read_small_json::<ErrorBody>(resp)
                .await
                .map_or_else(|| status.to_string(), |b| b.error);
            return Err(ApiClientError::Http { status, message });
        }
        Ok(resp)
    }

    async fn finish<T: DeserializeOwned>(
        &self,
        builder: RequestBuilder,
    ) -> Result<T, ApiClientError> {
        self.send(builder)
            .await?
            .json::<T>()
            .await
            .map_err(|e| ApiClientError::Other(anyhow::Error::new(e).context("decoding response")))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiClientError> {
        self.finish(self.client.get(self.url(path))).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiClientError> {
        self.finish(self.client.post(self.url(path))).await
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiClientError> {
        self.finish(self.client.post(self.url(path)).json(body))
            .await
    }
}

#[cfg(test)]
mod tests;
