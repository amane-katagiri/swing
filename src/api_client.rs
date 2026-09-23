use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

// Above the dashboard's own REQUEST_TIMEOUT (120s, src/dashboard/mod.rs) so a
// slow server-side call surfaces as the server's own timeout response rather
// than a client-side cutoff.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(125);

#[derive(Debug)]
pub enum ApiClientError {
    Unreachable(SocketAddr),
    Http { status: StatusCode, message: String },
    Other(anyhow::Error),
}

impl fmt::Display for ApiClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(addr) => {
                write!(f, "swing up is not running (cannot connect to {addr})")
            }
            Self::Http { message, .. } => f.write_str(message),
            Self::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for ApiClientError {}

#[derive(Debug, serde::Deserialize)]
struct ErrorBody {
    error: String,
}

// The dashboard always binds a concrete address (docs/architecture/dashboard.md),
// but an unspecified one (0.0.0.0/::) is only meaningful as a bind address;
// connecting to it, and sending it as the Host header, has to target loopback
// instead to satisfy the dashboard's own Host guard (src/dashboard/guard.rs).
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
}

impl ApiClient {
    pub fn new(listen: SocketAddr) -> Self {
        Self {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("building the dashboard API client"),
            addr: loopback_addr(listen),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn finish<T: DeserializeOwned>(
        &self,
        result: reqwest::Result<reqwest::Response>,
    ) -> Result<T, ApiClientError> {
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
            let message = resp
                .json::<ErrorBody>()
                .await
                .map(|b| b.error)
                .unwrap_or_else(|_| status.to_string());
            return Err(ApiClientError::Http { status, message });
        }
        resp.json::<T>()
            .await
            .map_err(|e| ApiClientError::Other(anyhow::Error::new(e).context("decoding response")))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiClientError> {
        let result = self.client.get(self.url(path)).send().await;
        self.finish(result).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiClientError> {
        let result = self
            .client
            .post(self.url(path))
            .header("X-Swing-Dashboard", "1")
            .send()
            .await;
        self.finish(result).await
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiClientError> {
        let result = self
            .client
            .post(self.url(path))
            .header("X-Swing-Dashboard", "1")
            .json(body)
            .send()
            .await;
        self.finish(result).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::routing::get;
    use serde::{Deserialize, Serialize};
    use tokio::net::TcpListener;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Pong {
        pong: bool,
    }

    async fn ready() -> Json<Pong> {
        Json(Pong { pong: true })
    }

    async fn not_ready() -> axum::response::Response {
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "agent is not ready" })),
        )
            .into_response()
    }

    async fn spawn(router: axum::Router) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn get_against_a_stopped_server_is_unreachable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let client = ApiClient::new(addr);
        let err = client.get::<Pong>("/ping").await.unwrap_err();
        assert!(matches!(err, ApiClientError::Unreachable(a) if a == addr));
        assert_eq!(
            err.to_string(),
            format!("swing up is not running (cannot connect to {addr})")
        );
    }

    #[tokio::test]
    async fn get_success_decodes_the_body() {
        let router = axum::Router::new().route("/ping", get(ready));
        let addr = spawn(router).await;

        let client = ApiClient::new(addr);
        let pong = client.get::<Pong>("/ping").await.unwrap();
        assert_eq!(pong, Pong { pong: true });
    }

    #[tokio::test]
    async fn get_503_extracts_the_error_message() {
        let router = axum::Router::new().route("/ping", get(not_ready));
        let addr = spawn(router).await;

        let client = ApiClient::new(addr);
        let err = client.get::<Pong>("/ping").await.unwrap_err();
        match err {
            ApiClientError::Http { status, message } => {
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(message, "agent is not ready");
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unspecified_listen_address_connects_over_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let router = axum::Router::new().route("/ping", get(ready));
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let unspecified = SocketAddr::from(([0, 0, 0, 0], port));
        let client = ApiClient::new(unspecified);
        assert_eq!(client.addr(), SocketAddr::from(([127, 0, 0, 1], port)));
        let pong = client.get::<Pong>("/ping").await.unwrap();
        assert_eq!(pong, Pong { pong: true });
    }
}
