mod image;
mod nofollow;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tracing::warn;

use super::AppState;
use super::assets::bytes_asset;
use image::check_sprite;
use nofollow::{Dir, read_limited};

const JSON: &str = "application/json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SPRITE_BYTES: u64 = 1024 * 1024;
const MAX_USER_PACKS: usize = 32;
const MAX_PACK_ID_LEN: usize = 32;
const MAX_DIR_ENTRIES: usize = 1024;
const MAX_LOGGED_SKIPS: usize = 10;

struct BundledPack {
    id: &'static str,
    manifest: &'static str,
    sprite: &'static [u8],
}

const BUNDLED: &[BundledPack] = &[
    BundledPack {
        id: "yureko",
        manifest: include_str!("../../../web/mascots/yureko/manifest.json"),
        sprite: include_bytes!("../../../web/mascots/yureko/sprite.png"),
    },
    BundledPack {
        id: "mochi",
        manifest: include_str!("../../../web/mascots/mochi/manifest.json"),
        sprite: include_bytes!("../../../web/mascots/mochi/sprite.png"),
    },
    BundledPack {
        id: "neko",
        manifest: include_str!("../../../web/mascots/neko/manifest.json"),
        sprite: include_bytes!("../../../web/mascots/neko/sprite.png"),
    },
];

struct MascotPack {
    manifest: Bytes,
    sprite_name: String,
    sprite: Bytes,
    sprite_content_type: &'static str,
}

pub struct MascotRegistry {
    packs: HashMap<String, MascotPack>,
    index_json: Bytes,
}

impl MascotRegistry {
    pub fn load(mascots_dir: Option<&Path>) -> Self {
        let mut order = Vec::new();
        let mut packs = HashMap::new();
        for bundled in BUNDLED {
            order.push(bundled.id.to_string());
            packs.insert(
                bundled.id.to_string(),
                MascotPack {
                    manifest: Bytes::from_static(bundled.manifest.as_bytes()),
                    sprite_name: "sprite.png".to_string(),
                    sprite: Bytes::from_static(bundled.sprite),
                    sprite_content_type: "image/png",
                },
            );
        }

        if let Some(dir) = mascots_dir {
            for (id, pack) in load_user_packs(dir) {
                order.push(id.clone());
                packs.insert(id, pack);
            }
        }

        let index_json = build_index_json(&order);
        Self { packs, index_json }
    }

    pub fn index_json(&self) -> Bytes {
        self.index_json.clone()
    }

    fn find(&self, id: &str) -> Option<&MascotPack> {
        self.packs.get(id)
    }
}

fn build_index_json(order: &[String]) -> Bytes {
    let packs: Vec<_> = order
        .iter()
        .map(|id| json!({"id": id, "base": format!("/mascots/{id}/")}))
        .collect();
    Bytes::from(serde_json::to_vec(&json!({ "packs": packs })).expect("mascot index serializes"))
}

fn is_valid_pack_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_PACK_ID_LEN {
        return false;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_valid_sprite_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('.') {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    sprite_extension(name).is_some()
}

fn sprite_extension(name: &str) -> Option<&'static str> {
    match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn load_user_pack(root: &Dir, id: &str) -> Result<MascotPack, String> {
    let dir = root
        .open_dir(id)
        .map_err(|err| format!("pack directory {err}"))?;

    let manifest_bytes = read_limited(&dir, "manifest.json", MAX_MANIFEST_BYTES)
        .map_err(|err| format!("manifest.json {err}"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|err| format!("manifest.json is not valid JSON: {err}"))?;
    let Some(manifest) = manifest.as_object() else {
        return Err("manifest.json must be a JSON object".to_string());
    };
    if manifest.get("format").and_then(serde_json::Value::as_i64) != Some(1) {
        return Err("manifest.json format must be 1".to_string());
    }
    let Some(sprite_name) = manifest.get("sprite").and_then(|v| v.as_str()) else {
        return Err("manifest.json sprite must be a string".to_string());
    };
    if !is_valid_sprite_name(sprite_name) {
        return Err(format!(
            "manifest.json sprite {sprite_name:?} is not a plain .png/.gif/.webp file name"
        ));
    }
    let content_type = sprite_extension(sprite_name).expect("validated above");

    let sprite_bytes = read_limited(&dir, sprite_name, MAX_SPRITE_BYTES)
        .and_then(|bytes| check_sprite(&bytes, content_type).map(|()| bytes))
        .map_err(|err| format!("sprite {sprite_name:?} {err}"))?;

    Ok(MascotPack {
        manifest: Bytes::from(manifest_bytes),
        sprite_name: sprite_name.to_string(),
        sprite: Bytes::from(sprite_bytes),
        sprite_content_type: content_type,
    })
}

#[derive(Default)]
struct SkipLog {
    logged: usize,
    suppressed: usize,
}

impl SkipLog {
    fn skip(&mut self, entry: &OsStr, reason: &str) {
        if self.logged < MAX_LOGGED_SKIPS {
            self.logged += 1;
            warn!(entry = ?entry, reason, "skipping mascot pack");
        } else {
            self.suppressed += 1;
        }
    }

    fn finish(self) {
        if self.suppressed > 0 {
            warn!(
                count = self.suppressed,
                "skipping more mascot packs; only the first {MAX_LOGGED_SKIPS} are logged individually"
            );
        }
    }
}

fn candidate_id(entry: &std::fs::DirEntry) -> Result<String, String> {
    let name = entry.file_name();
    let Some(id) = name.to_str() else {
        return Err("directory name is not valid UTF-8".to_string());
    };
    if !is_valid_pack_id(id) {
        return Err("invalid pack id (expected ^[a-z0-9][a-z0-9-]{0,31}$)".to_string());
    }
    if BUNDLED.iter().any(|b| b.id == id) {
        return Err("pack id collides with a bundled pack".to_string());
    }
    let file_type = entry
        .file_type()
        .map_err(|err| format!("cannot stat: {err}"))?;
    if file_type.is_symlink() {
        return Err("pack directory is a symlink".to_string());
    }
    if !file_type.is_dir() {
        return Err("not a directory".to_string());
    }
    Ok(id.to_string())
}

fn candidate_ids(dir: &Path, log: &mut SkipLog) -> Option<Vec<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            warn!(
                dir = ?dir,
                error = %err,
                "mascots_dir could not be read; serving bundled mascot packs only"
            );
            return None;
        }
    };
    let mut ids = Vec::new();
    for (scanned, entry) in entries.enumerate() {
        if scanned == MAX_DIR_ENTRIES {
            warn!(
                dir = ?dir,
                "mascots_dir has more than {MAX_DIR_ENTRIES} entries; ignoring the rest"
            );
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                log.skip(OsStr::new(""), &format!("unreadable entry: {err}"));
                continue;
            }
        };
        match candidate_id(&entry) {
            Ok(id) => ids.push(id),
            Err(reason) => log.skip(&entry.file_name(), &reason),
        }
    }
    Some(ids)
}

fn load_user_packs(dir: &Path) -> Vec<(String, MascotPack)> {
    let mut log = SkipLog::default();
    let Some(mut ids) = candidate_ids(dir, &mut log) else {
        return Vec::new();
    };
    ids.sort();
    if ids.len() > MAX_USER_PACKS {
        for id in ids.split_off(MAX_USER_PACKS) {
            log.skip(
                OsStr::new(&id),
                &format!("more than {MAX_USER_PACKS} user packs; keeping the first {MAX_USER_PACKS} by id"),
            );
        }
    }

    let root = match Dir::open_root(dir) {
        Ok(root) => root,
        Err(err) => {
            warn!(
                dir = ?dir,
                error = %err,
                "mascots_dir could not be opened; serving bundled mascot packs only"
            );
            log.finish();
            return Vec::new();
        }
    };
    let mut packs = Vec::new();
    for id in ids {
        match load_user_pack(&root, &id) {
            Ok(pack) => packs.push((id, pack)),
            Err(reason) => log.skip(OsStr::new(&id), &reason),
        }
    }
    log.finish();
    packs
}

fn registry(state: &AppState) -> &MascotRegistry {
    state
        .mascots
        .as_ref()
        .expect("mascot routes are only mounted when [dashboard].ui is enabled")
}

pub async fn index(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    bytes_asset(&headers, JSON, registry(&state).index_json())
}

pub async fn file(
    State(state): State<Arc<AppState>>,
    UrlPath((id, file)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(pack) = registry(&state).find(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if file == "manifest.json" {
        return bytes_asset(&headers, JSON, pack.manifest.clone());
    }
    if file == pack.sprite_name {
        return bytes_asset(&headers, pack.sprite_content_type, pack.sprite.clone());
    }
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests;
