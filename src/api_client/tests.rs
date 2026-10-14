use super::*;
use crate::test_support::{identity_route, serve_router};
use axum::Json;
use axum::routing::get;
use serde::{Deserialize, Serialize};

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
    let addr = crate::test_support::unused_local_addr();

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
    let router = axum::Router::new().route("/ping", get(ready));
    let port = serve_router(router).await.port();

    let unspecified = SocketAddr::from(([0, 0, 0, 0], port));
    let client = ApiClient::new(unspecified, None);
    assert_eq!(client.addr(), SocketAddr::from(([127, 0, 0, 1], port)));
    let pong = client.get::<Pong>("/ping").await.unwrap();
    assert_eq!(pong, Pong { pong: true });
}
