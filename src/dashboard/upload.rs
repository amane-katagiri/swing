use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::io::AsyncWriteExt;

use crate::auth::{create_private_dir_all_io, create_private_dir_io};

use super::AppState;
use super::dto;
use super::error::{ApiError, internal};
use super::locks::PublishLock;
use super::publish::{PublishFields, PublishOutcome, run_publish, try_lock_publish};

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
    if path.contains(':') {
        return Err(format!("file path must not contain a colon: {path}"));
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

// Rejected on every platform, extension or not (`nul.txt`), so a site published from Linux still mirrors onto Windows.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "CONIN$",
    "CONOUT$",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

fn is_windows_reserved_segment(segment: &str) -> bool {
    let base = segment.split('.').next().unwrap_or(segment).trim_end();
    WINDOWS_RESERVED_NAMES
        .iter()
        .any(|name| base.eq_ignore_ascii_case(name))
}

const UPLOAD_NAME_BYTES: usize = 16;

pub async fn cleanup_upload_dir(state_dir: &Path) -> Result<()> {
    let upload_dir = state_dir.join(crate::publish::DASHBOARD_UPLOAD_DIR);
    match tokio::fs::remove_dir_all(&upload_dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("cleaning {}", upload_dir.display())),
    }
}

async fn create_dir_blocking(
    create: fn(&Path) -> std::io::Result<()>,
    path: &Path,
) -> std::io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || create(&path))
        .await
        .map_err(std::io::Error::other)?
}

async fn create_private_file(path: &Path) -> std::io::Result<tokio::fs::File> {
    tokio::fs::OpenOptions::from(crate::auth::private_file_options())
        .open(path)
        .await
}

// status() gives 500 when the request body stream itself fails (e.g. the client drops mid-upload); that is not a server fault.
pub(super) fn multipart_error_to_api(err: axum::extract::multipart::MultipartError) -> ApiError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge(err.to_string())
    } else {
        ApiError::BadRequest(err.to_string())
    }
}

pub const MAX_TEXT_FIELD_BYTES: usize = 64 * 1024;

pub(super) async fn read_text_field(
    field: axum::extract::multipart::Field<'_>,
) -> Result<String, ApiError> {
    let name = field.name().unwrap_or("").to_string();
    let buf = read_field_bytes(field, MAX_TEXT_FIELD_BYTES).await?;
    String::from_utf8(buf).map_err(|_| ApiError::BadRequest(format!("{name} must be UTF-8")))
}

pub(super) async fn read_field_bytes(
    mut field: axum::extract::multipart::Field<'_>,
    max: usize,
) -> Result<Vec<u8>, ApiError> {
    let name = field.name().unwrap_or("").to_string();
    let mut buf = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error_to_api)? {
        if buf.len() + chunk.len() > max {
            return Err(ApiError::BadRequest(format!(
                "{name} must be at most {max} bytes"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

fn path_conflict_or_internal(filename: &str, e: std::io::Error) -> ApiError {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::AlreadyExists | ErrorKind::NotADirectory | ErrorKind::IsADirectory => {
            path_conflict(filename)
        }
        _ => internal("storing an uploaded file failed", e),
    }
}

fn path_conflict(filename: &str) -> ApiError {
    ApiError::BadRequest(format!(
        "file path conflicts with another uploaded path: {filename}"
    ))
}

// A timeout or client disconnect drops the handler mid-await, so only Drop is sure to free the temp dir.
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
        if let Some(dest) = self.dest.take() {
            let remove = move || {
                if let Err(e) = std::fs::remove_dir_all(&dest) {
                    tracing::warn!(path = %dest.display(), error = %e, "cleaning up upload directory failed");
                }
            };
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => drop(handle.spawn_blocking(remove)),
                Err(_) => remove(),
            }
        }
    }
}

#[derive(Default)]
pub(super) struct PathRules {
    seen_paths: HashSet<String>,
    seen_dirs: HashSet<String>,
}

impl PathRules {
    pub(super) fn add(&mut self, filename: &str) -> Result<(), ApiError> {
        validate_relative_path(filename).map_err(ApiError::BadRequest)?;
        let key = filename.to_lowercase();
        let parents: Vec<&str> = key.match_indices('/').map(|(i, _)| &key[..i]).collect();
        if self.seen_dirs.contains(&key) || parents.iter().any(|p| self.seen_paths.contains(*p)) {
            return Err(path_conflict(filename));
        }
        self.seen_dirs.extend(parents.iter().map(|p| p.to_string()));
        if !self.seen_paths.insert(key) {
            return Err(ApiError::BadRequest(format!(
                "duplicate file path: {filename}"
            )));
        }
        Ok(())
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
    let mut note = false;
    let mut modes = crate::publish::ModeOverrides::default();
    let mut rules = PathRules::default();
    let mut created_dirs: HashSet<PathBuf> = HashSet::from([dest.to_path_buf()]);
    let mut file_count = 0usize;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => return Err(multipart_error_to_api(e)),
        };
        match field.name().unwrap_or("") {
            "site" => site = Some(read_text_field(field).await?),
            "url" => url = Some(read_text_field(field).await?),
            "title" => title = Some(read_text_field(field).await?),
            "message" => {
                let text = read_text_field(field).await?;
                crate::publish::validate_message(&text)
                    .map_err(|e| ApiError::BadRequest(format!("invalid message: {e}")))?;
                message = Some(text);
            }
            "note" => {
                note = match read_text_field(field).await?.as_str() {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(ApiError::BadRequest(
                            "invalid note: must be true or false".to_string(),
                        ));
                    }
                };
            }
            name @ ("nip05" | "check_dotfiles" | "check_size" | "check_links"
            | "check_unchanged") => {
                let slot = match name {
                    "nip05" => &mut modes.nip05,
                    "check_dotfiles" => &mut modes.check_dotfiles,
                    "check_size" => &mut modes.check_size,
                    "check_links" => &mut modes.check_links,
                    _ => &mut modes.check_unchanged,
                };
                *slot = Some(read_text_field(field).await?);
            }
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
                rules.add(&filename)?;
                let target = dest.join(&filename);
                if !target.starts_with(dest) {
                    return Err(ApiError::BadRequest(format!(
                        "file path escapes the upload directory: {filename}"
                    )));
                }
                if let Some(parent) = target.parent()
                    && !created_dirs.contains(parent)
                {
                    create_dir_blocking(create_private_dir_all_io, parent)
                        .await
                        .map_err(|e| path_conflict_or_internal(&filename, e))?;
                    created_dirs.insert(parent.to_path_buf());
                }
                let mut out = match create_private_file(&target).await {
                    Ok(out) => out,
                    Err(_) if target.is_dir() => return Err(path_conflict(&filename)),
                    Err(e) => return Err(path_conflict_or_internal(&filename, e)),
                };
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
                let mut field = field;
                while field
                    .chunk()
                    .await
                    .map_err(multipart_error_to_api)?
                    .is_some()
                {}
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
            note,
            modes,
        },
        file_count,
    })
}

pub async fn publish_upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let publishing = try_lock_publish(&state)?;
    let upload_root = state
        .config
        .agent
        .state_dir
        .join(crate::publish::DASHBOARD_UPLOAD_DIR);
    create_dir_blocking(create_private_dir_all_io, &upload_root)
        .await
        .map_err(|e| internal("creating the upload directory failed", e))?;
    let dest: PathBuf = upload_root.join(crate::auth::random_hex(UPLOAD_NAME_BYTES));
    create_dir_blocking(create_private_dir_io, &dest)
        .await
        .map_err(|e| internal("creating the upload directory failed", e))?;

    let guard = UploadDirGuard::new(dest.clone());
    let result = handle_upload(&state, &publishing, &mut multipart, &dest).await;
    guard.cleanup().await;

    let (outcome, file_count) = result?;
    match outcome {
        PublishOutcome::Success(result) => Ok(Json(dto::PublishUploadResultDto {
            result: *result,
            files: file_count,
        })
        .into_response()),
        PublishOutcome::CheckFailed(resp) => Ok(resp),
    }
}

async fn handle_upload(
    state: &AppState,
    publishing: &PublishLock<'_>,
    multipart: &mut Multipart,
    dest: &Path,
) -> Result<(PublishOutcome, usize), ApiError> {
    let parsed = receive_upload(multipart, dest).await?;
    let outcome = run_publish(state, publishing, dest, parsed.fields).await?;
    Ok((outcome, parsed.file_count))
}

#[cfg(test)]
mod tests;
