use super::*;
use crate::test_support::serve_router;
use axum::Json;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        "cookie": headers.get(header::COOKIE).and_then(|v| v.to_str().ok()),
        "authorization": headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()),
        "method": method,
        "path": path,
        "body": String::from_utf8_lossy(&body_bytes),
    }))
    .into_response()
}

struct Gateway {
    url: String,
    token: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}

impl Gateway {
    async fn start(upstream: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let token = CancellationToken::new();
        let child = token.clone();
        let hosts = vec!["example.com".to_string()];
        let handle = tokio::spawn(async move {
            serve(listener, hosts, upstream, child).await.unwrap();
        });
        tokio::task::yield_now().await;
        Self { url, token, handle }
    }

    async fn echo() -> Self {
        let upstream = serve_router(Router::new().fallback(echo_upstream)).await;
        Self::start(format!("http://{upstream}")).await
    }

    async fn stop(self) {
        self.token.cancel();
        self.handle.await.unwrap();
    }
}

async fn raw_request(gateway: &str, method: &str, path: &str) -> String {
    let mut raw = tokio::net::TcpStream::connect(gateway.trim_start_matches("http://"))
        .await
        .unwrap();
    raw.write_all(
        format!("{method} {path} HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let mut buf = Vec::new();
    raw.read_to_end(&mut buf).await.ok();
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn methods_other_than_get_and_head_are_not_allowed() {
    let gw = Gateway::echo().await;
    let gateway = &gw.url;

    let client = reqwest::Client::new();
    for method in ["POST", "PUT", "DELETE", "PATCH", "OPTIONS", "PROPFIND"] {
        let resp = client
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
                format!("{gateway}/index.html"),
            )
            .header("host", "example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
        assert_eq!(resp.headers()[header::ALLOW], "GET, HEAD", "{method}");
        assert_eq!(resp.bytes().await.unwrap().len(), 0, "{method}");
    }

    let resp = client
        .head(format!("{gateway}/index.html"))
        .header("host", "example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    gw.stop().await;
}

#[tokio::test]
async fn content_paths_are_not_found_however_spelled() {
    let gw = Gateway::echo().await;
    let gateway = &gw.url;

    for path in [
        "/ipfs",
        "/ipns",
        "/ipfs/",
        "/ipfs/bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
        "/ipns/example.org/index.html",
        "/ipfs?x=1",
        "/IPFS/bafy",
        "//ipfs/bafy",
        "/./ipns/k51",
        "/a/../ipfs/bafy",
        "/%69pfs/bafy",
        "/%49%50%4E%53/k51",
        "/ipfs%2Fbafy",
        "/%2e%2e/ipfs/bafy",
        "/%2569pfs/bafy",
        "/\\ipfs\\bafy",
    ] {
        for method in ["GET", "HEAD"] {
            let text = raw_request(gateway, method, path).await;
            assert!(text.starts_with("HTTP/1.1 404"), "{method} {path}: {text}");
        }
    }

    gw.stop().await;
}

#[tokio::test]
async fn site_paths_are_proxied() {
    let gw = Gateway::echo().await;
    let gateway = &gw.url;

    for path in [
        "/",
        "/index.html",
        "/ipfsx/a",
        "/ipfs-notes/",
        "/blog/ipfs/bafy",
        "/a/ipns",
        "/?ipfs=/ipfs/bafy",
        "/%41bout.html",
    ] {
        let text = raw_request(gateway, "GET", path).await;
        assert!(text.starts_with("HTTP/1.1 200"), "{path}: {text}");
        assert!(
            text.contains(&format!("\"path\":\"{path}\"")),
            "{path}: {text}"
        );
    }

    gw.stop().await;
}

#[tokio::test]
async fn allowed_host_is_proxied_with_forwarded_headers() {
    let gw = Gateway::echo().await;
    let gateway = &gw.url;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{gateway}/some/path?q=1"))
        .header("host", "example.com")
        .header("x-forwarded-for", "9.9.9.9")
        .header("cookie", "session=secret")
        .header("authorization", "Bearer secret")
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
    assert!(body["cookie"].is_null());
    assert!(body["authorization"].is_null());
    assert_eq!(body["method"], "GET");
    assert_eq!(body["path"], "/some/path?q=1");
    assert_eq!(body["body"], "hello body");

    gw.stop().await;
}

#[tokio::test]
async fn a_disallowed_or_missing_host_is_not_found() {
    let gw = Gateway::echo().await;
    let gateway = &gw.url;

    let resp = reqwest::Client::new()
        .get(format!("{gateway}/"))
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND, "disallowed");
    assert_eq!(resp.bytes().await.unwrap().len(), 0, "disallowed");

    let mut raw = tokio::net::TcpStream::connect(gateway.trim_start_matches("http://"))
        .await
        .unwrap();
    raw.write_all(b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    raw.read_to_end(&mut buf).await.ok();
    let text = String::from_utf8_lossy(&buf);
    assert!(text.starts_with("HTTP/1.1 404"), "missing: {text}");

    gw.stop().await;
}

#[tokio::test]
async fn the_configured_host_is_forwarded_instead_of_the_client_header() {
    let gw = Gateway::echo().await;
    let gateway = &gw.url;

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

    gw.stop().await;
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
    let dead = crate::test_support::unused_local_addr();
    let gw = Gateway::start(format!("http://{dead}")).await;
    let gateway = &gw.url;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{gateway}/"))
        .header("host", "example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    assert_eq!(resp.bytes().await.unwrap().len(), 0);

    gw.stop().await;
}
