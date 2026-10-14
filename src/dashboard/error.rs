use axum::Json;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use tracing::error;

use crate::settings;

pub enum ApiError {
    BadRequest(String),
    Unauthorized(String),
    PayloadTooLarge(String),
    NotReady,
    NotConfigured,
    Busy,
    Upstream(String),
    Conflict(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            ApiError::PayloadTooLarge(msg) => (StatusCode::PAYLOAD_TOO_LARGE, msg),
            ApiError::NotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent is not ready".to_string(),
            ),
            ApiError::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent is not configured".to_string(),
            ),
            ApiError::Busy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "too many relay queries are running; try again later".to_string(),
            ),
            ApiError::Upstream(msg) => (StatusCode::BAD_GATEWAY, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::Internal(detail) => {
                error!(error = %detail, "dashboard request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error; see the swing log for details".to_string(),
                )
            }
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

pub(super) fn upstream(e: anyhow::Error) -> ApiError {
    ApiError::Upstream(format!("{e:#}"))
}

// axum maps deserialize errors to 422, which this API reserves for the publish NIP-05 require failure.
pub struct AppJson<T>(pub T);

impl<T, S> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(AppJson(value)),
            Err(rejection) => {
                let message = rejection.to_string();
                Err(if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::PayloadTooLarge(message)
                } else {
                    ApiError::BadRequest(message)
                })
            }
        }
    }
}

pub(super) fn internal(context: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(format!("{context}: {e:#}"))
}

pub(super) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| internal("a background task failed", e))?
}

pub(super) fn settings_error(e: settings::EditError) -> ApiError {
    match e {
        settings::EditError::Invalid(e) => ApiError::BadRequest(format!("{e:#}")),
        settings::EditError::Io(e) => internal("saving the config file failed", e),
    }
}

#[cfg(test)]
mod tests {
    use super::super::router;
    use super::super::test_support::*;
    use axum::body::Body;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn internal_errors_do_not_reach_the_client() {
        use axum::response::IntoResponse;
        let resp = super::internal(
            "saving the config file failed",
            "/home/someone/.config/swing/swing.toml: Permission denied",
        )
        .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(!body.contains("someone"), "{body}");
        assert!(!body.contains("Permission denied"), "{body}");
    }

    #[tokio::test]
    async fn unreadable_json_bodies_are_bad_request_with_a_json_error() {
        let key = nostr_sdk::prelude::Keys::generate().public_key().to_hex();
        let cases = [
            ("malformed", post_json("/api/mirror/add", "{not json"), ""),
            ("missing a field", post_json("/api/mirror/add", "{}"), ""),
            (
                "no content type",
                request("POST", "/api/mirror/add")
                    .header("x-swing-dashboard", "1")
                    .body(Body::from(serde_json::json!({ "keys": [key] }).to_string()))
                    .unwrap(),
                "Content-Type",
            ),
        ];
        for (case, req, error) in cases {
            let resp = call(router(test_state()), req).await;
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{case}");
            let body = json_body(resp).await;
            assert!(
                body["error"].as_str().unwrap().contains(error),
                "{case}: {body}"
            );
        }
    }
}
