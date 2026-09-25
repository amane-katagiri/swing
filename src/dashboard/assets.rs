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
const SETUP_JS: &str = include_str!("../../web/setup.js");
const PAIRING_JS: &str = include_str!("../../web/pairing.js");
const LOGIN_JS: &str = include_str!("../../web/login.js");
const DESKTOP_JS: &str = include_str!("../../web/desktop.js");
const DESKTOP_WINDOW_JS: &str = include_str!("../../web/desktop-window.js");
const DESKTOP_SETTINGS_JS: &str = include_str!("../../web/desktop-settings.js");
const DESKTOP_ICONS_SVG: &str = include_str!("../../web/desktop-icons.svg");
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

pub async fn setup_js() -> Response {
    asset("text/javascript; charset=utf-8", SETUP_JS)
}

pub async fn pairing_js() -> Response {
    asset("text/javascript; charset=utf-8", PAIRING_JS)
}

pub async fn login_js() -> Response {
    asset("text/javascript; charset=utf-8", LOGIN_JS)
}

pub async fn desktop_js() -> Response {
    asset("text/javascript; charset=utf-8", DESKTOP_JS)
}

pub async fn desktop_window_js() -> Response {
    asset("text/javascript; charset=utf-8", DESKTOP_WINDOW_JS)
}

pub async fn desktop_settings_js() -> Response {
    asset("text/javascript; charset=utf-8", DESKTOP_SETTINGS_JS)
}

pub async fn desktop_icons_svg() -> Response {
    asset("image/svg+xml", DESKTOP_ICONS_SVG)
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
    async fn style_and_scripts_have_expected_content_types() {
        for (path, expected) in [
            ("/favicon.svg", "image/svg+xml"),
            ("/favicon-32.png", "image/png"),
            ("/apple-touch-icon.png", "image/png"),
            ("/style.css", "text/css; charset=utf-8"),
            ("/desktop.css", "text/css; charset=utf-8"),
            ("/boot.js", "text/javascript; charset=utf-8"),
            ("/app.js", "text/javascript; charset=utf-8"),
            ("/graph.js", "text/javascript; charset=utf-8"),
            ("/storage.js", "text/javascript; charset=utf-8"),
            ("/i18n.js", "text/javascript; charset=utf-8"),
            ("/util.js", "text/javascript; charset=utf-8"),
            ("/ui.js", "text/javascript; charset=utf-8"),
            ("/sites.js", "text/javascript; charset=utf-8"),
            ("/webring.js", "text/javascript; charset=utf-8"),
            ("/publish.js", "text/javascript; charset=utf-8"),
            ("/settings.js", "text/javascript; charset=utf-8"),
            ("/desktop.js", "text/javascript; charset=utf-8"),
            ("/desktop-window.js", "text/javascript; charset=utf-8"),
            ("/desktop-settings.js", "text/javascript; charset=utf-8"),
            ("/desktop-icons.svg", "image/svg+xml"),
            ("/desktop-page.html", "text/html; charset=utf-8"),
            ("/desktop-page.css", "text/css; charset=utf-8"),
            ("/desktop-frame.css", "text/css; charset=utf-8"),
            ("/desktop-banner", "image/gif"),
            ("/fonts/pixelmplus12-regular.woff2", "font/woff2"),
            ("/fonts/pixelmplus12-bold.woff2", "font/woff2"),
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
