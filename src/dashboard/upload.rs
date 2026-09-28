use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::io::AsyncWriteExt;

use super::AppState;
use super::api::{ApiError, PublishFields, PublishOutcome, internal, run_publish};
use super::dto;

pub const MAX_UPLOAD_FILES: usize = 10_000;
pub const MAX_PATH_SEGMENTS: usize = 32;
pub const MAX_PATH_LEN: usize = 4096;

pub fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("file path must not be empty".to_string());
    }
    if path.len() > MAX_PATH_LEN {
        return Err(format!(
            "file path must be at most {MAX_PATH_LEN} bytes: {path}"
        ));
    }
    if path.starts_with('/') {
        return Err(format!("file path must not start with /: {path}"));
    }
    if path.contains('\\') {
        return Err(format!("file path must not contain a backslash: {path}"));
    }
    if path.chars().any(|c| c.is_control()) {
        return Err(format!(
            "file path must not contain control characters: {path}"
        ));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > MAX_PATH_SEGMENTS {
        return Err(format!(
            "file path must have at most {MAX_PATH_SEGMENTS} segments: {path}"
        ));
    }
    for segment in segments {
        if segment.is_empty() {
            return Err(format!("file path must not contain empty segments: {path}"));
        }
        if segment == "." || segment == ".." {
            return Err(format!(
                "file path must not contain . or .. segments: {path}"
            ));
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Err(format!(
                "file path segments must not end with a dot or space: {path}"
            ));
        }
        if is_windows_reserved_segment(segment) {
            return Err(format!(
                "file path must not use a reserved device name: {path}"
            ));
        }
    }
    Ok(())
}

// Reserved on Windows regardless of extension (e.g. `nul.txt`); rejected on every platform so a
// site published from Linux still mirrors cleanly onto a Windows checkout.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_windows_reserved_segment(segment: &str) -> bool {
    let base = segment.split('.').next().unwrap_or(segment);
    WINDOWS_RESERVED_NAMES
        .iter()
        .any(|name| base.eq_ignore_ascii_case(name))
}

static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_upload_name() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let counter = UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{counter}", now.as_nanos(), std::process::id())
}

pub async fn cleanup_upload_dir(state_dir: &Path) -> Result<()> {
    let upload_dir = state_dir.join("upload");
    match tokio::fs::remove_dir_all(&upload_dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("cleaning {}", upload_dir.display())),
    }
}

/// Creates `path` (and any missing parents) like `create_dir_all`, but directories this call
/// actually creates are `0o700` on unix. A directory that already exists is left untouched.
async fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path).await
}

async fn create_private_file(path: &Path) -> std::io::Result<tokio::fs::File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await
}

// status() gives 500 when the request body stream itself fails (e.g. the client drops mid-upload); that is not a server fault.
fn multipart_error_to_api(err: axum::extract::multipart::MultipartError) -> ApiError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge(err.to_string())
    } else {
        ApiError::BadRequest(err.to_string())
    }
}

// Owns the upload temp dir so a timeout or client disconnect (which drops the handler future
// mid-await, skipping any code after the `.await`) still frees it via Drop.
struct UploadDirGuard {
    dest: Option<PathBuf>,
}

impl UploadDirGuard {
    fn new(dest: PathBuf) -> Self {
        Self { dest: Some(dest) }
    }

    async fn cleanup(mut self) {
        if let Some(dest) = self.dest.take()
            && let Err(e) = tokio::fs::remove_dir_all(&dest).await
        {
            tracing::warn!(path = %dest.display(), error = %e, "cleaning up upload directory failed");
        }
    }
}

impl Drop for UploadDirGuard {
    fn drop(&mut self) {
        if let Some(dest) = self.dest.take()
            && let Err(e) = std::fs::remove_dir_all(&dest)
        {
            tracing::warn!(path = %dest.display(), error = %e, "cleaning up upload directory failed");
        }
    }
}

struct ParsedUpload {
    fields: PublishFields,
    file_count: usize,
}

async fn receive_upload(multipart: &mut Multipart, dest: &Path) -> Result<ParsedUpload, ApiError> {
    let mut site: Option<String> = None;
    let mut url: Option<String> = None;
    let mut title: Option<String> = None;
    let mut message: Option<String> = None;
    let mut nip05: Option<String> = None;
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut file_count = 0usize;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => return Err(multipart_error_to_api(e)),
        };
        match field.name().unwrap_or("") {
            "site" => site = Some(field.text().await.map_err(multipart_error_to_api)?),
            "url" => url = Some(field.text().await.map_err(multipart_error_to_api)?),
            "title" => title = Some(field.text().await.map_err(multipart_error_to_api)?),
            "message" => message = Some(field.text().await.map_err(multipart_error_to_api)?),
            "nip05" => nip05 = Some(field.text().await.map_err(multipart_error_to_api)?),
            "file" => {
                if file_count >= MAX_UPLOAD_FILES {
                    return Err(ApiError::BadRequest(format!(
                        "file must include at most {MAX_UPLOAD_FILES} entries"
                    )));
                }
                let Some(filename) = field.file_name().map(str::to_string) else {
                    return Err(ApiError::BadRequest(
                        "file part is missing a filename".to_string(),
                    ));
                };
                validate_relative_path(&filename).map_err(ApiError::BadRequest)?;
                if !seen_paths.insert(filename.clone()) {
                    return Err(ApiError::BadRequest(format!(
                        "duplicate file path: {filename}"
                    )));
                }
                let target = dest.join(&filename);
                if let Some(parent) = target.parent() {
                    create_private_dir_all(parent)
                        .await
                        .map_err(|e| internal("creating an upload directory failed", e))?;
                }
                let mut out = create_private_file(&target)
                    .await
                    .map_err(|e| internal("creating an uploaded file failed", e))?;
                let mut field = field;
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => out
                            .write_all(&chunk)
                            .await
                            .map_err(|e| internal("writing an uploaded file failed", e))?,
                        Ok(None) => break,
                        Err(e) => return Err(multipart_error_to_api(e)),
                    }
                }
                file_count += 1;
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let Some(site) = site else {
        return Err(ApiError::BadRequest("missing site".to_string()));
    };
    if file_count == 0 {
        return Err(ApiError::BadRequest(
            "file must include at least one entry".to_string(),
        ));
    }

    Ok(ParsedUpload {
        fields: PublishFields {
            site,
            url,
            title,
            message,
            nip05,
        },
        file_count,
    })
}

pub async fn publish_upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let upload_root = state.config.agent.state_dir.join("upload");
    create_private_dir_all(&upload_root)
        .await
        .map_err(|e| internal("creating the upload directory failed", e))?;
    let dest: PathBuf = upload_root.join(random_upload_name());
    create_private_dir_all(&dest)
        .await
        .map_err(|e| internal("creating the upload directory failed", e))?;

    let guard = UploadDirGuard::new(dest.clone());
    let result = handle_upload(&state, &mut multipart, &dest).await;
    guard.cleanup().await;

    let (outcome, file_count) = result?;
    match outcome {
        PublishOutcome::Success(result) => Ok(Json(dto::PublishUploadResultDto {
            result,
            files: file_count,
        })
        .into_response()),
        PublishOutcome::Nip05Failed(resp) => Ok(resp),
    }
}

async fn handle_upload(
    state: &AppState,
    multipart: &mut Multipart,
    dest: &Path,
) -> Result<(PublishOutcome, usize), ApiError> {
    let parsed = receive_upload(multipart, dest).await?;
    let outcome = run_publish(state, dest, parsed.fields).await?;
    Ok((outcome, parsed.file_count))
}

#[cfg(test)]
mod tests {
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
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("upload").join("abc");
        create_private_dir_all(&nested).await.unwrap();
        for p in [dir.path().join("upload"), nested] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", p.display());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn create_private_file_makes_new_files_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        create_private_file(&path).await.unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    use super::super::router;
    use super::super::test_support::*;
    use axum::http::StatusCode;

    fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, filename, content) in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match filename {
                Some(fname) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{fname}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(content);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        body
    }

    fn multipart_request(
        uri: &str,
        boundary: &str,
        body: Vec<u8>,
    ) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(axum::body::Body::from(body))
            .unwrap()
    }

    fn upload_dir_entries(state_dir: &Path) -> Vec<PathBuf> {
        match std::fs::read_dir(state_dir.join("upload")) {
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
        let body = error_body(resp).await;
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
        let body = error_body(resp).await;
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
        let body = error_body(resp).await;
        assert!(body["error"].as_str().unwrap().contains("invalid title"));
        assert!(upload_dir_entries(dir.path()).is_empty());
    }

    #[test]
    fn upload_dir_guard_removes_the_dir_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("upload").join("abc");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("partial.bin"), b"data").unwrap();
        drop(UploadDirGuard::new(dest.clone()));
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn upload_dir_guard_cleanup_removes_the_dir_once() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("upload").join("abc");
        std::fs::create_dir_all(&dest).unwrap();
        UploadDirGuard::new(dest.clone()).cleanup().await;
        assert!(!dest.exists());
    }

    // Regression test for the bug where TimeoutLayer (or a client disconnect) drops the handler
    // future mid-`.await`, skipping the post-await cleanup and leaking `<state_dir>/upload/<id>/`.
    #[tokio::test]
    async fn upload_temp_dir_is_removed_when_the_request_is_cancelled_mid_upload() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_with(dir.path().to_path_buf(), 2 * (1u64 << 30));
        let boundary = "SwingTestBoundary";

        let mut preamble = Vec::new();
        preamble.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        preamble.extend_from_slice(
            b"Content-Disposition: form-data; name=\"site\"\r\n\r\nexample.com\r\n",
        );
        preamble.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        preamble.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"index.html\"\r\n\
              Content-Type: application/octet-stream\r\n\r\n<html>",
        );

        // Yields the preamble once, then stalls forever (no more data, no terminating
        // boundary), simulating a slow upload that gets cancelled mid-flight.
        let mut chunk = Some(preamble);
        let stream = futures_util::stream::poll_fn(move |_cx| match chunk.take() {
            Some(data) => std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(data))),
            None => std::task::Poll::Pending,
        });
        let body = axum::body::Body::from_stream(stream);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/publish/upload")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .unwrap();

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

        assert!(
            upload_dir_entries(dir.path()).is_empty(),
            "the upload temp dir must be removed once the request is cancelled"
        );
    }
}
