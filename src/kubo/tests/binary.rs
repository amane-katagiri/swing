use super::*;

#[test]
fn kubo_commands_do_not_inherit_secret_settings() {
    let command = crate::kubo::binary::kubo_command(Path::new("ipfs"));
    let removed: Vec<_> = command
        .as_std()
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect();
    assert_eq!(removed, ["SWING_NOSTR_SECRET_KEY"]);
}

#[test]
fn find_on_path_skips_relative_entries() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("ipfs-test-exe");
    std::fs::write(&exe, b"").unwrap();
    let relative = std::env::join_paths([PathBuf::from("."), PathBuf::from("")]).unwrap();
    assert_eq!(
        crate::kubo::binary::find_on_path(&relative, "ipfs-test-exe"),
        None
    );
    let absolute = std::env::join_paths([PathBuf::from("."), dir.path().to_path_buf()]).unwrap();
    assert_eq!(
        crate::kubo::binary::find_on_path(&absolute, "ipfs-test-exe"),
        Some(exe)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_repo_creates_a_private_repo_dir() {
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_kubo(dir.path(), "exit 0", "mine");
    let repo = dir.path().join("custom/repo");
    assert!(ensure_repo(&bin, &repo).await.unwrap());
    assert_eq!(crate::test_support::mode(&repo), 0o700);
}

#[test]
fn locate_binary_errors_on_missing_explicit_path() {
    let missing = PathBuf::from("/nonexistent/path/to/ipfs-binary-that-does-not-exist");
    let err = locate_binary(Some(&missing)).unwrap_err().to_string();
    assert!(err.contains("Kubo binary not found"), "{err}");
}

#[test]
fn windows_installer_pins_the_same_kubo_version() {
    let pinned = include_str!("../../../packaging/windows/kubo.sha512");
    let fields: Vec<&str> = pinned.split_whitespace().collect();
    assert_eq!(fields.len(), 2, "{pinned}");
    assert_eq!(fields[0].len(), 128, "{pinned}");
    assert_eq!(fields[1], format!("kubo_v{KUBO_VERSION}_windows-amd64.zip"));
}

#[test]
fn install_sh_pins_the_same_kubo_version() {
    let script = include_str!("../../../packaging/linux/install.sh");
    let value = |name: &str| {
        script
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("{name} is not set in install.sh"))
    };
    assert_eq!(value("KUBO_VERSION"), KUBO_VERSION);
    for name in ["KUBO_SHA512_AMD64", "KUBO_SHA512_ARM64"] {
        let hash = value(name);
        assert!(
            hash.len() == 128 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "{name}={hash}"
        );
    }
}
