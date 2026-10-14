use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
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

fn percent_decode(input: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%'
            && let Some(hi) = input.get(i + 1).copied().and_then(hex)
            && let Some(lo) = input.get(i + 2).copied().and_then(hex)
        {
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    out
}

fn is_content_path(path: &str) -> bool {
    let mut decoded = path.as_bytes().to_vec();
    // Decoding until stable also covers a front proxy that decodes once more before us.
    loop {
        let next = percent_decode(&decoded);
        if next == decoded {
            break;
        }
        decoded = next;
    }
    let mut segments: Vec<&[u8]> = Vec::new();
    for segment in decoded.split(|b| matches!(b, b'/' | b'\\')) {
        match segment {
            b"" | b"." => {}
            b".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments
        .first()
        .is_some_and(|s| s.eq_ignore_ascii_case(b"ipfs") || s.eq_ignore_ascii_case(b"ipns"))
}

fn strip_hop_by_hop(headers: &mut HeaderMap, extra: &[String]) {
    remove_headers_matching(headers, |name| {
        HOP_BY_HOP.contains(&name) || extra.iter().any(|e| e == name)
    });
}

pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("building the gateway HTTP client")
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

    if req.method() != Method::GET && req.method() != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, HeaderValue::from_static("GET, HEAD"))],
        )
            .into_response();
    }
    if is_content_path(req.uri().path()) {
        return StatusCode::NOT_FOUND.into_response();
    }

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
        matches!(name, "forwarded" | "cookie" | "authorization") || name.starts_with("x-forwarded-")
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
mod tests;
