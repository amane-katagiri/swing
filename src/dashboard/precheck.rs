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
use super::publish::{resolve_modes, validate_site_and_url, verify_own_nip05};
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
    pub nip05: dto::Nip05ResultDto,
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
            name @ ("nip05" | "check_dotfiles" | "check_size" | "check_links") => {
                let slot = match name {
                    "nip05" => &mut modes.nip05,
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

    let pubkey_hex = match modes.nip05 {
        CheckMode::Off => None,
        _ => Some(state.require_own_pubkey()?.to_hex()),
    };

    let allow = state.config.publish.dotfiles_allow.clone();
    let ParsedCheck {
        site,
        url,
        contents,
        ..
    } = parsed;
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
    });
    let nip05 = async {
        match pubkey_hex.as_deref() {
            Some(pubkey_hex) => verify_own_nip05(modes.nip05, &site, pubkey_hex).await,
            None => (dto::nip05_off_dto(), None),
        }
    };
    let (local, (nip05, nip05_abort)) = tokio::join!(local, nip05);
    let local = local
        .map_err(|e| internal("the check task failed", e))?
        .map_err(|e| internal("checking the files failed", e))?;

    let abort = [nip05_abort, local.abort_message()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok(Json(PublishCheckDto {
        nip05,
        checks: dto::publish_checks_dto(&local, None),
        abort: (!abort.is_empty()).then(|| abort.join("; ")),
    }))
}

#[cfg(test)]
mod tests;
