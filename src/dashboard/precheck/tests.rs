use super::super::router;
use super::super::test_support::*;
use axum::http::StatusCode;

async fn check_with(parts: &[(&str, Option<&str>, &[u8])]) -> (StatusCode, serde_json::Value) {
    check_with_limit(parts, 2 * (1u64 << 30)).await
}

async fn check_with_limit(
    parts: &[(&str, Option<&str>, &[u8])],
    max_upload: u64,
) -> (StatusCode, serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with(dir.path().to_path_buf(), max_upload);
    let boundary = "SwingTestBoundary";
    let body = multipart_body(boundary, parts);
    let resp = call(
        router(state),
        multipart_request("/api/publish/check", boundary, body),
    )
    .await;
    let status = resp.status();
    let body = json_body(resp).await;
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "the check must not write anything"
    );
    (status, body)
}

#[tokio::test]
async fn check_reports_dotfiles_size_and_links_without_storing_anything() {
    let (status, body) = check_with(&[
            ("site", None, b"example.com"),
            ("url", None, b"https://example.com/"),
            (
                "files",
                None,
                br#"[{"path":"index.html","size":120},{"path":"img/a.png","size":5000000},{"path":".env","size":3},{"path":"big.js","size":9000000}]"#,
            ),
            (
                "file",
                Some("index.html"),
                br#"<img src="img/a.png"><img src="/b.png"><a href="https://example.com/x">x</a>"#,
            ),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let checks = &body["checks"];
    assert_eq!(checks["dotfiles"]["status"], "found");
    assert_eq!(checks["dotfiles"]["mode"], "require");
    assert_eq!(checks["size"]["bytes"], 120 + 5_000_000 + 3 + 9_000_000);
    let links = &checks["links"];
    assert_eq!(links["mode"], "warn");
    assert_eq!(links["count"], 3);
    assert_eq!(links["blocking"], 2);
    assert_eq!(links["skipped"], 1);
    assert!(checks["unchanged"].is_null());
    assert!(body["abort"].as_str().unwrap().contains("dotfiles_allow"));
}

#[tokio::test]
async fn check_uses_the_requested_modes_and_reads_nothing_when_links_are_off() {
    let (status, body) = check_with(&[
        ("site", None, b"example.com"),
        ("check_dotfiles", None, b"off"),
        ("check_size", None, b"off"),
        ("check_links", None, b"off"),
        ("files", None, br#"[{"path":".env","size":3}]"#),
    ])
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["checks"]["dotfiles"]["status"], "off");
    assert_eq!(body["checks"]["size"]["status"], "off");
    assert_eq!(body["checks"]["links"]["status"], "off");
    assert!(body["abort"].is_null());
}

#[tokio::test]
async fn check_require_returns_the_reason_publish_would_stop_with() {
    let (status, body) = check_with(&[
        ("site", None, b"example.com"),
        ("check_links", None, b"require"),
        ("files", None, br#"[{"path":"ipfs/x.html","size":1}]"#),
    ])
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["checks"]["links"]["kinds"][0]["kind"], "reserved");
    assert!(body["abort"].as_str().unwrap().contains("--check-links"));
}

#[tokio::test]
async fn check_leaves_nip05_off_by_default_and_when_asked() {
    for parts in [
        vec![("site", None, &b"example.com"[..])],
        vec![("site", None, b"example.com"), ("nip05", None, b"off")],
    ] {
        let mut parts = parts;
        parts.push(("files", None, br#"[{"path":"index.html","size":1}]"#));
        let (status, body) = check_with(&parts).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["nip05"]["status"], "off");
        assert!(body["nip05"]["detail"].is_null());
        assert!(body["abort"].is_null());
    }
}

#[tokio::test]
async fn check_nip05_warn_reports_the_result_without_stopping() {
    let (status, body) = check_with(&[
        ("site", None, b"my-site"),
        ("nip05", None, b"warn"),
        ("files", None, br#"[{"path":"index.html","size":1}]"#),
    ])
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["nip05"]["status"], "not_applicable");
    assert!(body["abort"].is_null());
}

#[tokio::test]
async fn check_nip05_require_returns_the_reason_upload_would_stop_with_first() {
    let (status, body) = check_with(&[
        ("site", None, b"my-site"),
        ("nip05", None, b"require"),
        ("check_links", None, b"off"),
        ("files", None, br#"[{"path":"index.html","size":1}]"#),
    ])
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["nip05"]["status"], "not_applicable");
    assert_eq!(
        body["abort"],
        "NIP-05 verification required, but \"my-site\" is not a valid domain name"
    );

    let (status, body) = check_with(&[
        ("site", None, b"my-site"),
        ("nip05", None, b"require"),
        ("files", None, br#"[{"path":".env","size":1}]"#),
    ])
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let abort = body["abort"].as_str().unwrap();
    let (first, rest) = abort.split_once("; ").unwrap();
    assert!(first.starts_with("NIP-05 verification required"), "{abort}");
    assert!(rest.contains("dotfiles_allow"), "{abort}");
}

#[tokio::test]
async fn check_validates_paths_like_upload() {
    for (listing, cause) in [
        (&br#"[{"path":"../x","size":1}]"#[..], ". or .. segments"),
        (br#"[{"path":"/x","size":1}]"#, "must not start with /"),
        (br#"[{"path":"a//b","size":1}]"#, "empty segments"),
        (
            br#"[{"path":"a","size":1},{"path":"A","size":1}]"#,
            "duplicate file path",
        ),
        (
            br#"[{"path":"a","size":1},{"path":"a/b","size":1}]"#,
            "conflicts with another uploaded path",
        ),
        (br#"[{"path":"nul.txt","size":1}]"#, "reserved device name"),
        (br#"[]"#, "at least one entry"),
        (br#"{"path":"x"}"#, "must be a JSON array"),
    ] {
        let (status, body) =
            check_with(&[("site", None, b"example.com"), ("files", None, listing)]).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{}: {body}",
            String::from_utf8_lossy(listing)
        );
        assert!(body["error"].as_str().unwrap().contains(cause), "{body}");
    }
}

#[tokio::test]
async fn check_rejects_contents_that_are_not_listed_or_not_read() {
    for (listing, name, cause) in [
        (
            &br#"[{"path":"a.html","size":1}]"#[..],
            "b.html",
            "not in files",
        ),
        (
            br#"[{"path":"a.png","size":1}]"#,
            "a.png",
            "not read by the link check",
        ),
        (
            br#"[{"path":"big.html","size":9000000}]"#,
            "big.html",
            "not read by the link check",
        ),
    ] {
        let (status, body) = check_with(&[
            ("site", None, b"example.com"),
            ("files", None, listing),
            ("file", Some(name), b"<p>"),
        ])
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert!(body["error"].as_str().unwrap().contains(cause), "{body}");
    }
}

#[tokio::test]
async fn check_rejects_sites_over_the_upload_limit_and_bad_fields() {
    let (status, _) = check_with_limit(
        &[
            ("site", None, b"example.com"),
            ("files", None, br#"[{"path":"a.png","size":2000}]"#),
        ],
        1000,
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    for (parts, cause) in [
        (
            vec![("files", None, &br#"[{"path":"a","size":1}]"#[..])],
            "missing site",
        ),
        (vec![("site", None, &b"example.com"[..])], "missing files"),
        (
            vec![
                ("site", None, b"example.com"),
                ("nip05", None, b"strict"),
                ("files", None, br#"[{"path":"a","size":1}]"#),
            ],
            "invalid nip05",
        ),
        (
            vec![
                ("site", None, b"example.com"),
                ("check_links", None, b"strict"),
                ("files", None, br#"[{"path":"a","size":1}]"#),
            ],
            "invalid check_links",
        ),
        (
            vec![
                ("site", None, b"example.com"),
                ("url", None, b"ftp://example.com/"),
                ("files", None, br#"[{"path":"a","size":1}]"#),
            ],
            "invalid url",
        ),
    ] {
        let (status, body) = check_with(&parts).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains(cause), "{body}");
    }
}
