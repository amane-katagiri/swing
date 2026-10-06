use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use super::AppState;
use crate::config::DashboardConfig;

enum AssetBody {
    Text(&'static str),
    Bytes(&'static [u8]),
}

struct StaticAsset {
    path: &'static str,
    content_type: &'static str,
    body: AssetBody,
}

macro_rules! text_asset {
    ($content_type:expr, $file:literal) => {
        StaticAsset {
            path: concat!("/", $file),
            content_type: $content_type,
            body: AssetBody::Text(include_str!(concat!("../../web/", $file))),
        }
    };
}

macro_rules! bytes_asset {
    ($content_type:expr, $file:literal) => {
        StaticAsset {
            path: concat!("/", $file),
            content_type: $content_type,
            body: AssetBody::Bytes(include_bytes!(concat!("../../web/", $file))),
        }
    };
}

const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";

const STATIC_ASSETS: &[StaticAsset] = &[
    StaticAsset {
        path: "/",
        content_type: "text/html; charset=utf-8",
        body: AssetBody::Text(include_str!("../../web/index.html")),
    },
    text_asset!("image/svg+xml", "favicon.svg"),
    bytes_asset!("image/png", "favicon-32.png"),
    bytes_asset!("image/png", "apple-touch-icon.png"),
    text_asset!(CSS, "style.css"),
    text_asset!(CSS, "desktop.css"),
    text_asset!(CSS, "desktop-dialog.css"),
    text_asset!(CSS, "desktop-wallpaper.css"),
    text_asset!(CSS, "desktop-mascot-settings.css"),
    text_asset!(CSS, "desktop-system-settings.css"),
    text_asset!(CSS, "desktop-frame.css"),
    text_asset!(CSS, "desktop-mascot.css"),
    text_asset!(JS, "boot.js"),
    text_asset!(JS, "app.js"),
    text_asset!(JS, "graph.js"),
    text_asset!(JS, "storage.js"),
    text_asset!(JS, "i18n.js"),
    text_asset!(JS, "util.js"),
    text_asset!(JS, "ui.js"),
    text_asset!(JS, "sites.js"),
    text_asset!(JS, "webring.js"),
    text_asset!(JS, "publish.js"),
    text_asset!(JS, "settings.js"),
    text_asset!(JS, "settings-notify.js"),
    text_asset!(JS, "notify-settings.js"),
    text_asset!(JS, "stats.js"),
    text_asset!(JS, "setup.js"),
    text_asset!(JS, "pairing.js"),
    text_asset!(JS, "login.js"),
    text_asset!(JS, "desktop.js"),
    text_asset!(JS, "desktop-window.js"),
    text_asset!(JS, "desktop-settings.js"),
    text_asset!(JS, "desktop-updates.js"),
    text_asset!(JS, "desktop-dialog.js"),
    text_asset!(JS, "desktop-wallpaper.js"),
    text_asset!(JS, "desktop-wallpaper-image.js"),
    text_asset!(JS, "desktop-combobox.js"),
    text_asset!(JS, "desktop-focus.js"),
    text_asset!(JS, "desktop-drag.js"),
    text_asset!(JS, "desktop-scale.js"),
    text_asset!(JS, "desktop-mascot.js"),
    text_asset!(JS, "desktop-mascot-pack.js"),
    text_asset!(JS, "desktop-mascot-sprite.js"),
    text_asset!(JS, "desktop-mascot-behavior.js"),
    text_asset!(JS, "desktop-mascot-balloon.js"),
    text_asset!(JS, "desktop-mascot-settings.js"),
    text_asset!(JS, "desktop-notify.js"),
    text_asset!(JS, "desktop-notify-settings.js"),
    text_asset!(JS, "desktop-system-settings.js"),
    text_asset!(JS, "desktop-mirror-add.js"),
    text_asset!("image/svg+xml", "desktop-icons.svg"),
    bytes_asset!("font/woff2", "fonts/pixelmplus12-regular.woff2"),
    bytes_asset!("font/woff2", "fonts/pixelmplus12-bold.woff2"),
];

fn serve_static(asset: &'static StaticAsset) -> Response {
    match asset.body {
        AssetBody::Text(body) => {
            ([(header::CONTENT_TYPE, asset.content_type)], body).into_response()
        }
        AssetBody::Bytes(body) => {
            ([(header::CONTENT_TYPE, asset.content_type)], body).into_response()
        }
    }
}

pub(super) fn register(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    STATIC_ASSETS.iter().fold(router, |router, asset| {
        router.route(asset.path, get(move || async move { serve_static(asset) }))
    })
}

pub(super) fn bytes_asset(content_type: &str, body: Bytes) -> Response {
    ([(header::CONTENT_TYPE, content_type.to_string())], body).into_response()
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

pub async fn desktop_page_css(State(state): State<Arc<AppState>>) -> Response {
    bytes_asset("text/css; charset=utf-8", desktop(&state).page_css.clone())
}

pub async fn desktop_banner(State(state): State<Arc<AppState>>) -> Response {
    bytes_asset(
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
        const DESKTOP_PAGE_HTML: &str = include_str!("../../web/desktop-page.html");
        const DESKTOP_PAGE_CSS: &str = include_str!("../../web/desktop-page.css");
        const DESKTOP_BANNER_GIF: &[u8] = include_bytes!("../../web/desktop-banner.gif");
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
mod tests {
    use super::super::router;
    use super::super::test_support::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    #[tokio::test]
    async fn every_imported_module_is_served() {
        let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
        let mut modules = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&web).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "js") {
                let source = std::fs::read_to_string(&path).unwrap();
                for part in source.split("from './").skip(1) {
                    modules.insert(part.split('\'').next().unwrap().to_string());
                }
            }
        }
        assert!(modules.contains("pairing.js"));
        for module in modules {
            let req = Request::builder()
                .uri(format!("/{module}"))
                .header("Host", "127.0.0.1:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(router(test_state()), req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{module}");
        }
    }

    #[tokio::test]
    async fn every_linked_stylesheet_is_served() {
        let index = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/index.html");
        let source = std::fs::read_to_string(&index).unwrap();
        let mut hrefs = std::collections::BTreeSet::new();
        for part in source.split("<link rel=\"stylesheet\" href=\"").skip(1) {
            hrefs.insert(part.split('"').next().unwrap().to_string());
        }
        assert!(hrefs.contains("/style.css"));
        for href in hrefs {
            let req = Request::builder()
                .uri(href.clone())
                .header("Host", "127.0.0.1:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(router(test_state()), req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{href}");
        }
    }

    #[tokio::test]
    async fn index_is_served_as_html_with_security_headers() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/")
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert!(resp.headers().get("content-security-policy").is_some());
        assert_eq!(
            resp.headers().get("referrer-policy").unwrap(),
            "no-referrer"
        );
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "SAMEORIGIN");
    }

    #[tokio::test]
    async fn table_driven_assets_have_expected_content_types() {
        for asset in super::STATIC_ASSETS {
            let app = router(test_state());
            let req = Request::builder()
                .uri(asset.path)
                .header("Host", "127.0.0.1:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(app, req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{}", asset.path);
            assert_eq!(
                resp.headers().get("content-type").unwrap(),
                asset.content_type,
                "{}",
                asset.path
            );
        }
    }

    #[tokio::test]
    async fn state_dependent_assets_have_expected_content_types() {
        for (path, expected) in [
            ("/desktop-page.html", "text/html; charset=utf-8"),
            ("/desktop-page.css", "text/css; charset=utf-8"),
            ("/desktop-banner", "image/gif"),
            ("/custom.css", "text/css; charset=utf-8"),
        ] {
            let app = router(test_state());
            let req = Request::builder()
                .uri(path)
                .header("Host", "127.0.0.1:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(app, req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers().get("content-type").unwrap(),
                expected,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn desktop_page_assets_come_from_the_configured_files() {
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("page.html");
        let page_css = dir.path().join("page.css");
        let banner = dir.path().join("banner.gif");
        std::fs::write(&page, "<!doctype html><title>mine</title>").unwrap();
        std::fs::write(&page_css, "body { color: red }").unwrap();
        std::fs::write(&banner, b"GIF89a").unwrap();

        let (mut config, secret_hex) = test_config(true);
        config.dashboard.desktop_page = Some(page);
        config.dashboard.desktop_page_css = Some(page_css);
        config.dashboard.desktop_banner = Some(banner);
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);

        for (path, content_type, expected) in [
            (
                "/desktop-page.html",
                "text/html; charset=utf-8",
                &b"<!doctype html><title>mine</title>"[..],
            ),
            (
                "/desktop-page.css",
                "text/css; charset=utf-8",
                b"body { color: red }",
            ),
            ("/desktop-banner", "image/gif", b"GIF89a"),
        ] {
            let app = router(std::sync::Arc::clone(&state));
            let req = Request::builder()
                .uri(path)
                .header("Host", "127.0.0.1:8082")
                .body(Body::empty())
                .unwrap();
            let resp = call(app, req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers().get("content-type").unwrap(),
                content_type,
                "{path}"
            );
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), expected, "{path}");
        }
    }

    #[test]
    fn an_unreadable_desktop_page_fails_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, secret_hex) = test_config(true);
        config.dashboard.desktop_page = Some(dir.path().join("missing.html"));
        let err = match super::super::AppState::new(
            std::sync::Arc::new(config),
            std::sync::Arc::new(tokio::sync::Notify::new()),
            test_exit(),
            test_keys(&secret_hex),
            TEST_TOKEN.to_string(),
        ) {
            Ok(_) => panic!("expected a startup error"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("missing.html"));
    }

    #[test]
    fn a_banner_with_an_unknown_extension_fails_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let banner = dir.path().join("banner.bmp");
        std::fs::write(&banner, b"BM").unwrap();
        let (mut config, secret_hex) = test_config(true);
        config.dashboard.desktop_banner = Some(banner);
        let err = match super::super::AppState::new(
            std::sync::Arc::new(config),
            std::sync::Arc::new(tokio::sync::Notify::new()),
            test_exit(),
            test_keys(&secret_hex),
            TEST_TOKEN.to_string(),
        ) {
            Ok(_) => panic!("expected a startup error"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("banner.bmp"));
    }
}
