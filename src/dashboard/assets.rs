use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use super::AppState;
use crate::config::DashboardConfig;

const INDEX_HTML: &str = include_str!("../../web/index.html");
const STYLE_CSS: &str = include_str!("../../web/style.css");
const DESKTOP_CSS: &str = include_str!("../../web/desktop.css");
const BOOT_JS: &str = include_str!("../../web/boot.js");
const APP_JS: &str = include_str!("../../web/app.js");
const GRAPH_JS: &str = include_str!("../../web/graph.js");
const STORAGE_JS: &str = include_str!("../../web/storage.js");
const I18N_JS: &str = include_str!("../../web/i18n.js");
const UTIL_JS: &str = include_str!("../../web/util.js");
const UI_JS: &str = include_str!("../../web/ui.js");
const SITES_JS: &str = include_str!("../../web/sites.js");
const WEBRING_JS: &str = include_str!("../../web/webring.js");
const PUBLISH_JS: &str = include_str!("../../web/publish.js");
const SETTINGS_JS: &str = include_str!("../../web/settings.js");
const DESKTOP_JS: &str = include_str!("../../web/desktop.js");
const DESKTOP_PAGE_HTML: &str = include_str!("../../web/desktop-page.html");
const DESKTOP_PAGE_CSS: &str = include_str!("../../web/desktop-page.css");
const DESKTOP_FRAME_CSS: &str = include_str!("../../web/desktop-frame.css");
const FAVICON_SVG: &str = include_str!("../../web/favicon.svg");
const FAVICON_32_PNG: &[u8] = include_bytes!("../../web/favicon-32.png");
const APPLE_TOUCH_ICON_PNG: &[u8] = include_bytes!("../../web/apple-touch-icon.png");
const DESKTOP_BANNER_GIF: &[u8] = include_bytes!("../../web/desktop-banner.gif");
const FONT_PIXELMPLUS12_REGULAR: &[u8] =
    include_bytes!("../../web/fonts/pixelmplus12-regular.woff2");
const FONT_PIXELMPLUS12_BOLD: &[u8] = include_bytes!("../../web/fonts/pixelmplus12-bold.woff2");

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

fn bytes_asset(content_type: &str, body: Bytes) -> Response {
    ([(header::CONTENT_TYPE, content_type.to_string())], body).into_response()
}

fn binary_asset(content_type: &'static str, body: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

pub async fn index() -> Response {
    asset("text/html; charset=utf-8", INDEX_HTML)
}

pub async fn favicon() -> Response {
    asset("image/svg+xml", FAVICON_SVG)
}

pub async fn favicon_32() -> Response {
    binary_asset("image/png", FAVICON_32_PNG)
}

pub async fn apple_touch_icon() -> Response {
    binary_asset("image/png", APPLE_TOUCH_ICON_PNG)
}

pub async fn style() -> Response {
    asset("text/css; charset=utf-8", STYLE_CSS)
}

pub async fn desktop_css() -> Response {
    asset("text/css; charset=utf-8", DESKTOP_CSS)
}

pub async fn boot_js() -> Response {
    asset("text/javascript; charset=utf-8", BOOT_JS)
}

pub async fn app_js() -> Response {
    asset("text/javascript; charset=utf-8", APP_JS)
}

pub async fn graph_js() -> Response {
    asset("text/javascript; charset=utf-8", GRAPH_JS)
}

pub async fn storage_js() -> Response {
    asset("text/javascript; charset=utf-8", STORAGE_JS)
}

pub async fn i18n_js() -> Response {
    asset("text/javascript; charset=utf-8", I18N_JS)
}

pub async fn util_js() -> Response {
    asset("text/javascript; charset=utf-8", UTIL_JS)
}

pub async fn ui_js() -> Response {
    asset("text/javascript; charset=utf-8", UI_JS)
}

pub async fn sites_js() -> Response {
    asset("text/javascript; charset=utf-8", SITES_JS)
}

pub async fn webring_js() -> Response {
    asset("text/javascript; charset=utf-8", WEBRING_JS)
}

pub async fn publish_js() -> Response {
    asset("text/javascript; charset=utf-8", PUBLISH_JS)
}

pub async fn settings_js() -> Response {
    asset("text/javascript; charset=utf-8", SETTINGS_JS)
}

pub async fn desktop_js() -> Response {
    asset("text/javascript; charset=utf-8", DESKTOP_JS)
}

fn desktop(state: &AppState) -> &DesktopAssets {
    state
        .desktop
        .as_ref()
        .expect("desktop asset routes are only mounted when [dashboard].ui is enabled")
}

pub async fn desktop_page(State(state): State<Arc<AppState>>) -> Response {
    bytes_asset("text/html; charset=utf-8", desktop(&state).page.clone())
}

pub async fn desktop_frame_css() -> Response {
    asset("text/css; charset=utf-8", DESKTOP_FRAME_CSS)
}

pub async fn desktop_page_css(State(state): State<Arc<AppState>>) -> Response {
    bytes_asset("text/css; charset=utf-8", desktop(&state).page_css.clone())
}

pub async fn desktop_banner(State(state): State<Arc<AppState>>) -> Response {
    bytes_asset(
        desktop(&state).banner_content_type,
        desktop(&state).banner.clone(),
    )
}

pub async fn font_pixelmplus12_regular() -> Response {
    binary_asset("font/woff2", FONT_PIXELMPLUS12_REGULAR)
}

pub async fn font_pixelmplus12_bold() -> Response {
    binary_asset("font/woff2", FONT_PIXELMPLUS12_BOLD)
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

/// The link page, its stylesheet and its banner, each either the bundled
/// default or the file named in `[dashboard]`, read once at startup.
pub struct DesktopAssets {
    pub page: Bytes,
    pub page_css: Bytes,
    pub banner: Bytes,
    pub banner_content_type: &'static str,
}

impl DesktopAssets {
    pub fn load(config: &DashboardConfig) -> Result<Self> {
        Ok(Self {
            page: read_override(config.desktop_page.as_deref(), DESKTOP_PAGE_HTML.as_bytes())?,
            page_css: read_override(
                config.desktop_page_css.as_deref(),
                DESKTOP_PAGE_CSS.as_bytes(),
            )?,
            banner: read_override(config.desktop_banner.as_deref(), DESKTOP_BANNER_GIF)?,
            banner_content_type: match config.desktop_banner.as_deref() {
                Some(path) => image_content_type(path)?,
                None => "image/gif",
            },
        })
    }
}

fn read_override(path: Option<&Path>, bundled: &'static [u8]) -> Result<Bytes> {
    match path {
        Some(path) => {
            let body = std::fs::read(path)
                .with_context(|| format!("reading dashboard asset {}", path.display()))?;
            Ok(Bytes::from(body))
        }
        None => Ok(Bytes::from_static(bundled)),
    }
}

fn image_content_type(path: &Path) -> Result<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => Ok("image/png"),
        "gif" => Ok("image/gif"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        "svg" => Ok("image/svg+xml"),
        _ => bail!(
            "unsupported banner image {} (expected .png, .gif, .jpg, .jpeg, .webp, or .svg)",
            path.display()
        ),
    }
}
