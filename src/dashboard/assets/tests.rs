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
async fn every_asset_is_served_with_its_content_type() {
    let mut cases: Vec<(&str, &str)> = super::STATIC_ASSETS
        .iter()
        .map(|asset| {
            let expected = match asset.path.rsplit_once('.').map(|(_, ext)| ext) {
                None if asset.path == "/" => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("woff2") => "font/woff2",
                _ => panic!("no expected content type for {}", asset.path),
            };
            (asset.path, expected)
        })
        .collect();
    cases.extend([
        ("/desktop-page.html", "text/html; charset=utf-8"),
        ("/desktop-page.css", "text/css; charset=utf-8"),
        ("/desktop-banner", "image/gif"),
        ("/custom.css", "text/css; charset=utf-8"),
    ]);
    for (path, expected) in cases {
        let resp = call(router(test_state()), get(path)).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            expected,
            "{path}"
        );
    }
}

#[test]
fn etags_differ_between_assets() {
    let tags: std::collections::BTreeSet<_> = super::PREPARED.iter().map(|p| &p.etag).collect();
    assert_eq!(tags.len(), super::STATIC_ASSETS.len());
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
