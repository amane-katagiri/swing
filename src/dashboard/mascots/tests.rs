use super::super::router;
use super::super::test_support::*;
use super::image::png;
use super::*;

fn write_pack(dir: &std::path::Path, id: &str, manifest: &str, sprite: &[u8]) {
    let pack_dir = dir.join(id);
    std::fs::create_dir_all(&pack_dir).unwrap();
    std::fs::write(pack_dir.join("manifest.json"), manifest).unwrap();
    std::fs::write(pack_dir.join("sprite.png"), sprite).unwrap();
}

fn load_error(dir: &std::path::Path, id: &str) -> String {
    let root = Dir::open_root(dir).unwrap();
    match load_user_pack(&root, id) {
        Ok(_) => panic!("{id} loaded"),
        Err(reason) => reason,
    }
}

fn candidate_error(dir: &std::path::Path, name: &str) -> String {
    let entry = std::fs::read_dir(dir)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| entry.file_name() == name)
        .unwrap();
    candidate_id(&entry).unwrap_err()
}

fn minimal_manifest(sprite: &str) -> String {
    format!(
        r#"{{"format":1,"sprite":"{sprite}","frame":{{"width":8,"height":8}},"animations":{{"idle":{{"frames":[0]}}}}}}"#
    )
}

#[test]
fn pack_ids_are_short_lowercase_names() {
    let longest = "a".repeat(32);
    let too_long = "a".repeat(33);
    for (id, valid) in [
        ("a", true),
        ("neko2", true),
        ("my-pack-99", true),
        (longest.as_str(), true),
        ("", false),
        (too_long.as_str(), false),
        ("-abc", false),
        ("Abc", false),
        ("abc_def", false),
        ("abc/def", false),
    ] {
        assert_eq!(is_valid_pack_id(id), valid, "{id:?}");
    }
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
    write_pack(
        dir.path(),
        "Bad_Id",
        &minimal_manifest("sprite.png"),
        &png(8, 8),
    );
    let registry = MascotRegistry::load(Some(dir.path()));
    assert!(registry.find("Bad_Id").is_none());
    assert!(candidate_error(dir.path(), "Bad_Id").contains("invalid pack id"));
}

#[test]
fn a_bundled_id_collision_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write_pack(
        dir.path(),
        "neko",
        &minimal_manifest("sprite.png"),
        &png(8, 8),
    );
    let registry = MascotRegistry::load(Some(dir.path()));
    let bundled = BUNDLED.iter().find(|b| b.id == "neko").unwrap();
    let pack = registry.find("neko").unwrap();
    assert_eq!(pack.sprite.as_ref(), bundled.sprite);
    assert_eq!(pack.manifest.as_ref(), bundled.manifest.as_bytes());
    assert!(candidate_error(dir.path(), "neko").contains("collides with a bundled pack"));
}

#[test]
fn a_pack_with_a_bad_manifest_or_sprite_is_skipped() {
    let manifest = minimal_manifest("sprite.png");
    let mut too_big_sprite = png(8, 8);
    too_big_sprite.resize((MAX_SPRITE_BYTES + 1) as usize, 0);
    let too_big_manifest = format!(
        "{manifest}{}",
        " ".repeat((MAX_MANIFEST_BYTES + 1) as usize)
    );
    type Sprite<'a> = Option<(&'a str, Vec<u8>)>;
    let cases: Vec<(&str, Option<String>, Sprite, &[&str])> = vec![
        ("missing manifest", None, None, &[]),
        ("bad json", Some("not json".into()), None, &[]),
        (
            "wrong format",
            Some(manifest.replace(r#""format":1"#, r#""format":2"#)),
            Some(("sprite.png", png(8, 8))),
            &["manifest.json format must be 1"],
        ),
        (
            "sprite path traversal",
            Some(minimal_manifest("../sprite.png")),
            Some(("../sprite.png", b"x".to_vec())),
            &[],
        ),
        (
            "sprite with a bad extension",
            Some(minimal_manifest("sprite.bmp")),
            Some(("sprite.bmp", b"x".to_vec())),
            &[],
        ),
        ("sprite missing", Some(manifest.clone()), None, &[]),
        (
            "sprite over the byte limit",
            Some(manifest.clone()),
            Some(("sprite.png", too_big_sprite)),
            &["byte limit"],
        ),
        (
            "manifest over the byte limit",
            Some(too_big_manifest),
            Some(("sprite.png", png(8, 8))),
            &["manifest.json ", "byte limit"],
        ),
        (
            "oversized sprite image",
            Some(manifest.clone()),
            Some(("sprite.png", png(65535, 65535))),
            &[],
        ),
    ];
    for (case, manifest, sprite, reason) in cases {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        if let Some(manifest) = manifest {
            std::fs::write(pack_dir.join("manifest.json"), manifest).unwrap();
        }
        if let Some((name, bytes)) = sprite {
            std::fs::write(pack_dir.join(name), bytes).unwrap();
        }
        let registry = MascotRegistry::load(Some(dir.path()));
        assert!(registry.find("my-pack").is_none(), "{case}");
        if !reason.is_empty() {
            let got = load_error(dir.path(), "my-pack");
            for part in reason {
                assert!(got.contains(part), "{case}: {got}");
            }
        }
    }
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
        std::ffi::CString::new(pack_dir.join("sprite.png").as_os_str().as_encoded_bytes()).unwrap();
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
fn user_packs_are_capped_before_they_are_read() {
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
    let loaded: Vec<usize> = (0..40)
        .filter(|i| registry.find(&format!("pack-{i:02}")).is_some())
        .collect();
    assert_eq!(loaded, (0..MAX_USER_PACKS).collect::<Vec<_>>());

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
            .is_some(),
        "a broken pack still takes a slot"
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

    let resp = call(router(state), get("/mascots/index.json")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
    let index = json_body(resp).await;
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

    let resp = call(
        router(std::sync::Arc::clone(&state)),
        get("/mascots/my-pack/manifest.json"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );

    let resp = call(
        router(std::sync::Arc::clone(&state)),
        get("/mascots/my-pack/sprite.png"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("content-type").unwrap(), "image/png");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), png(16, 8).as_slice());

    let resp = call(router(state), get("/mascots/my-pack/other.png")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_pack_id_is_not_found() {
    let app = router(test_state());
    let resp = call(app, get("/mascots/does-not-exist/manifest.json")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn every_bundled_mascot_pack_is_served() {
    let resp = call(router(test_state()), get("/mascots/index.json")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let index = json_body(resp).await;
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
        let resp = call(router(test_state()), get(&manifest_path)).await;
        assert_eq!(resp.status(), StatusCode::OK, "{manifest_path}");
        let manifest = json_body(resp).await;
        let sprite = manifest["sprite"].as_str().unwrap();
        let sprite_path = format!("{base}{sprite}");
        let resp = call(router(test_state()), get(&sprite_path)).await;
        assert_eq!(resp.status(), StatusCode::OK, "{sprite_path}");
    }
}
