mod image;
mod nofollow;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
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
                &format!("more than {MAX_USER_PACKS} user packs, keeping the first {MAX_USER_PACKS} by id"),
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

pub async fn index(State(state): State<Arc<AppState>>) -> Response {
    bytes_asset(JSON, registry(&state).index_json())
}

pub async fn file(
    State(state): State<Arc<AppState>>,
    UrlPath((id, file)): UrlPath<(String, String)>,
) -> Response {
    let Some(pack) = registry(&state).find(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if file == "manifest.json" {
        return bytes_asset(JSON, pack.manifest.clone());
    }
    if file == pack.sprite_name {
        return bytes_asset(pack.sprite_content_type, pack.sprite.clone());
    }
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use super::super::router;
    use super::super::test_support::*;
    use super::image::png;
    use super::*;
    use axum::body::Body;
    use axum::http::Request;

    async fn get(app: axum::Router, path: &str) -> Response {
        let req = Request::builder()
            .uri(path)
            .header("Host", "127.0.0.1:8082")
            .body(Body::empty())
            .unwrap();
        call(app, req).await
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn write_pack(dir: &std::path::Path, id: &str, manifest: &str, sprite: &[u8]) {
        let pack_dir = dir.join(id);
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(pack_dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(pack_dir.join("sprite.png"), sprite).unwrap();
    }

    fn minimal_manifest(sprite: &str) -> String {
        format!(
            r#"{{"format":1,"sprite":"{sprite}","frame":{{"width":8,"height":8}},"animations":{{"idle":{{"frames":[0]}}}}}}"#
        )
    }

    #[test]
    fn valid_pack_ids() {
        assert!(is_valid_pack_id("a"));
        assert!(is_valid_pack_id("neko2"));
        assert!(is_valid_pack_id("my-pack-99"));
        assert!(is_valid_pack_id(&"a".repeat(32)));
    }

    #[test]
    fn invalid_pack_ids() {
        assert!(!is_valid_pack_id(""));
        assert!(!is_valid_pack_id(&"a".repeat(33)));
        assert!(!is_valid_pack_id("-abc"));
        assert!(!is_valid_pack_id("Abc"));
        assert!(!is_valid_pack_id("abc_def"));
        assert!(!is_valid_pack_id("abc/def"));
    }

    #[test]
    fn invalid_sprite_names() {
        assert!(!is_valid_sprite_name(""));
        assert!(!is_valid_sprite_name(".hidden.png"));
        assert!(!is_valid_sprite_name("../sprite.png"));
        assert!(!is_valid_sprite_name("sub/sprite.png"));
        assert!(!is_valid_sprite_name("sprite.bmp"));
        assert!(is_valid_sprite_name("sprite.PNG"));
        assert!(is_valid_sprite_name("sprite.gif"));
        assert!(is_valid_sprite_name("sprite.webp"));
    }

    #[test]
    fn a_valid_user_pack_loads() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            "my-pack",
            &minimal_manifest("sprite.png"),
            &png(8, 8),
        );
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_some());
        assert!(registry.find("yureko").is_some());
        assert!(registry.find("mochi").is_some());
        assert!(registry.find("neko").is_some());
    }

    #[test]
    fn an_invalid_id_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path(), "Bad_Id", &minimal_manifest("sprite.png"), b"x");
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("Bad_Id").is_none());
    }

    #[test]
    fn a_bundled_id_collision_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path(), "neko", &minimal_manifest("sprite.png"), b"x");
        let registry = MascotRegistry::load(Some(dir.path()));
        let pack = registry.find("neko").unwrap();
        assert_eq!(pack.sprite_content_type, "image/png");
        assert_ne!(pack.sprite.as_ref(), b"x");
    }

    #[test]
    fn a_missing_manifest_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("my-pack")).unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn bad_json_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(pack_dir.join("manifest.json"), "not json").unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn wrong_format_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            "my-pack",
            r#"{"format":2,"sprite":"sprite.png"}"#,
            b"x",
        );
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn sprite_path_traversal_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("manifest.json"),
            minimal_manifest("../sprite.png"),
        )
        .unwrap();
        std::fs::write(dir.path().join("sprite.png"), b"x").unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn sprite_bad_extension_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("manifest.json"),
            minimal_manifest("sprite.bmp"),
        )
        .unwrap();
        std::fs::write(pack_dir.join("sprite.bmp"), b"x").unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn sprite_missing_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("manifest.json"),
            minimal_manifest("sprite.png"),
        )
        .unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn sprite_too_big_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("manifest.json"),
            minimal_manifest("sprite.png"),
        )
        .unwrap();
        let big = vec![0u8; (MAX_SPRITE_BYTES + 1) as usize];
        std::fs::write(pack_dir.join("sprite.png"), big).unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn manifest_too_big_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        let padding = " ".repeat((MAX_MANIFEST_BYTES + 1) as usize);
        std::fs::write(
            pack_dir.join("manifest.json"),
            format!("{}{padding}", minimal_manifest("sprite.png")),
        )
        .unwrap();
        std::fs::write(pack_dir.join("sprite.png"), b"x").unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_pack_directory_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real-pack");
        write_pack(&real, "unused", &minimal_manifest("sprite.png"), &png(8, 8));
        let real_pack = real.join("unused");
        let link = dir.path().join("linked-pack");
        std::os::unix::fs::symlink(&real_pack, &link).unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("linked-pack").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_sprite_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("manifest.json"),
            minimal_manifest("sprite.png"),
        )
        .unwrap();
        let real_sprite = dir.path().join("real-sprite.png");
        std::fs::write(&real_sprite, png(8, 8)).unwrap();
        std::os::unix::fs::symlink(&real_sprite, pack_dir.join("sprite.png")).unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn bundled_sprites_pass_the_sprite_checks() {
        for bundled in BUNDLED {
            check_sprite(bundled.sprite, "image/png").unwrap();
        }
    }

    #[test]
    fn an_oversized_sprite_image_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            "my-pack",
            &minimal_manifest("sprite.png"),
            &png(65535, 65535),
        );
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_rejected_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fifo =
            std::ffi::CString::new(dir.path().join("fifo").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let root = Dir::open_root(dir.path()).unwrap();
        assert_eq!(
            read_limited(&root, "fifo", 10).unwrap_err(),
            "is not a regular file"
        );

        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("manifest.json"),
            minimal_manifest("sprite.png"),
        )
        .unwrap();
        let sprite =
            std::ffi::CString::new(pack_dir.join("sprite.png").as_os_str().as_encoded_bytes())
                .unwrap();
        assert_eq!(unsafe { libc::mkfifo(sprite.as_ptr(), 0o600) }, 0);
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_linked_sprite_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            "my-pack",
            &minimal_manifest("sprite.png"),
            &png(8, 8),
        );
        std::fs::hard_link(
            dir.path().join("my-pack").join("sprite.png"),
            dir.path().join("elsewhere.png"),
        )
        .unwrap();
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none());
    }

    #[test]
    fn packs_over_the_cap_are_dropped_before_they_are_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pack-00")).unwrap();
        for i in 1..=MAX_USER_PACKS {
            write_pack(
                dir.path(),
                &format!("pack-{i:02}"),
                &minimal_manifest("sprite.png"),
                &png(8, 8),
            );
        }
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("pack-00").is_none());
        assert!(
            registry
                .find(&format!("pack-{:02}", MAX_USER_PACKS - 1))
                .is_some()
        );
        assert!(
            registry
                .find(&format!("pack-{MAX_USER_PACKS:02}"))
                .is_none()
        );
    }

    #[test]
    fn scanning_stops_after_the_entry_cap() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..MAX_DIR_ENTRIES + 50 {
            std::fs::create_dir(dir.path().join(format!("p{i:04}"))).unwrap();
        }
        let mut log = SkipLog::default();
        let ids = candidate_ids(dir.path(), &mut log).unwrap();
        assert_eq!(ids.len(), MAX_DIR_ENTRIES);
    }

    #[test]
    fn skip_warnings_are_capped() {
        let mut log = SkipLog::default();
        for _ in 0..MAX_LOGGED_SKIPS + 5 {
            log.skip(OsStr::new("x"), "reason");
        }
        assert_eq!(log.logged, MAX_LOGGED_SKIPS);
        assert_eq!(log.suppressed, 5);
    }

    #[test]
    fn more_than_32_user_packs_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..40 {
            write_pack(
                dir.path(),
                &format!("pack-{i:02}"),
                &minimal_manifest("sprite.png"),
                &png(8, 8),
            );
        }
        let registry = MascotRegistry::load(Some(dir.path()));
        let user_count = (0..40)
            .filter(|i| registry.find(&format!("pack-{i:02}")).is_some())
            .count();
        assert_eq!(user_count, MAX_USER_PACKS);
        for i in 0..MAX_USER_PACKS {
            assert!(
                registry.find(&format!("pack-{i:02}")).is_some(),
                "pack-{i:02}"
            );
        }
    }

    #[test]
    fn a_missing_mascots_dir_falls_back_to_bundled_packs_only() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let registry = MascotRegistry::load(Some(&missing));
        assert!(registry.find("yureko").is_some());
        assert!(registry.find("mochi").is_some());
        assert!(registry.find("neko").is_some());
    }

    #[tokio::test]
    async fn index_json_lists_bundled_then_user_packs_sorted_by_id() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            "zzz-pack",
            &minimal_manifest("sprite.png"),
            &png(8, 8),
        );
        write_pack(
            dir.path(),
            "aaa-pack",
            &minimal_manifest("sprite.png"),
            &png(8, 8),
        );

        let (mut config, secret_hex) = test_config(true);
        config.dashboard.mascots_dir = Some(dir.path().to_path_buf());
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);

        let resp = get(router(state), "/mascots/index.json").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let index = body_json(resp).await;
        let ids: Vec<&str> = index["packs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["yureko", "mochi", "neko", "aaa-pack", "zzz-pack"]);
    }

    #[tokio::test]
    async fn a_user_pack_manifest_and_sprite_are_served() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            "my-pack",
            &minimal_manifest("sprite.png"),
            &png(16, 8),
        );

        let (mut config, secret_hex) = test_config(true);
        config.dashboard.mascots_dir = Some(dir.path().to_path_buf());
        let state = build_state(config, test_exit(), test_keys(&secret_hex), TEST_TOKEN);

        let resp = get(
            router(std::sync::Arc::clone(&state)),
            "/mascots/my-pack/manifest.json",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );

        let resp = get(
            router(std::sync::Arc::clone(&state)),
            "/mascots/my-pack/sprite.png",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("content-type").unwrap(), "image/png");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), png(16, 8).as_slice());

        let resp = get(router(state), "/mascots/my-pack/other.png").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_unknown_pack_id_is_not_found() {
        let app = router(test_state());
        let resp = get(app, "/mascots/does-not-exist/manifest.json").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn every_bundled_mascot_pack_is_served() {
        let resp = get(router(test_state()), "/mascots/index.json").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let index = body_json(resp).await;
        let packs = index["packs"].as_array().unwrap();
        assert_eq!(
            packs
                .iter()
                .map(|p| p["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["yureko", "mochi", "neko"]
        );
        for pack in packs {
            let base = pack["base"].as_str().unwrap();
            let manifest_path = format!("{base}manifest.json");
            let resp = get(router(test_state()), &manifest_path).await;
            assert_eq!(resp.status(), StatusCode::OK, "{manifest_path}");
            let manifest = body_json(resp).await;
            let sprite = manifest["sprite"].as_str().unwrap();
            let sprite_path = format!("{base}{sprite}");
            let resp = get(router(test_state()), &sprite_path).await;
            assert_eq!(resp.status(), StatusCode::OK, "{sprite_path}");
        }
    }
}
