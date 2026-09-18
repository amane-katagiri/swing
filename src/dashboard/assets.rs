use std::sync::Arc;

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use super::AppState;

const INDEX_HTML: &str = include_str!("../../web/index.html");
const STYLE_CSS: &str = include_str!("../../web/style.css");
const APP_JS: &str = include_str!("../../web/app.js");
const GRAPH_JS: &str = include_str!("../../web/graph.js");

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

pub async fn index() -> Response {
    asset("text/html; charset=utf-8", INDEX_HTML)
}

pub async fn style() -> Response {
    asset("text/css; charset=utf-8", STYLE_CSS)
}

pub async fn app_js() -> Response {
    asset("text/javascript; charset=utf-8", APP_JS)
}

pub async fn graph_js() -> Response {
    asset("text/javascript; charset=utf-8", GRAPH_JS)
}

pub async fn custom_css(State(state): State<Arc<AppState>>) -> Response {
    let body = match &state.config.dashboard.custom_css {
        Some(path) => tokio::fs::read_to_string(path).await.unwrap_or_default(),
        None => String::new(),
    };
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}
