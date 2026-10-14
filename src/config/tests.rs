use super::*;

#[test]
fn config_parse_errors_point_at_the_line_without_quoting_it() {
    let secret = "nsec1qqqqsecretqqqq";
    let text = format!("[nostr]\nsecret_key = \"{secret}\"\nrelays = 5\n");
    let err = format!("{:#}", parse_config_file(&text).unwrap_err());
    assert!(!err.contains(secret), "{err}");
    assert!(err.contains("line 3, column 10"), "{err}");

    let text = format!("[nostr]\nsecret_key = \"{secret}\n");
    let err = format!("{:#}", parse_config_file(&text).unwrap_err());
    assert!(!err.contains(secret), "{err}");
    assert!(err.contains("line 2"), "{err}");
}

#[test]
fn nostr_file_debug_redacts_the_secret_key() {
    let file = NostrFile {
        secret_key: Some(Zeroizing::new("nsec1qqqqsecretqqqq".to_string())),
        mirror_set: Some("set".to_string()),
        ..Default::default()
    };
    let text = format!("{file:?}");
    assert!(!text.contains("secretqqqq"), "{text}");
    assert!(
        text.contains("<redacted>") && text.contains("set"),
        "{text}"
    );
}

#[test]
fn parse_size_units() {
    for (input, want) in [
        ("100GB", 100 * (1u64 << 30)),
        ("512MB", 512 * (1u64 << 20)),
        ("1TB", 1u64 << 40),
        ("2KB", 2 * (1u64 << 10)),
        ("12345", 12345),
        ("100gb", 100 * (1u64 << 30)),
        ("100Gb", 100 * (1u64 << 30)),
        (" 100GB ", 100 * (1u64 << 30)),
        ("1.5KB", 1536),
        ("100GiB", 100 * (1u64 << 30)),
        ("512MiB", 512 * (1u64 << 20)),
        ("1TiB", 1u64 << 40),
        ("2KiB", 2 * (1u64 << 10)),
        ("1.5KiB", 1536),
        ("100gib", 100 * (1u64 << 30)),
        ("100 GIB", 100 * (1u64 << 30)),
    ] {
        assert_eq!(parse_size(input).unwrap(), want, "{input:?}");
    }
}

#[test]
fn parse_size_rejects_garbage() {
    for input in [
        "",
        "GB",
        "GiB",
        "5iB",
        "-5GB",
        "inf",
        "NaN",
        "1e3",
        "+5GB",
        "99999999999TB",
    ] {
        assert!(parse_size(input).is_err(), "{input:?}");
    }
}

#[test]
fn parse_duration_units() {
    assert_eq!(parse_duration_secs("10m").unwrap(), 600);
    assert_eq!(parse_duration_secs("2h").unwrap(), 7200);
    assert_eq!(parse_duration_secs("365d").unwrap(), 365 * 86_400);
    assert_eq!(parse_duration_secs("30s").unwrap(), 30);
    assert_eq!(parse_duration_secs("45").unwrap(), 45);
    assert_eq!(parse_duration_secs("2H").unwrap(), 7200);
}

#[test]
fn parse_duration_rejects_garbage() {
    assert!(parse_duration_secs("").is_err());
    assert!(parse_duration_secs("m").is_err());
    assert!(parse_duration_secs("-5m").is_err());
    assert!(parse_duration_secs("99999999999999999d").is_err());
}

#[test]
fn mfs_root_is_normalized_and_validated() {
    assert_eq!(parse_mfs_root("/swing").unwrap(), "/swing");
    assert_eq!(parse_mfs_root(" /a/b/ ").unwrap(), "/a/b");
    for bad in ["", "/", "swing", "/a//b", "/a/./b", "/a/../b"] {
        assert!(parse_mfs_root(bad).is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn missing_config_file_is_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.toml");
    let err = load_file(Some(&missing)).unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[cfg(unix)]
#[test]
fn a_secret_key_in_a_file_others_can_read_is_flagged() {
    let with_secret = parse_config_file("[nostr]\nsecret_key = \"k\"\n").unwrap();
    let without_secret = parse_config_file("[nostr]\nrelays = [\"wss://r\"]\n").unwrap();
    assert!(secret_readable_by_others(&with_secret, 0o100644));
    assert!(secret_readable_by_others(&with_secret, 0o100640));
    assert!(secret_readable_by_others(&with_secret, 0o100604));
    assert!(!secret_readable_by_others(&with_secret, 0o100600));
    assert!(!secret_readable_by_others(&with_secret, 0o100700));
    assert!(!secret_readable_by_others(&without_secret, 0o100644));
}

#[test]
fn secret_key_debug_is_redacted() {
    let key = NostrSecretKey::from("super-secret-nsec".to_string());
    assert_eq!(format!("{key:?}"), "<redacted>");
    assert_eq!(key.expose_secret(), "super-secret-nsec");
}

#[test]
fn config_load_remembers_the_config_file_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(&path, crate::test_support::MINIMAL_TOML).unwrap();
    let cfg = Config::load(Some(&path)).unwrap();
    assert_eq!(cfg.config_path, path);
    assert!(cfg.config_exists);
}

#[test]
fn relative_file_paths_resolve_against_the_config_file_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swing.toml");
    std::fs::write(
        &path,
        "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n[agent]\nstate_dir = \"./data\"\n[dashboard]\ncustom_css = \"theme/custom.css\"\n",
    )
    .unwrap();
    let cfg = Config::load(Some(&path)).unwrap();
    let base = std::path::absolute(dir.path()).unwrap();
    assert_eq!(cfg.agent.state_dir, base.join("data"));
    assert_eq!(cfg.kubo.repo, base.join("data").join("kubo"));
    assert_eq!(
        cfg.dashboard.custom_css,
        Some(base.join("theme").join("custom.css"))
    );
}

#[test]
fn is_valid_gateway_host_rules() {
    for good in ["example.com", "blog.example.net", "a.b-c.de", "localhost"] {
        assert!(is_valid_gateway_host(good), "{good:?} should be valid");
    }
    for bad in [
        "",
        ".example.com",
        "example.com.",
        "exa..mple.com",
        "EXAMPLE.com",
        "exa mple.com",
        "exa_mple.com",
        "example.com/path",
    ] {
        assert!(!is_valid_gateway_host(bad), "{bad:?} should be invalid");
    }
}

#[test]
fn parse_listen_off_and_addr() {
    assert_eq!(parse_listen("off").unwrap(), Listen::Off);
    assert_eq!(parse_listen("OFF").unwrap(), Listen::Off);
    assert_eq!(
        parse_listen("127.0.0.1:8081").unwrap(),
        Listen::Addr(([127, 0, 0, 1], 8081).into())
    );
    assert!(parse_listen("not-an-address").is_err());
}

fn env_of<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        vars.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_string())
    }
}

#[cfg(windows)]
const HOME_VARS: &[(&str, &str)] = &[("LOCALAPPDATA", r"C:\Users\u\AppData\Local")];
#[cfg(windows)]
const HOME_DIR: &str = r"C:\Users\u\AppData\Local";
#[cfg(not(windows))]
const HOME_VARS: &[(&str, &str)] = &[("HOME", "/home/u")];
#[cfg(not(windows))]
const HOME_DIR: &str = "/home/u";

fn expected_default() -> PathBuf {
    let base = PathBuf::from(HOME_DIR);
    if cfg!(windows) {
        base.join("swing").join("swing.toml")
    } else if cfg!(target_os = "macos") {
        base.join("Library/Application Support/swing/swing.toml")
    } else {
        base.join(".local/share/swing/swing.toml")
    }
}

fn cwd() -> PathBuf {
    PathBuf::from(if cfg!(windows) { r"D:\work" } else { "/work" })
}

#[test]
fn locate_config_uses_the_default_even_with_a_swing_toml_in_cwd() {
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(cwd.path().join("swing.toml"), "").unwrap();
    let home = tempfile::tempdir().unwrap();
    let home_str = home.path().to_str().unwrap();
    let vars = [(HOME_VARS[0].0, home_str)];
    let found = locate_config_with(None, cwd.path(), env_of(&vars), |p| p.is_dir());
    assert_eq!(found.1, ConfigOrigin::UserDefault);
    assert!(found.0.starts_with(home.path()));
    assert_ne!(found.0, cwd.path().join("swing.toml"));
    let found = locate_config_with(None, cwd.path(), env_of(HOME_VARS), |_| true);
    assert_eq!(found, (expected_default(), ConfigOrigin::UserDefault));
}

#[test]
fn locate_config_prefers_cli_and_env_over_the_default() {
    let cwd = cwd();
    let mut vars = HOME_VARS.to_vec();
    vars.push(("SWING_CONFIG", "/etc/from-env.toml"));
    let cli = PathBuf::from("/tmp/from-cli.toml");
    let found = locate_config_with(Some(&cli), &cwd, env_of(&vars), |_| true);
    assert_eq!(found, (cli.clone(), ConfigOrigin::Explicit));
    assert_eq!(resolve_config_path(Some(&cli)), cli);
    let found = locate_config_with(None, &cwd, env_of(&vars), |_| true);
    assert_eq!(
        found,
        (PathBuf::from("/etc/from-env.toml"), ConfigOrigin::Explicit)
    );
}

#[test]
fn locate_config_falls_back_to_cwd_without_a_usable_home() {
    let cwd = cwd();
    let in_cwd = cwd.join("swing.toml");
    let found = locate_config_with(None, &cwd, env_of(&[]), |_| true);
    assert_eq!(found, (in_cwd.clone(), ConfigOrigin::Cwd));
    let found = locate_config_with(None, &cwd, env_of(HOME_VARS), |_| false);
    assert_eq!(found, (in_cwd.clone(), ConfigOrigin::Cwd));
    let relative = [(HOME_VARS[0].0, "relative")];
    let found = locate_config_with(None, &cwd, env_of(&relative), |_| true);
    assert_eq!(found, (in_cwd, ConfigOrigin::Cwd));
}

#[cfg(not(any(windows, target_os = "macos")))]
#[test]
fn locate_config_honors_xdg_data_home() {
    let cwd = cwd();
    let vars = [("HOME", "/home/u"), ("XDG_DATA_HOME", "/srv/xdg")];
    let found = locate_config_with(None, &cwd, env_of(&vars), |_| false);
    assert_eq!(
        found,
        (
            PathBuf::from("/srv/xdg/swing/swing.toml"),
            ConfigOrigin::UserDefault
        )
    );
    let vars = [("HOME", "/home/u"), ("XDG_DATA_HOME", "xdg")];
    let found = locate_config_with(None, &cwd, env_of(&vars), |_| true);
    assert_eq!(found, (expected_default(), ConfigOrigin::UserDefault));
}

#[test]
fn public_url_accepts_scheme_and_authority_only() {
    assert_eq!(
        parse_public_url(" http://127.0.0.1:18082/ ").unwrap(),
        "http://127.0.0.1:18082"
    );
    assert_eq!(
        parse_public_url("https://swing.example").unwrap(),
        "https://swing.example"
    );
    for bad in [
        "127.0.0.1:8082",
        "http://",
        "ftp://x",
        "http://x/dash",
        "http://x?y",
        "http://x y",
        "http://a\\b",
        "http://a\tb",
        "http://a\u{7f}b",
        "http://a\nb:80",
        "http://x:port",
        "http://x:99999",
        "http://user@x",
        "http://x#frag",
        "http://x/?",
        "http://x:80",
        "https://X.example",
    ] {
        assert!(parse_public_url(bad).is_err(), "{bad:?}");
    }
    assert_eq!(
        parse_public_url("http://[::1]:5001/").unwrap(),
        "http://[::1]:5001"
    );
}
