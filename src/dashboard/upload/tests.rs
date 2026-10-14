use super::*;

#[test]
fn validate_relative_path_accepts_normal_paths() {
    assert!(validate_relative_path("index.html").is_ok());
    assert!(validate_relative_path("css/style.css").is_ok());
    assert!(validate_relative_path("a/b/c.txt").is_ok());
}

#[test]
fn validate_relative_path_rejects_traversal_and_absolute_paths() {
    assert!(validate_relative_path("").is_err());
    assert!(validate_relative_path("/index.html").is_err());
    assert!(validate_relative_path("../secret").is_err());
    assert!(validate_relative_path("a/../b").is_err());
    assert!(validate_relative_path("a/./b").is_err());
    assert!(validate_relative_path("a//b").is_err());
    assert!(validate_relative_path("a\\b").is_err());
    assert!(validate_relative_path("a\u{0}b").is_err());
    assert!(validate_relative_path(".").is_err());
    assert!(validate_relative_path("..").is_err());
}

#[test]
fn validate_relative_path_rejects_every_windows_device_name() {
    for name in [
        "nul",
        "a/NUL.txt",
        "nul .txt",
        "com\u{b9}",
        "LPT\u{b3}.html",
        "conin$",
        "CONOUT$.log",
    ] {
        assert!(validate_relative_path(name).is_err(), "{name}");
    }
    for name in ["null", "com10", "lpt0.txt", "console.txt"] {
        assert!(validate_relative_path(name).is_ok(), "{name}");
    }
}

#[test]
fn validate_relative_path_rejects_colons() {
    assert!(validate_relative_path("C:foo/x").is_err());
    assert!(validate_relative_path("index.html:stream").is_err());
    assert!(validate_relative_path("dir/a:b").is_err());
}

#[test]
fn validate_relative_path_rejects_too_many_segments() {
    let deep = (0..MAX_PATH_SEGMENTS)
        .map(|_| "a")
        .collect::<Vec<_>>()
        .join("/");
    assert!(validate_relative_path(&deep).is_ok());
    let too_deep = (0..=MAX_PATH_SEGMENTS)
        .map(|_| "a")
        .collect::<Vec<_>>()
        .join("/");
    assert!(validate_relative_path(&too_deep).is_err());
}

#[test]
fn validate_relative_path_rejects_too_long_paths() {
    let ok = "a".repeat(MAX_PATH_LEN);
    assert!(validate_relative_path(&ok).is_ok());
    let too_long = "a".repeat(MAX_PATH_LEN + 1);
    assert!(validate_relative_path(&too_long).is_err());
}

#[test]
fn validate_relative_path_rejects_windows_reserved_device_names() {
    for name in [
        "CON", "con", "Con", "PRN", "AUX", "NUL", "COM1", "com9", "LPT1", "lpt9",
    ] {
        assert!(validate_relative_path(name).is_err(), "{name}");
        assert!(
            validate_relative_path(&format!("{name}.txt")).is_err(),
            "{name}.txt"
        );
        assert!(
            validate_relative_path(&format!("dir/{name}")).is_err(),
            "dir/{name}"
        );
    }
    assert!(validate_relative_path("COM10").is_ok());
    assert!(validate_relative_path("NULL").is_ok());
    assert!(validate_relative_path("scones.txt").is_ok());
}

#[test]
fn validate_relative_path_rejects_trailing_dot_or_space_segments() {
    assert!(validate_relative_path("foo.").is_err());
    assert!(validate_relative_path("foo ").is_err());
    assert!(validate_relative_path("dir./file").is_err());
    assert!(validate_relative_path("dir /file").is_err());
    assert!(validate_relative_path("foo.bar").is_ok());
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

#[tokio::test]
async fn upload_rejects_an_invalid_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(
        boundary,
        &[
            ("site", None, b"example.com"),
            ("file", Some("../evil.txt"), b"hello"),
        ],
    );
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(upload_dir_entries(dir.path()).is_empty());
}

#[tokio::test]
async fn upload_rejects_more_files_than_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let mut parts: Vec<(&str, Option<&str>, &[u8])> = vec![("site", None, b"example.com")];
    let names: Vec<String> = (0..=MAX_UPLOAD_FILES)
        .map(|i| format!("f{i}.txt"))
        .collect();
    for name in &names {
        parts.push(("file", Some(name.as_str()), b"x"));
    }
    let body = multipart_body(boundary, &parts);
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(upload_dir_entries(dir.path()).is_empty());
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
async fn upload_rejects_zero_files() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(boundary, &[("site", None, b"example.com")]);
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(upload_dir_entries(dir.path()).is_empty());
}

#[tokio::test]
async fn upload_rejects_a_missing_site() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(boundary, &[("file", Some("index.html"), b"<html></html>")]);
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(upload_dir_entries(dir.path()).is_empty());
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
async fn upload_rejects_an_invalid_site_before_touching_the_relay() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(
        boundary,
        &[
            ("site", None, b""),
            ("file", Some("index.html"), b"<html></html>"),
        ],
    );
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert!(body["error"].as_str().unwrap().contains("invalid site"));
    assert!(upload_dir_entries(dir.path()).is_empty());
}

#[tokio::test]
async fn upload_rejects_an_invalid_title_before_touching_the_relay() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(
        boundary,
        &[
            ("site", None, b"example.com"),
            ("title", None, b"bad\ntitle"),
            ("file", Some("index.html"), b"<html></html>"),
        ],
    );
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert!(body["error"].as_str().unwrap().contains("invalid title"));
    assert!(upload_dir_entries(dir.path()).is_empty());
}

#[tokio::test]
async fn upload_rejects_a_note_without_a_url_before_touching_the_relay() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
    let boundary = "SwingTestBoundary";
    let body = multipart_body(
        boundary,
        &[
            ("site", None, b"example.com"),
            ("note", None, b"true"),
            ("file", Some("index.html"), b"<html></html>"),
        ],
    );
    let app = router(state);
    let resp = call(
        app,
        multipart_request("/api/publish/upload", boundary, body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "invalid note: needs a URL");
    assert!(upload_dir_entries(dir.path()).is_empty());
}

async fn upload_with(parts: &[(&str, Option<&str>, &[u8])]) -> (StatusCode, serde_json::Value) {
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
async fn upload_rejects_a_message_over_the_content_limit() {
    let long = vec![b'a'; crate::nostr::budget::MAX_CONTENT_BYTES + 1];
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("message", None, &long),
        ("file", Some("index.html"), b"<html></html>"),
    ])
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("invalid message: "),
        "{body}"
    );
}

#[tokio::test]
async fn upload_rejects_an_invalid_check_mode_part() {
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("check_size", None, b"loud"),
        ("file", Some("index.html"), b"<html></html>"),
    ])
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("invalid check_size")
    );
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

#[tokio::test]
async fn upload_with_breaking_links_under_warn_goes_on_to_ipfs() {
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("check_links", None, b"warn"),
        ("file", Some("index.html"), br#"<a href="/x">x</a>"#),
    ])
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "agent is not ready");
}

#[tokio::test]
async fn upload_rejects_an_invalid_check_links_part() {
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("check_links", None, b"strict"),
        ("file", Some("index.html"), b"<html></html>"),
    ])
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("invalid check_links")
    );
}

#[tokio::test]
async fn upload_with_dotfiles_under_warn_goes_on_to_ipfs() {
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("check_dotfiles", None, b"warn"),
        ("file", Some(".env"), b"SECRET=1"),
    ])
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "agent is not ready");
}

#[tokio::test]
async fn upload_rejects_paths_that_differ_only_in_case() {
    let (status, body) = upload_with(&[
        ("site", None, b"example.com"),
        ("file", Some("Index.html"), b"a"),
        ("file", Some("index.HTML"), b"b"),
    ])
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("duplicate"));
}

#[tokio::test]
async fn upload_rejects_a_file_and_a_directory_at_the_same_path() {
    for (first, second) in [("a", "a/b"), ("a/b", "a")] {
        let (status, body) = upload_with(&[
            ("site", None, b"example.com"),
            ("file", Some(first), b"x"),
            ("file", Some(second), b"y"),
        ])
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{first} then {second}");
        assert!(
            body["error"].as_str().unwrap().contains("conflicts"),
            "{body}"
        );
    }
}

#[tokio::test]
async fn upload_rejects_an_oversized_text_field() {
    let big = vec![b'a'; MAX_TEXT_FIELD_BYTES + 1];
    let (status, body) = upload_with(&[
        ("message", None, &big),
        ("site", None, b"example.com"),
        ("file", Some("index.html"), b"x"),
    ])
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("message"));
}

#[tokio::test]
async fn upload_skips_unknown_fields() {
    let big = vec![b'a'; MAX_TEXT_FIELD_BYTES * 4];
    let (status, body) = upload_with(&[
        ("unknown", None, &big),
        ("site", None, b""),
        ("file", Some("index.html"), b"x"),
    ])
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("invalid site"));
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
