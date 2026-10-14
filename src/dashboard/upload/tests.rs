use super::*;

#[test]
fn validate_relative_path_accepts_only_portable_relative_paths() {
    let deep = vec!["a"; MAX_PATH_SEGMENTS].join("/");
    let too_deep = vec!["a"; MAX_PATH_SEGMENTS + 1].join("/");
    let long = "a".repeat(MAX_PATH_LEN);
    let too_long = "a".repeat(MAX_PATH_LEN + 1);
    for path in [
        "index.html",
        "css/style.css",
        "a/b/c.txt",
        "foo.bar",
        deep.as_str(),
        long.as_str(),
        "null",
        "NULL",
        "com10",
        "COM10",
        "lpt0.txt",
        "console.txt",
        "scones.txt",
    ] {
        assert!(validate_relative_path(path).is_ok(), "{path:?}");
    }
    for (case, path) in [
        ("empty", ""),
        ("absolute", "/index.html"),
        ("parent", "../secret"),
        ("inner parent", "a/../b"),
        ("inner current", "a/./b"),
        ("empty segment", "a//b"),
        ("backslash", "a\\b"),
        ("nul byte", "a\u{0}b"),
        ("current", "."),
        ("parent only", ".."),
        ("drive", "C:foo/x"),
        ("stream", "index.html:stream"),
        ("inner colon", "dir/a:b"),
        ("trailing dot", "foo."),
        ("trailing space", "foo "),
        ("directory with trailing dot", "dir./file"),
        ("directory with trailing space", "dir /file"),
        ("too many segments", too_deep.as_str()),
        ("too long", too_long.as_str()),
    ] {
        assert!(validate_relative_path(path).is_err(), "{case}: {path:?}");
    }
}

#[test]
fn validate_relative_path_rejects_every_windows_device_name() {
    for name in [
        "CON", "con", "Con", "PRN", "AUX", "NUL", "COM1", "com9", "LPT1", "lpt9",
    ] {
        for path in [
            name.to_string(),
            format!("{name}.txt"),
            format!("dir/{name}"),
        ] {
            assert!(validate_relative_path(&path).is_err(), "{path}");
        }
    }
    for path in [
        "nul",
        "a/NUL.txt",
        "nul .txt",
        "com\u{b9}",
        "LPT\u{b3}.html",
        "conin$",
        "CONOUT$.log",
    ] {
        assert!(validate_relative_path(path).is_err(), "{path}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn create_private_dir_all_makes_new_dirs_0700() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir
        .path()
        .join(crate::publish::DASHBOARD_UPLOAD_DIR)
        .join("abc");
    create_dir_blocking(create_private_dir_all_io, &nested)
        .await
        .unwrap();
    for p in [
        dir.path().join(crate::publish::DASHBOARD_UPLOAD_DIR),
        nested,
    ] {
        assert_eq!(crate::test_support::mode(&p), 0o700, "{}", p.display());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn create_private_file_makes_new_files_0600() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    create_private_file(&path).await.unwrap();
    assert_eq!(crate::test_support::mode(&path), 0o600);
}

use super::super::router;
use super::super::test_support::*;
use axum::http::StatusCode;

fn upload_dir_entries(state_dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(state_dir.join(crate::publish::DASHBOARD_UPLOAD_DIR)) {
        Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => panic!("reading upload dir: {e}"),
    }
}

type Part<'a> = (&'a str, Option<&'a str>, &'a [u8]);

async fn upload_with(parts: &[Part<'_>]) -> (StatusCode, serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(boundary, parts);
    let resp = call(
        router(state),
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    let status = resp.status();
    let body = json_body(resp).await;
    assert!(upload_dir_entries(dir.path()).is_empty());
    (status, body)
}

#[tokio::test]
async fn upload_rejects_bad_parts_before_writing_or_touching_the_relay() {
    const HTML: &[u8] = b"<html></html>";
    let names: Vec<String> = (0..=MAX_UPLOAD_FILES)
        .map(|i| format!("f{i}.txt"))
        .collect();
    let mut too_many: Vec<Part> = vec![("site", None, b"example.com")];
    too_many.extend(
        names
            .iter()
            .map(|name| ("file", Some(name.as_str()), &b"x"[..])),
    );
    let long_message = vec![b'a'; crate::nostr::budget::MAX_CONTENT_BYTES + 1];
    let big_field = vec![b'a'; MAX_TEXT_FIELD_BYTES + 1];
    let big_unknown = vec![b'a'; MAX_TEXT_FIELD_BYTES * 4];
    let site = ("site", None, &b"example.com"[..]);
    let index = ("file", Some("index.html"), HTML);
    let cases: Vec<(&str, Vec<Part>, StatusCode, &str)> = vec![
        (
            "invalid relative path",
            vec![site, ("file", Some("../evil.txt"), b"hello")],
            StatusCode::BAD_REQUEST,
            "../evil.txt",
        ),
        (
            "more files than the limit",
            too_many,
            StatusCode::BAD_REQUEST,
            "file must include at most",
        ),
        (
            "no files",
            vec![site],
            StatusCode::BAD_REQUEST,
            "file must include at least one entry",
        ),
        (
            "no site",
            vec![index],
            StatusCode::BAD_REQUEST,
            "missing site",
        ),
        (
            "invalid site",
            vec![("site", None, b""), index],
            StatusCode::BAD_REQUEST,
            "invalid site",
        ),
        (
            "invalid title",
            vec![site, ("title", None, b"bad\ntitle"), index],
            StatusCode::BAD_REQUEST,
            "invalid title",
        ),
        (
            "note without a url",
            vec![site, ("note", None, b"true"), index],
            StatusCode::BAD_REQUEST,
            "invalid note: needs a URL",
        ),
        (
            "message over the content limit",
            vec![site, ("message", None, &long_message), index],
            StatusCode::BAD_REQUEST,
            "invalid message: ",
        ),
        (
            "invalid check_size",
            vec![site, ("check_size", None, b"loud"), index],
            StatusCode::BAD_REQUEST,
            "invalid check_size",
        ),
        (
            "invalid check_links",
            vec![site, ("check_links", None, b"strict"), index],
            StatusCode::BAD_REQUEST,
            "invalid check_links",
        ),
        (
            "paths differing only in case",
            vec![
                site,
                ("file", Some("Index.html"), b"a"),
                ("file", Some("index.HTML"), b"b"),
            ],
            StatusCode::BAD_REQUEST,
            "duplicate",
        ),
        (
            "a file then a directory at its path",
            vec![site, ("file", Some("a"), b"x"), ("file", Some("a/b"), b"y")],
            StatusCode::BAD_REQUEST,
            "conflicts",
        ),
        (
            "a directory then a file at its path",
            vec![site, ("file", Some("a/b"), b"x"), ("file", Some("a"), b"y")],
            StatusCode::BAD_REQUEST,
            "conflicts",
        ),
        (
            "oversized text field",
            vec![
                ("message", None, &big_field),
                site,
                ("file", Some("index.html"), b"x"),
            ],
            StatusCode::BAD_REQUEST,
            "message",
        ),
        (
            "unknown fields are skipped whatever their size",
            vec![
                ("unknown", None, &big_unknown),
                ("site", None, b""),
                ("file", Some("index.html"), b"x"),
            ],
            StatusCode::BAD_REQUEST,
            "invalid site",
        ),
        (
            "breaking links under warn go on to ipfs",
            vec![
                site,
                ("check_links", None, b"warn"),
                ("file", Some("index.html"), br#"<a href="/x">x</a>"#),
            ],
            StatusCode::SERVICE_UNAVAILABLE,
            "agent is not ready",
        ),
        (
            "dotfiles under warn go on to ipfs",
            vec![
                site,
                ("check_dotfiles", None, b"warn"),
                ("file", Some(".env"), b"SECRET=1"),
            ],
            StatusCode::SERVICE_UNAVAILABLE,
            "agent is not ready",
        ),
    ];
    for (case, parts, want_status, error) in cases {
        let (status, body) = upload_with(&parts).await;
        assert_eq!(status, want_status, "{case}");
        assert!(
            body["error"].as_str().unwrap().contains(error),
            "{case}: {body}"
        );
    }
}

#[tokio::test]
async fn upload_during_a_running_publish_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(
        boundary,
        &[
            ("site", None, b"example.com"),
            ("file", Some("index.html"), b"<html></html>"),
        ],
    );
    let Ok(publishing) = try_lock_publish(&state) else {
        panic!("publish lock is free");
    };
    let resp = call(
        router(Arc::clone(&state)),
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert!(
        !dir.path()
            .join(crate::publish::DASHBOARD_UPLOAD_DIR)
            .exists()
    );
    drop(publishing);
}

#[tokio::test]
async fn create_private_dir_refuses_an_existing_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upload");
    create_dir_blocking(create_private_dir_io, &path)
        .await
        .unwrap();
    let err = create_dir_blocking(create_private_dir_io, &path)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
}

#[tokio::test]
async fn upload_over_the_body_limit_is_rejected_with_json_413() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 16);
    let boundary = "SwingTestBoundary";
    let body = multipart_body(
        boundary,
        &[
            ("site", None, b"example.com"),
            ("file", Some("index.html"), &[b'a'; 4096]),
        ],
    );
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = json_body(resp).await;
    assert!(body["error"].is_string());
    assert!(upload_dir_entries(dir.path()).is_empty());
}

#[tokio::test]
async fn upload_with_dotfiles_under_require_is_422_with_the_check_results() {
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("file", Some("index.html"), b"<html></html>"),
        ("file", Some(".well-known/nostr.json"), b"{}"),
        ("file", Some(".env"), b"SECRET=1"),
        ("file", Some(".git/HEAD"), b"ref"),
        ("file", Some(".git/config"), b"x"),
    ])
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("dotfiles_allow"));
    let dotfiles = &body["checks"]["dotfiles"];
    assert_eq!(dotfiles["status"], "found");
    assert_eq!(dotfiles["mode"], "require");
    assert_eq!(dotfiles["count"], 2);
    assert_eq!(dotfiles["paths"], serde_json::json!([".env", ".git"]));
    assert_eq!(body["checks"]["size"]["status"], "ok");
    assert_eq!(body["checks"]["size"]["bytes"], 13 + 2 + 8 + 3 + 1);
    assert_eq!(body["checks"]["links"]["status"], "ok");
    assert_eq!(body["checks"]["links"]["mode"], "warn");
    assert!(body["checks"]["links"]["guide"].is_null());
    assert!(body["checks"]["unchanged"].is_null());
    assert_eq!(body["nip05"]["status"], "off");
}

#[tokio::test]
async fn upload_with_breaking_links_under_require_is_422_with_the_link_results() {
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("check_links", None, b"require"),
        (
            "file",
            Some("index.html"),
            br#"<link rel="stylesheet" href="/style.css"><form method="post"></form>"#,
        ),
    ])
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("--check-links"));
    let links = &body["checks"]["links"];
    assert_eq!(links["status"], "found");
    assert_eq!(links["mode"], "require");
    assert_eq!(links["count"], 3);
    assert_eq!(links["blocking"], 2);
    let kinds: Vec<&str> = links["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["root_relative", "broken", "post_form"]);
    assert_eq!(links["kinds"][0]["blocks"], true);
    assert_eq!(links["kinds"][2]["blocks"], false);
    assert_eq!(
        links["kinds"][0]["items"],
        serde_json::json!([{"file": "index.html", "reference": "/style.css", "files": 1}])
    );
    assert!(links["guide"].as_str().unwrap() == crate::publish::links::SITE_GUIDE_URL);
}

#[test]
fn upload_dir_guard_removes_the_dir_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir
        .path()
        .join(crate::publish::DASHBOARD_UPLOAD_DIR)
        .join("abc");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("partial.bin"), b"data").unwrap();
    drop(UploadDirGuard::new(dest.clone()));
    assert!(!dest.exists());
}

#[tokio::test]
async fn upload_dir_guard_cleanup_removes_the_dir_once() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir
        .path()
        .join(crate::publish::DASHBOARD_UPLOAD_DIR)
        .join("abc");
    std::fs::create_dir_all(&dest).unwrap();
    UploadDirGuard::new(dest.clone()).cleanup().await;
    assert!(!dest.exists());
}

#[tokio::test]
async fn upload_temp_dir_is_removed_when_the_request_is_cancelled_mid_upload() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";

    let mut preamble = Vec::new();
    preamble.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    preamble
        .extend_from_slice(b"Content-Disposition: form-data; name=\"site\"\r\n\r\nexample.com\r\n");
    preamble.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    preamble.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"index.html\"\r\n\
              Content-Type: application/octet-stream\r\n\r\n<html>",
    );

    let mut chunk = Some(preamble);
    let stream = futures_util::stream::poll_fn(move |_cx| match chunk.take() {
        Some(data) => std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(data))),
        None => std::task::Poll::Pending,
    });
    let body = axum::body::Body::from_stream(stream);
    let req = multipart_request("/api/publish/upload", boundary, body);

    let app = router(state);
    let handle = tokio::spawn(async move {
        let _ = call(app, req).await;
    });

    for _ in 0..200 {
        if !upload_dir_entries(dir.path()).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        !upload_dir_entries(dir.path()).is_empty(),
        "the upload temp dir was never created"
    );

    handle.abort();
    let _ = handle.await;

    for _ in 0..200 {
        if upload_dir_entries(dir.path()).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        upload_dir_entries(dir.path()).is_empty(),
        "the upload temp dir must be removed once the request is cancelled"
    );
}
