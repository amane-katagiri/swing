use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::{Multipart, State};
use axum::response::{IntoResponse, Response};
use tokio::io::AsyncWriteExt;

use super::AppState;
use super::api::{ApiError, PublishFields, PublishOutcome, run_publish};
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
    }
    Ok(())
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

// multer hides the length-limit error behind a generic MultipartError; only its Display text tells 413 from 400.
fn multipart_error_to_api(err: axum::extract::multipart::MultipartError) -> ApiError {
    let mut current: &dyn std::error::Error = &err;
    loop {
        if current
            .to_string()
            .to_ascii_lowercase()
            .contains("length limit")
        {
            return ApiError::PayloadTooLarge(err.to_string());
        }
        match current.source() {
            Some(source) => current = source,
            None => return ApiError::BadRequest(err.to_string()),
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
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| ApiError::Internal(format!("{e:#}")))?;
                }
                let mut out = tokio::fs::File::create(&target)
                    .await
                    .map_err(|e| ApiError::Internal(format!("{e:#}")))?;
                let mut field = field;
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => out
                            .write_all(&chunk)
                            .await
                            .map_err(|e| ApiError::Internal(format!("{e:#}")))?,
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
    tokio::fs::create_dir_all(&upload_root)
        .await
        .map_err(|e| ApiError::Internal(format!("creating upload directory: {e:#}")))?;
    let dest: PathBuf = upload_root.join(random_upload_name());
    tokio::fs::create_dir_all(&dest)
        .await
        .map_err(|e| ApiError::Internal(format!("creating upload directory: {e:#}")))?;

    let result = handle_upload(&state, &mut multipart, &dest).await;
    if let Err(e) = tokio::fs::remove_dir_all(&dest).await {
        tracing::warn!(path = %dest.display(), error = %e, "cleaning up upload directory failed");
    }

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
}
