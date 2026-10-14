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
mod tests {
    use super::super::router;
    use super::super::test_support::*;
    use axum::http::StatusCode;

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
            let req = get(format!("/{module}"));
            let resp = call(router(test_state()), req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{module}");
        }
    }

    #[test]
    fn every_referenced_desktop_icon_is_in_the_sprite() {
        let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
        let sprite = std::fs::read_to_string(web.join("desktop-icons.svg")).unwrap();
        let mut ids = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&web).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !(name == "index.html" || name.ends_with(".js")) {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for part in source.split("icon-desk-").skip(1) {
                let tail: String = part
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                    .collect();
                ids.insert(format!("icon-desk-{tail}"));
            }
        }
        for state in ["", "-private", "-unknown", "-offline"] {
            assert!(ids.contains(&format!("icon-desk-tray-network{state}")));
        }
        for id in ids {
            assert!(sprite.contains(&format!("<symbol id=\"{id}\"")), "{id}");
        }
    }

    #[test]
    fn every_static_i18n_key_exists_in_both_languages() {
        let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
        let messages = std::fs::read_to_string(web.join("i18n.js")).unwrap();
        let (en, ja) = messages.split_once("\n  ja: {\n").unwrap();
        let keys = |block: &str| -> std::collections::BTreeSet<String> {
            block
                .lines()
                .filter_map(|line| line.strip_prefix("    ")?.split_once(": ").map(|(k, _)| k))
                .filter(|k| k.chars().all(|c| c.is_ascii_alphanumeric()))
                .map(str::to_string)
                .collect()
        };
        let (en, ja) = (keys(en), keys(ja.split("\n};\n").next().unwrap()));
        let index = std::fs::read_to_string(web.join("index.html")).unwrap();
        let mut used = std::collections::BTreeSet::new();
        for attr in [
            "data-i18n",
            "data-i18n-title",
            "data-i18n-aria-label",
            "data-i18n-placeholder",
        ] {
            for part in index.split(&format!(" {attr}=\"")).skip(1) {
                used.insert(part.split('"').next().unwrap().to_string());
            }
        }
        assert!(used.contains("deskClose"));
        for key in used {
            assert!(en.contains(&key), "en: {key}");
            assert!(ja.contains(&key), "ja: {key}");
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
            let req = get(href.clone());
            let resp = call(router(test_state()), req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{href}");
        }
    }

    #[tokio::test]
    async fn index_is_served_as_html_with_security_headers() {
        let app = router(test_state());
        let req = get("/");
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
            let expected = match asset.path.rsplit_once('.').map(|(_, ext)| ext) {
                None if asset.path == "/" => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("woff2") => "font/woff2",
                _ => panic!("no expected content type for {}", asset.path),
            };
            let app = router(test_state());
            let req = get(asset.path);
            let resp = call(app, req).await;
            assert_eq!(resp.status(), StatusCode::OK, "{}", asset.path);
            assert_eq!(
                resp.headers().get("content-type").unwrap(),
                expected,
                "{}",
                asset.path
            );
        }
    }

    #[test]
    fn etags_differ_between_assets() {
        let tags: std::collections::BTreeSet<_> = super::PREPARED.iter().map(|p| &p.etag).collect();
        assert_eq!(tags.len(), super::STATIC_ASSETS.len());
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
            let req = get(path);
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
            let req = get(path);
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
