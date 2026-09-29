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
mod tests {
    use super::*;
    use crate::test_support::{identity_route, serve_router};
    use axum::Json;
    use axum::routing::get;
    use serde::{Deserialize, Serialize};
    use tokio::net::TcpListener;

    #[test]
    fn http_error_display_strips_terminal_controls() {
        let err = ApiClientError::Http {
            status: StatusCode::BAD_REQUEST,
            message: "bad\x1b]0;owned\x07 \u{202e}input".to_string(),
        };
        let shown = err.to_string();
        assert!(
            !shown.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "{shown:?}"
        );
        assert!(
            shown.starts_with("bad") && shown.ends_with("input"),
            "{shown:?}"
        );
    }

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

    #[tokio::test]
    async fn get_against_a_stopped_server_is_unreachable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let client = ApiClient::new(addr, None);
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
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, None);
        let pong = client.get::<Pong>("/ping").await.unwrap();
        assert_eq!(pong, Pong { pong: true });
    }

    #[tokio::test]
    async fn get_503_extracts_the_error_message() {
        let router = axum::Router::new().route("/ping", get(not_ready));
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, None);
        let err = client.get::<Pong>("/ping").await.unwrap_err();
        match err {
            ApiClientError::Http { status, message } => {
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(message, "agent is not ready");
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    async fn echo_authorization(headers: axum::http::HeaderMap) -> String {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    #[tokio::test]
    async fn oversized_answers_are_not_read_whole() {
        let huge = "x".repeat(MAX_UNVERIFIED_BODY_BYTES + 1);
        let proof = huge.clone();
        let router = axum::Router::new()
            .route(
                "/api/identity",
                axum::routing::post(move || {
                    let proof = proof.clone();
                    async move {
                        Json(IdentityDto {
                            proof,
                            instance: "inst".to_string(),
                        })
                    }
                }),
            )
            .route(
                "/fail",
                get(move || {
                    let huge = huge.clone();
                    async move {
                        (
                            axum::http::StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": huge })),
                        )
                    }
                }),
            );
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, Some("tok".to_string()));
        assert!(matches!(
            client.identity().await,
            Err(ApiClientError::NotSwing(_))
        ));
        let anonymous = ApiClient::new(addr, None);
        match anonymous.get::<Pong>("/fail").await.unwrap_err() {
            ApiClientError::Http { status, message } => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert_eq!(message, status.to_string());
            }
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn token_is_sent_as_bearer_on_get_and_post() {
        let router = axum::Router::new()
            .route(
                "/auth",
                get(|h| async move { Json(echo_authorization(h).await) })
                    .post(|h| async move { Json(echo_authorization(h).await) }),
            )
            .route(
                "/api/identity",
                identity_route(|| ("tok", "inst".to_string())),
            );
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, Some("tok".to_string()));
        assert_eq!(client.get::<String>("/auth").await.unwrap(), "Bearer tok");
        assert_eq!(client.post::<String>("/auth").await.unwrap(), "Bearer tok");
        let anonymous = ApiClient::new(addr, None);
        assert_eq!(anonymous.get::<String>("/auth").await.unwrap(), "");
    }

    #[tokio::test]
    async fn token_is_withheld_from_a_server_that_cannot_prove_it() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let record = std::sync::Arc::clone(&seen);
        let router = axum::Router::new()
            .route(
                "/auth",
                get(move |h| {
                    let record = std::sync::Arc::clone(&record);
                    async move {
                        let value = echo_authorization(h).await;
                        record.lock().unwrap().push(value.clone());
                        Json(value)
                    }
                }),
            )
            .route(
                "/api/identity",
                identity_route(|| ("other", "inst".to_string())),
            );
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, Some("tok".to_string()));
        let err = client.get::<String>("/auth").await.unwrap_err();
        assert!(matches!(err, ApiClientError::NotSwing(a) if a == addr));
        assert!(seen.lock().unwrap().is_empty());

        let squatter = serve_router(axum::Router::new().route("/auth", get(ready))).await;
        let client = ApiClient::new(squatter, Some("tok".to_string()));
        assert!(client.get::<Pong>("/auth").await.is_err());
    }

    #[tokio::test]
    async fn every_token_bearing_request_is_preceded_by_a_fresh_proof() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let swapped = std::sync::Arc::new(AtomicBool::new(false));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let (answer, record) = (
            std::sync::Arc::clone(&swapped),
            std::sync::Arc::clone(&seen),
        );
        let router = axum::Router::new()
            .route(
                "/auth",
                get(move |h| {
                    let record = std::sync::Arc::clone(&record);
                    async move {
                        let value = echo_authorization(h).await;
                        record.lock().unwrap().push(value.clone());
                        Json(value)
                    }
                }),
            )
            .route(
                "/api/identity",
                identity_route(move || {
                    let token = if answer.load(Ordering::SeqCst) {
                        "other"
                    } else {
                        "tok"
                    };
                    (token, "inst".to_string())
                }),
            );
        let addr = serve_router(router).await;

        let client = ApiClient::new(addr, Some("tok".to_string()));
        assert_eq!(client.get::<String>("/auth").await.unwrap(), "Bearer tok");
        assert_eq!(client.identity().await.unwrap(), "inst");
        swapped.store(true, Ordering::SeqCst);
        let err = client.get::<String>("/auth").await.unwrap_err();
        assert!(matches!(err, ApiClientError::NotSwing(_)));
        assert!(matches!(
            client.identity().await,
            Err(ApiClientError::NotSwing(_))
        ));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn matches_compares_the_normalized_address_and_token() {
        let listen = SocketAddr::from(([0, 0, 0, 0], 8082));
        let client = ApiClient::new(listen, Some("tok".to_string()));
        assert!(client.matches(SocketAddr::from(([127, 0, 0, 1], 8082)), Some("tok")));
        assert!(!client.matches(listen, Some("new")));
        assert!(!client.matches(listen, None));
        assert!(!client.matches(SocketAddr::from(([127, 0, 0, 1], 8083)), Some("tok")));
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
        let client = ApiClient::new(unspecified, None);
        assert_eq!(client.addr(), SocketAddr::from(([127, 0, 0, 1], port)));
        let pong = client.get::<Pong>("/ping").await.unwrap();
        assert_eq!(pong, Pong { pong: true });
    }
}
