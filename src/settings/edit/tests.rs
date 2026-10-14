use super::*;
use crate::test_support::{MINIMAL_TOML, config_at};

fn base_config(dir: &Path) -> Config {
    let path = dir.join("swing.toml");
    std::fs::write(&path, MINIMAL_TOML).unwrap();
    Config::load(Some(&path)).unwrap()
}

#[test]
fn a_broken_config_file_is_reported_without_its_source_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(&path, "[nostr]\nsecret_key = \"nsec1verysecret\n").unwrap();
    let Err(EditError::Invalid(e)) = load_document(&path) else {
        panic!("expected an invalid-config error");
    };
    let shown = format!("{e:#}");
    assert!(shown.contains("line 2"), "{shown}");
    assert!(!shown.contains("verysecret"), "{shown}");
}

fn items(pairs: &[(&str, &str)]) -> BTreeMap<String, InputValue> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), InputValue::Str(v.to_string())))
        .collect()
}

#[test]
fn validation_resolves_relative_paths_like_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    let text = "[agent]\nstate_dir = \"./data\"\n[dashboard]\ncustom_css = \"theme/custom.css\"\n";
    std::fs::write(&path, text).unwrap();
    let validated = config::build_config_for_file(text, &path, |_| None).unwrap();
    let loaded = Config::load(Some(&path)).unwrap();
    let base = std::path::absolute(dir.path()).unwrap();
    assert_eq!(validated.agent.state_dir, base.join("data"));
    assert_eq!(validated.agent.state_dir, loaded.agent.state_dir);
    assert_eq!(validated.kubo.repo, loaded.kubo.repo);
    assert_eq!(validated.dashboard.custom_css, loaded.dashboard.custom_css);
}

#[test]
fn rejects_unknown_key() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = base_config(dir.path());
    let items = items(&[("kubo.binary", "/bin/ipfs")]);
    let err = update(&cfg, &items).unwrap_err();
    assert!(err.to_string().contains("kubo.binary"));
}

#[test]
fn rejects_env_sourced_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(&path, MINIMAL_TOML).unwrap();
    let mut cfg = config::build_config_from_str(MINIMAL_TOML, |k| match k {
        "SWING_MAX_TOTAL_STORAGE" => Some("10GB".to_string()),
        _ => None,
    })
    .unwrap();
    cfg.config_path = path;
    cfg.config_exists = true;
    let items = items(&[("policy.max_total_storage", "20GB")]);
    let err = update(&cfg, &items).unwrap_err();
    assert!(err.to_string().contains("environment variable"));
}

#[test]
fn writer_preserves_comments_and_unrelated_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(
            &path,
            "# a comment\n[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\nmirror_set = \"keep-me\"\n",
        )
        .unwrap();
    let cfg = Config::load(Some(&path)).unwrap();
    let items = items(&[("policy.max_total_storage", "20GB")]);
    let updated = update(&cfg, &items).unwrap();
    assert_eq!(updated.policy.max_total_storage, 20 * (1u64 << 30));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# a comment"));
    assert!(text.contains("mirror_set = \"keep-me\""));
}

#[cfg(unix)]
#[test]
fn symlinked_config_is_written_through_to_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.toml");
    std::fs::write(&real, MINIMAL_TOML).unwrap();
    let link = dir.path().join("swing.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let cfg = Config::load(Some(&link)).unwrap();
    let items = items(&[("nostr.mirror_set", "newset")]);
    update(&cfg, &items).unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::read_to_string(&real)
            .unwrap()
            .contains("mirror_set = \"newset\"")
    );
}

#[cfg(unix)]
#[test]
fn a_new_file_and_its_missing_parent_are_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("nested");
    let path = parent.join("swing.toml");
    let cfg = config_at(MINIMAL_TOML, path.clone(), false);
    let items = items(&[("nostr.mirror_set", "newset")]);
    update(&cfg, &items).unwrap();
    assert_eq!(crate::test_support::mode(&path), 0o600);
    assert_eq!(crate::test_support::mode(&parent), 0o700);
}

#[cfg(unix)]
#[test]
fn existing_file_permissions_are_tightened_to_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(&path, MINIMAL_TOML).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let cfg = Config::load(Some(&path)).unwrap();
    let items = items(&[("nostr.mirror_set", "newset")]);
    update(&cfg, &items).unwrap();
    assert_eq!(crate::test_support::mode(&path), 0o600);
}

#[test]
fn validation_failure_leaves_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(&path, MINIMAL_TOML).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let cfg = Config::load(Some(&path)).unwrap();
    let items = items(&[("policy.max_total_storage", "not-a-size")]);
    assert!(update(&cfg, &items).is_err());
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after);
}

#[test]
fn every_editable_setting_round_trips_its_shown_value() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = base_config(dir.path());
    for setting in SETTINGS.iter().filter(|s| s.editable) {
        let shown = raw_value(&cfg, setting.key).unwrap();
        let input = match shown.clone() {
            RawValue::Str(s) => InputValue::Str(s),
            RawValue::List(l) => InputValue::List(l),
        };
        let items = BTreeMap::from([(setting.key.to_string(), input)]);
        let reloaded = update(&cfg, &items)
            .unwrap_or_else(|e| panic!("{}: {shown:?} was refused: {e}", setting.key));
        assert_eq!(
            reloaded.source_of(setting.key),
            Some(config::Source::File),
            "{} was not written to the file",
            setting.key
        );
        assert_eq!(
            serde_json::to_value(raw_value(&reloaded, setting.key).unwrap()).unwrap(),
            serde_json::to_value(&shown).unwrap(),
            "{} changed after a round trip",
            setting.key
        );
    }
}

#[test]
fn values_of_the_wrong_shape_are_refused_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = base_config(dir.path());
    let before = std::fs::read_to_string(&cfg.config_path).unwrap();
    let cases = [
        (
            "nostr.relays",
            InputValue::Str("wss://r".into()),
            "nostr.relays expects a list",
        ),
        (
            "nostr.mirror_set",
            InputValue::List(vec!["a".into()]),
            "nostr.mirror_set expects a single string",
        ),
        (
            "policy.remove_on_unfollow",
            InputValue::Str("maybe".into()),
            "policy.remove_on_unfollow: invalid boolean",
        ),
        (
            "policy.keep_versions",
            InputValue::Str("five".into()),
            "policy.keep_versions: expected an integer",
        ),
        (
            "policy.min_update_interval",
            InputValue::Str("soon".into()),
            "policy.min_update_interval: invalid duration",
        ),
        (
            "policy.nip05",
            InputValue::Str("sometimes".into()),
            "policy.nip05: invalid mode: sometimes",
        ),
    ];
    for (key, value, expected) in cases {
        let items = BTreeMap::from([(key.to_string(), value)]);
        let Err(EditError::Invalid(e)) = update(&cfg, &items) else {
            panic!("{key}: expected an invalid-value error");
        };
        let shown = format!("{e:#}");
        assert!(shown.contains(expected), "{key}: {shown}");
    }
    assert_eq!(std::fs::read_to_string(&cfg.config_path).unwrap(), before);
}

#[test]
fn empty_relay_list_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = base_config(dir.path());
    let items = BTreeMap::from([("nostr.relays".to_string(), InputValue::List(vec![]))]);
    let err = update(&cfg, &items).unwrap_err();
    assert!(err.to_string().contains("at least one relay"));
}

#[test]
fn setup_writes_generated_key_and_leaves_setup_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    let cfg = config_at("", path.clone(), false);
    assert!(cfg.nostr.secret_key.is_none());
    let items = BTreeMap::new();
    let keys = setup_keys(None).unwrap();
    let reloaded = setup(&cfg, Some(&keys), &items).unwrap();
    assert_eq!(
        reloaded
            .nostr
            .secret_key
            .as_ref()
            .map(|k| k.expose_secret()),
        Some(keys.secret_key().to_secret_hex().as_str())
    );
    assert!(path.exists());
}

#[test]
fn setup_for_a_signer_app_writes_no_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    let cfg = config_at("", path.clone(), false);
    let items = BTreeMap::from([(
        "nostr.relays".to_string(),
        InputValue::List(vec!["wss://relay.example".to_string()]),
    )]);
    let reloaded = setup(&cfg, None, &items).unwrap();
    assert!(reloaded.nostr.secret_key.is_none());
    assert_eq!(reloaded.nostr.relays, vec!["wss://relay.example"]);
}

#[test]
fn pin_addrs_keeps_other_keys_and_rejects_non_address_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(&path, "# keep\n[nostr]\nrelays = [\"wss://r\"]\n").unwrap();
    let cfg = Config::load(Some(&path)).unwrap();
    let listen: SocketAddr = "127.0.0.1:8083".parse().unwrap();
    let reloaded = pin_addrs(&cfg, &[("dashboard.listen", listen)]).unwrap();
    assert_eq!(reloaded.dashboard.listen, listen);
    assert_eq!(reloaded.nostr.relays, vec!["wss://r"]);
    assert!(std::fs::read_to_string(&path).unwrap().contains("# keep"));
    assert!(pin_addrs(&cfg, &[("nostr.mirror_set", listen)]).is_err());
}
