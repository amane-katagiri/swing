mod delivery;

use std::path::Path;
use std::sync::{Arc, LazyLock};

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use super::AppState;
use crate::config::DashboardConfig;
use delivery::Prepared;
pub(super) use delivery::bytes_asset;

struct StaticAsset {
    path: &'static str,
    content_type: &'static str,
    body: &'static [u8],
}

macro_rules! asset {
    ($content_type:expr, $file:literal) => {
        StaticAsset {
            path: concat!("/", $file),
            content_type: $content_type,
            body: include_bytes!(concat!("../../../web/", $file)),
        }
    };
}

const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";

const STATIC_ASSETS: &[StaticAsset] = &[
    StaticAsset {
        path: "/",
        content_type: "text/html; charset=utf-8",
        body: include_bytes!("../../../web/index.html"),
    },
    asset!("image/svg+xml", "favicon.svg"),
    asset!("image/png", "favicon-32.png"),
    asset!("image/png", "apple-touch-icon.png"),
    asset!(CSS, "style.css"),
    asset!(CSS, "desktop.css"),
    asset!(CSS, "desktop-dialog.css"),
    asset!(CSS, "desktop-wallpaper.css"),
    asset!(CSS, "desktop-mascot-settings.css"),
    asset!(CSS, "desktop-system-settings.css"),
    asset!(CSS, "desktop-frame.css"),
    asset!(CSS, "desktop-mascot.css"),
    asset!(JS, "boot.js"),
    asset!(JS, "app.js"),
    asset!(JS, "graph.js"),
    asset!(JS, "storage.js"),
    asset!(JS, "i18n.js"),
    asset!(JS, "util.js"),
    asset!(JS, "ui.js"),
    asset!(JS, "sites.js"),
    asset!(JS, "webring.js"),
    asset!(JS, "publish.js"),
    asset!(JS, "settings.js"),
    asset!(JS, "settings-notify.js"),
    asset!(JS, "notify-settings.js"),
    asset!(JS, "stats.js"),
    asset!(JS, "network.js"),
    asset!(JS, "setup.js"),
    asset!(JS, "pairing.js"),
    asset!(JS, "login.js"),
    asset!(JS, "desktop.js"),
    asset!(JS, "desktop-window.js"),
    asset!(JS, "desktop-settings.js"),
    asset!(JS, "desktop-updates.js"),
    asset!(JS, "desktop-dialog.js"),
    asset!(JS, "desktop-wallpaper.js"),
    asset!(JS, "desktop-wallpaper-image.js"),
    asset!(JS, "desktop-combobox.js"),
    asset!(JS, "desktop-focus.js"),
    asset!(JS, "desktop-drag.js"),
    asset!(JS, "desktop-scale.js"),
    asset!(JS, "desktop-mascot.js"),
    asset!(JS, "desktop-mascot-pack.js"),
    asset!(JS, "desktop-mascot-sprite.js"),
    asset!(JS, "desktop-mascot-behavior.js"),
    asset!(JS, "desktop-mascot-balloon.js"),
    asset!(JS, "desktop-mascot-settings.js"),
    asset!(JS, "desktop-notify.js"),
    asset!(JS, "desktop-notify-settings.js"),
    asset!(JS, "desktop-system-settings.js"),
    asset!(JS, "desktop-fingerprint.js"),
    asset!(JS, "desktop-mirror-add.js"),
    asset!(JS, "desktop-network.js"),
    asset!("image/svg+xml", "desktop-icons.svg"),
    asset!("image/svg+xml", "emoji.svg"),
    asset!("font/woff2", "fonts/pixelmplus12-regular.woff2"),
    asset!("font/woff2", "fonts/pixelmplus12-bold.woff2"),
];

static PREPARED: LazyLock<Vec<Prepared>> = LazyLock::new(|| {
    STATIC_ASSETS
        .iter()
        .map(|asset| Prepared::new(asset.content_type, asset.body))
        .collect()
});

pub(super) fn register(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    STATIC_ASSETS
        .iter()
        .enumerate()
        .fold(router, |router, (index, asset)| {
            router.route(
                asset.path,
                get(move |headers: HeaderMap| async move {
                    PREPARED[index].serve(&headers, asset.content_type, asset.body)
                }),
            )
        })
}

fn desktop(state: &AppState) -> &DesktopAssets {
    state
        .desktop
        .as_ref()
        .expect("desktop asset routes are only mounted when [dashboard].ui is enabled")
}

pub async fn desktop_page(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    bytes_asset(
        &headers,
        "text/html; charset=utf-8",
        desktop(&state).page.clone(),
    )
}

pub async fn desktop_page_css(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    bytes_asset(
        &headers,
        "text/css; charset=utf-8",
        desktop(&state).page_css.clone(),
    )
}

pub async fn desktop_banner(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    bytes_asset(
        &headers,
        desktop(&state).banner_content_type,
        desktop(&state).banner.clone(),
    )
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

pub struct DesktopAssets {
    pub page: Bytes,
    pub page_css: Bytes,
    pub banner: Bytes,
    pub banner_content_type: &'static str,
}

impl DesktopAssets {
    pub fn load(config: &DashboardConfig) -> Result<Self> {
        const DESKTOP_PAGE_HTML: &str = include_str!("../../../web/desktop-page.html");
        const DESKTOP_PAGE_CSS: &str = include_str!("../../../web/desktop-page.css");
        const DESKTOP_BANNER_GIF: &[u8] = include_bytes!("../../../web/desktop-banner.gif");
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

#[cfg(test)]
mod tests;
