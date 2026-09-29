use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::host::split_host_port;

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

const UPSTREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct GatewayState {
    hosts: Vec<String>,
    upstream: String,
    client: reqwest::Client,
    header_timeout: Duration,
}

fn connection_header_names(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn remove_headers_matching(headers: &mut HeaderMap, mut drop: impl FnMut(&str) -> bool) {
    let to_remove: Vec<HeaderName> = headers
        .keys()
        .filter(|name| drop(name.as_str()))
        .cloned()
        .collect();
    for name in to_remove {
        headers.remove(name);
    }
}

fn strip_hop_by_hop(headers: &mut HeaderMap, extra: &[String]) {
    remove_headers_matching(headers, |name| {
        HOP_BY_HOP.contains(&name) || extra.iter().any(|e| e == name)
    });
}

pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("building gateway http client")
}

pub fn router(hosts: Vec<String>, upstream: String, client: reqwest::Client) -> Router {
    router_with_header_timeout(hosts, upstream, client, UPSTREAM_HEADER_TIMEOUT)
}

fn router_with_header_timeout(
    hosts: Vec<String>,
    upstream: String,
    client: reqwest::Client,
    header_timeout: Duration,
) -> Router {
    let state = GatewayState {
        hosts,
        upstream,
        client,
        header_timeout,
    };
    Router::new().fallback(proxy).with_state(state)
}

async fn proxy(
    State(state): State<GatewayState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let Some(host_header) = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let (host, port) = split_host_port(&host_header);
    let Some(configured) = state.hosts.iter().find(|h| h.eq_ignore_ascii_case(host)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let forwarded_host = match port {
        Some(port) => format!("{configured}:{port}"),
        None => configured.clone(),
    };
    let Ok(forwarded_host) = HeaderValue::from_str(&forwarded_host) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{}{path_and_query}", state.upstream);
    let method = req.method().clone();

    let extra_drop = connection_header_names(req.headers());
    let mut out_headers = req.headers().clone();
    strip_hop_by_hop(&mut out_headers, &extra_drop);
    remove_headers_matching(&mut out_headers, |name| {
        name == "forwarded" || name.starts_with("x-forwarded-")
    });
    out_headers.insert(header::HOST, forwarded_host.clone());
    out_headers.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_str(&peer.ip().to_string()).unwrap(),
    );
    out_headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("http"),
    );
    out_headers.insert(HeaderName::from_static("x-forwarded-host"), forwarded_host);

    let body = reqwest::Body::wrap_stream(req.into_body().into_data_stream());

    let send = state
        .client
        .request(method, &url)
        .headers(out_headers)
        .body(body)
        .send();
    let Ok(upstream_response) = tokio::time::timeout(state.header_timeout, send).await else {
        warn!(
            timeout_secs = state.header_timeout.as_secs_f64(),
            url, "gateway upstream did not send response headers in time"
        );
        return StatusCode::GATEWAY_TIMEOUT.into_response();
    };

    match upstream_response {
        Ok(resp) => {
            let status = resp.status();
            let extra_drop = connection_header_names(resp.headers());
            let mut headers = resp.headers().clone();
            strip_hop_by_hop(&mut headers, &extra_drop);
            let mut response = Response::new(Body::from_stream(resp.bytes_stream()));
            *response.status_mut() = status;
            *response.headers_mut() = headers;
            response
        }
        Err(err) => {
            warn!(error = %err, url, "gateway upstream request failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

pub async fn serve(
    listener: TcpListener,
    hosts: Vec<String>,
    upstream: String,
    shutdown: CancellationToken,
) -> Result<()> {
    let addr = listener
        .local_addr()
        .context("reading gateway listener address")?;
    info!(%addr, "gateway listening");
    let app = router(hosts, upstream, client()?);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.cancelled_owned())
    .await
    .context("gateway server error")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::routing::get;
    use serde_json::{Value, json};

    async fn echo_upstream(req: Request) -> Response {
        let headers = req.headers().clone();
        let method = req.method().to_string();
        let path = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_default();
        let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .unwrap_or_default();
        Json(json!({
            "host": headers.get(header::HOST).and_then(|v| v.to_str().ok()),
            "x_forwarded_host": headers.get("x-forwarded-host").and_then(|v| v.to_str().ok()),
            "x_forwarded_for": headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
            "method": method,
            "path": path,
            "body": String::from_utf8_lossy(&body_bytes),
        }))
        .into_response()
    }

    async fn start_upstream() -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().fallback(get(echo_upstream).post(echo_upstream));
        let handle = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    async fn start_gateway(
        hosts: Vec<String>,
        upstream: String,
    ) -> (String, CancellationToken, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let token = CancellationToken::new();
        let child = token.clone();
        let handle = tokio::spawn(async move {
            serve(listener, hosts, upstream, child).await.unwrap();
        });
        tokio::task::yield_now().await;
        (format!("http://{addr}"), token, handle)
    }

    fn dead_upstream_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn allowed_host_is_proxied_with_forwarded_headers() {
        let (upstream, _upstream_handle) = start_upstream().await;
        let (gateway, token, gateway_handle) =
            start_gateway(vec!["example.com".to_string()], upstream).await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{gateway}/some/path?q=1"))
            .header("host", "example.com")
            .header("x-forwarded-for", "9.9.9.9")
            .body("hello body")
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let bytes = resp.bytes().await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["host"], "example.com");
        assert_eq!(body["x_forwarded_host"], "example.com");
        assert_eq!(body["x_forwarded_for"], "127.0.0.1");
        assert_eq!(body["method"], "POST");
        assert_eq!(body["path"], "/some/path?q=1");
        assert_eq!(body["body"], "hello body");

        token.cancel();
        gateway_handle.await.unwrap();
    }

    #[tokio::test]
    async fn disallowed_host_is_not_found() {
        let (upstream, _upstream_handle) = start_upstream().await;
        let (gateway, token, gateway_handle) =
            start_gateway(vec!["example.com".to_string()], upstream).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{gateway}/"))
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
        assert_eq!(resp.bytes().await.unwrap().len(), 0);

        token.cancel();
        gateway_handle.await.unwrap();
    }

    #[tokio::test]
    async fn missing_host_is_not_found() {
        let (upstream, _upstream_handle) = start_upstream().await;
        let (gateway, token, gateway_handle) =
            start_gateway(vec!["example.com".to_string()], upstream).await;

        let raw = tokio::net::TcpStream::connect(gateway.trim_start_matches("http://"))
            .await
            .unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut raw = raw;
        raw.write_all(b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        raw.read_to_end(&mut buf).await.ok();
        let text = String::from_utf8_lossy(&buf);
        assert!(text.starts_with("HTTP/1.1 404"), "response: {text}");

        token.cancel();
        gateway_handle.await.unwrap();
    }

    #[tokio::test]
    async fn host_matching_is_case_and_port_insensitive() {
        let (upstream, _upstream_handle) = start_upstream().await;
        let (gateway, token, gateway_handle) =
            start_gateway(vec!["example.com".to_string()], upstream).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{gateway}/"))
            .header("host", "Example.com:8081")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        token.cancel();
        gateway_handle.await.unwrap();
    }

    #[tokio::test]
    async fn the_configured_host_is_forwarded_instead_of_the_client_header() {
        let (upstream, _upstream_handle) = start_upstream().await;
        let (gateway, token, gateway_handle) =
            start_gateway(vec!["example.com".to_string()], upstream).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{gateway}/"))
            .header("host", "EXAMPLE.com:8081")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let body: Value = serde_json::from_slice(&resp.bytes().await.unwrap()).unwrap();
        assert_eq!(body["host"], "example.com:8081");
        assert_eq!(body["x_forwarded_host"], "example.com:8081");

        for junk in ["[example.com]junk", "[example.com]:x", "example.com:"] {
            let resp = client
                .get(format!("{gateway}/"))
                .header("host", junk)
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND, "{junk}");
        }

        token.cancel();
        gateway_handle.await.unwrap();
    }

    #[tokio::test]
    async fn upstream_that_never_sends_headers_is_gateway_timeout() {
        let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = format!("http://{}", silent.local_addr().unwrap());
        let _silent_handle = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = silent.accept().await {
                held.push(stream);
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway = format!("http://{}", listener.local_addr().unwrap());
        let app = router_with_header_timeout(
            vec!["example.com".to_string()],
            upstream,
            client().unwrap(),
            Duration::from_millis(200),
        );
        let _gateway_handle = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let resp = reqwest::Client::new()
            .get(format!("{gateway}/"))
            .header("host", "example.com")
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(resp.bytes().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn upstream_connection_failure_is_bad_gateway() {
        let (gateway, token, gateway_handle) =
            start_gateway(vec!["example.com".to_string()], dead_upstream_url()).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{gateway}/"))
            .header("host", "example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
        assert_eq!(resp.bytes().await.unwrap().len(), 0);

        token.cancel();
        gateway_handle.await.unwrap();
    }
}
