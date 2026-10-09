use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Multipart, State};
use serde::{Deserialize, Serialize};

use crate::config::CheckMode;
use crate::ipfs::SiteEntry;
use crate::publish::{self, links};

use super::AppState;
use super::dto;
use super::error::{ApiError, internal};
use super::publish::{resolve_modes, validate_site_and_url};
use super::upload::{
    MAX_PATH_LEN, MAX_UPLOAD_FILES, PathRules, multipart_error_to_api, read_field_bytes,
    read_text_field,
};

const MAX_LISTING_BYTES: usize = MAX_UPLOAD_FILES * (MAX_PATH_LEN + 64);

#[derive(Debug, Deserialize)]
struct ListedFile {
    path: String,
    size: u64,
}

#[derive(Debug, Serialize)]
pub struct PublishCheckDto {
    pub checks: dto::PublishChecksDto,
    pub abort: Option<String>,
}

struct ParsedCheck {
    site: String,
    url: Option<String>,
    modes: publish::ModeOverrides,
    listing: Vec<ListedFile>,
    contents: HashMap<String, Vec<u8>>,
}

async fn receive_check(multipart: &mut Multipart) -> Result<ParsedCheck, ApiError> {
    let mut site = None;
    let mut url = None;
    let mut modes = publish::ModeOverrides::default();
    let mut listing: Option<Vec<ListedFile>> = None;
    let mut contents: HashMap<String, Vec<u8>> = HashMap::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => return Err(multipart_error_to_api(e)),
        };
        match field.name().unwrap_or("") {
            "site" => site = Some(read_text_field(field).await?),
            "url" => url = Some(read_text_field(field).await?),
            name @ ("check_dotfiles" | "check_size" | "check_links") => {
                let slot = match name {
                    "check_dotfiles" => &mut modes.check_dotfiles,
                    "check_size" => &mut modes.check_size,
                    _ => &mut modes.check_links,
                };
                *slot = Some(read_text_field(field).await?);
            }
            "files" => {
                let bytes = read_field_bytes(field, MAX_LISTING_BYTES).await?;
                let parsed: Vec<ListedFile> = serde_json::from_slice(&bytes).map_err(|e| {
                    ApiError::BadRequest(format!(
                        "invalid files: must be a JSON array of {{path, size}}: {e}"
                    ))
                })?;
                listing = Some(parsed);
            }
            "file" => {
                let Some(filename) = field.file_name().map(str::to_string) else {
                    return Err(ApiError::BadRequest(
                        "file part is missing a filename".to_string(),
                    ));
                };
                if contents.len() >= MAX_UPLOAD_FILES {
                    return Err(ApiError::BadRequest(format!(
                        "file must include at most {MAX_UPLOAD_FILES} entries"
                    )));
                }
                let bytes = read_field_bytes(field, links::LINK_SCAN_MAX_FILE as usize).await?;
                if contents.insert(filename.clone(), bytes).is_some() {
                    return Err(ApiError::BadRequest(format!(
                        "duplicate file part: {filename}"
                    )));
                }
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
    let Some(listing) = listing else {
        return Err(ApiError::BadRequest("missing files".to_string()));
    };
    Ok(ParsedCheck {
        site,
        url,
        modes,
        listing,
        contents,
    })
}

fn path_key(path: &str) -> Vec<&str> {
    path.split('/').collect()
}

fn site_entries(listing: &[ListedFile], max_upload: u64) -> Result<Vec<SiteEntry>, ApiError> {
    if listing.is_empty() {
        return Err(ApiError::BadRequest(
            "files must include at least one entry".to_string(),
        ));
    }
    if listing.len() > MAX_UPLOAD_FILES {
        return Err(ApiError::BadRequest(format!(
            "files must include at most {MAX_UPLOAD_FILES} entries"
        )));
    }
    let mut rules = PathRules::default();
    let mut entries: BTreeMap<Vec<&str>, Option<u64>> = BTreeMap::new();
    let mut total: u64 = 0;
    for file in listing {
        rules.add(&file.path)?;
        total = total.saturating_add(file.size);
        for (i, _) in file.path.match_indices('/') {
            entries.insert(path_key(&file.path[..i]), None);
        }
        entries.insert(path_key(&file.path), Some(file.size));
    }
    if total > max_upload {
        return Err(ApiError::PayloadTooLarge(format!(
            "the files add up to more than the upload limit ({})",
            crate::format::format_bytes(max_upload)
        )));
    }
    Ok(entries
        .into_iter()
        .map(|(key, size)| SiteEntry {
            path: key.join("/"),
            size,
        })
        .collect())
}

fn check_contents(
    listing: &[ListedFile],
    contents: &HashMap<String, Vec<u8>>,
) -> Result<(), ApiError> {
    let sizes: HashMap<&str, u64> = listing.iter().map(|f| (f.path.as_str(), f.size)).collect();
    for path in contents.keys() {
        match sizes.get(path.as_str()) {
            Some(&size) if links::scanned(path, size) => {}
            Some(_) => {
                return Err(ApiError::BadRequest(format!(
                    "file part is not read by the link check: {path}"
                )));
            }
            None => {
                return Err(ApiError::BadRequest(format!(
                    "file part is not in files: {path}"
                )));
            }
        }
    }
    Ok(())
}

pub async fn publish_check(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<PublishCheckDto>, ApiError> {
    let parsed = receive_check(&mut multipart).await?;
    validate_site_and_url(&parsed.site, parsed.url.as_deref())?;
    let modes = resolve_modes(&parsed.modes, &state.config.publish)?;
    let entries = site_entries(&parsed.listing, state.config.dashboard.max_upload)?;
    check_contents(&parsed.listing, &parsed.contents)?;

    let allow = state.config.publish.dotfiles_allow.clone();
    let ParsedCheck { url, contents, .. } = parsed;
    let local = tokio::task::spawn_blocking(move || {
        let mut contents = contents;
        let report = if modes.check_links == CheckMode::Off {
            None
        } else {
            let read = |path: &str| {
                Ok(contents
                    .remove(path)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
            };
            Some(links::evaluate(&entries, read, url.as_deref())?)
        };
        anyhow::Ok(publish::LocalChecks::evaluate(
            &entries,
            modes.check_dotfiles,
            modes.check_size,
            modes.check_links,
            report,
            &allow,
        ))
    })
    .await
    .map_err(|e| internal("the check task failed", e))?
    .map_err(|e| internal("checking the files failed", e))?;

    Ok(Json(PublishCheckDto {
        checks: dto::publish_checks_dto(&local, None),
        abort: local.abort_message(),
    }))
}

#[cfg(test)]
mod tests {
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
        let body = error_body(resp).await;
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
    async fn check_validates_paths_like_upload() {
        for listing in [
            &br#"[{"path":"../x","size":1}]"#[..],
            br#"[{"path":"/x","size":1}]"#,
            br#"[{"path":"a//b","size":1}]"#,
            br#"[{"path":"a","size":1},{"path":"A","size":1}]"#,
            br#"[{"path":"a","size":1},{"path":"a/b","size":1}]"#,
            br#"[{"path":"nul.txt","size":1}]"#,
            br#"[]"#,
            br#"{"path":"x"}"#,
        ] {
            let (status, body) =
                check_with(&[("site", None, b"example.com"), ("files", None, listing)]).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{}: {body}",
                String::from_utf8_lossy(listing)
            );
        }
    }

    #[tokio::test]
    async fn check_rejects_contents_that_are_not_listed_or_not_read() {
        for (listing, name) in [
            (&br#"[{"path":"a.html","size":1}]"#[..], "b.html"),
            (br#"[{"path":"a.png","size":1}]"#, "a.png"),
            (br#"[{"path":"big.html","size":9000000}]"#, "big.html"),
        ] {
            let (status, body) = check_with(&[
                ("site", None, b"example.com"),
                ("files", None, listing),
                ("file", Some(name), b"<p>"),
            ])
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
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
        for parts in [
            vec![("files", None, &br#"[{"path":"a","size":1}]"#[..])],
            vec![("site", None, &b"example.com"[..])],
            vec![
                ("site", None, b"example.com"),
                ("check_links", None, b"strict"),
                ("files", None, br#"[{"path":"a","size":1}]"#),
            ],
            vec![
                ("site", None, b"example.com"),
                ("url", None, b"ftp://example.com/"),
                ("files", None, br#"[{"path":"a","size":1}]"#),
            ],
        ] {
            let (status, body) = check_with(&parts).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
    }
}
