use super::*;

fn minimal_file() -> ConfigFile {
    ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn assert_env_rejects(env_key: &'static str, value: &'static str, expected_substring: &str) {
    let err = build_config(minimal_file(), None, move |k| {
        (k == env_key).then(|| value.to_string())
    })
    .unwrap_err();
    assert!(
        err.to_string().contains(expected_substring),
        "expected error containing {expected_substring:?}, got: {err}"
    );
}

fn file_with_paths(state_dir: &str) -> ConfigFile {
    let mut file = minimal_file();
    file.agent.state_dir = Some(state_dir.into());
    file.kubo.binary = Some("bin/ipfs".into());
    file.dashboard.mascots_dir = Some("mascots".into());
    file
}

fn base_dir() -> PathBuf {
    std::env::temp_dir().join("swing-config-dir")
}

#[test]
fn relative_file_paths_are_joined_to_the_base_dir() {
    let base = base_dir();
    let cfg = build_config(file_with_paths("./data"), Some(&base), |_| None).unwrap();
    assert_eq!(cfg.agent.state_dir, base.join("data"));
    assert_eq!(cfg.kubo.repo, base.join("data").join("kubo"));
    assert_eq!(cfg.kubo.binary, Some(base.join("bin").join("ipfs")));
    assert_eq!(cfg.dashboard.mascots_dir, Some(base.join("mascots")));
}

#[test]
fn env_relative_paths_are_not_joined_to_the_base_dir() {
    let base = base_dir();
    let cfg = build_config(file_with_paths("./data"), Some(&base), |k| match k {
        "SWING_STATE_DIR" => Some("./env-data".into()),
        "SWING_KUBO_BINARY" => Some("env/ipfs".into()),
        "SWING_DASHBOARD_MASCOTS_DIR" => Some("env-mascots".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.agent.state_dir, PathBuf::from("./env-data"));
    assert_eq!(cfg.kubo.repo, PathBuf::from("./env-data").join("kubo"));
    assert_eq!(cfg.kubo.binary, Some(PathBuf::from("env/ipfs")));
    assert_eq!(
        cfg.dashboard.mascots_dir,
        Some(PathBuf::from("env-mascots"))
    );
}

#[test]
fn absolute_file_paths_ignore_the_base_dir() {
    let elsewhere = std::env::temp_dir().join("swing-elsewhere");
    let mut file = file_with_paths(&elsewhere.join("state").to_string_lossy());
    file.kubo.repo = Some(elsewhere.join("kubo").to_string_lossy().into_owned());
    let cfg = build_config(file, Some(&base_dir()), |_| None).unwrap();
    assert_eq!(cfg.agent.state_dir, elsewhere.join("state"));
    assert_eq!(cfg.kubo.repo, elsewhere.join("kubo"));
}

#[test]
fn default_kubo_repo_follows_the_rebased_state_dir() {
    let base = base_dir();
    let cfg = build_config(file_with_paths("data"), Some(&base), |_| None).unwrap();
    assert_eq!(cfg.kubo.repo, base.join("data").join("kubo"));
    assert_eq!(cfg.source_of("kubo.repo"), Some(Source::Default));
}

#[test]
fn default_state_dir_is_joined_to_the_base_dir() {
    let base = base_dir();
    let cfg = build_config(minimal_file(), Some(&base), |_| None).unwrap();
    assert_eq!(cfg.agent.state_dir, base.join("data"));
    assert_eq!(cfg.kubo.repo, base.join("data").join("kubo"));
}

#[test]
fn relative_file_paths_stay_as_written_without_a_base_dir() {
    let cfg = build_config(file_with_paths("./data"), None, |_| None).unwrap();
    assert_eq!(cfg.agent.state_dir, PathBuf::from("./data"));
}

#[test]
fn zero_poll_interval_is_rejected() {
    assert_env_rejects(
        "SWING_POLL_INTERVAL",
        "0s",
        "SWING_POLL_INTERVAL must be greater than 0",
    );
}

#[test]
fn zero_concurrency_and_idle_timeout_are_rejected() {
    assert_env_rejects(
        "SWING_CONCURRENCY",
        "0",
        "SWING_CONCURRENCY must be greater than 0",
    );
    assert_env_rejects("SWING_FETCH_IDLE_TIMEOUT", "0", "SWING_FETCH_IDLE_TIMEOUT");
}

#[test]
fn report_ttl_must_outlast_two_polls() {
    let env = |ttl: &'static str| {
        move |k: &str| match k {
            "SWING_POLL_INTERVAL" => Some("10m".to_string()),
            "SWING_REPORT_TTL" => Some(ttl.to_string()),
            _ => None,
        }
    };
    let err = build_config(minimal_file(), None, env("20m")).unwrap_err();
    assert!(err.to_string().contains("report_ttl"));
    let cfg = build_config(minimal_file(), None, env("21m")).unwrap();
    assert_eq!(cfg.agent.report_ttl, Duration::from_secs(21 * 60));
}

#[test]
fn report_ttl_must_not_outlive_the_receivers_max_report_age() {
    let env = |ttl: &'static str| {
        move |k: &str| match k {
            "SWING_REPORT_TTL" => Some(ttl.to_string()),
            _ => None,
        }
    };
    let cfg = build_config(minimal_file(), None, env("7d")).unwrap();
    assert_eq!(
        cfg.agent.report_ttl,
        Duration::from_secs(crate::nostr::MAX_REPORT_AGE)
    );
    let err = build_config(minimal_file(), None, env("604801s")).unwrap_err();
    assert!(
        err.to_string().contains("report_ttl must be at most 7d"),
        "{err}"
    );
}

#[test]
fn zero_max_sites_per_account_is_rejected() {
    assert_env_rejects(
        "SWING_MAX_SITES_PER_ACCOUNT",
        "0",
        "SWING_MAX_SITES_PER_ACCOUNT must be greater than 0",
    );
}

#[test]
fn mfs_root_env_override_is_normalized() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_MFS_ROOT" => Some("/mirror/".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.ipfs.mfs_root, "/mirror");
}

#[test]
fn zero_publish_keep_versions_is_rejected() {
    assert_env_rejects(
        "SWING_PUBLISH_KEEP_VERSIONS",
        "0",
        "SWING_PUBLISH_KEEP_VERSIONS must be greater than 0",
    );
}

#[test]
fn zero_fetch_timeout_is_rejected() {
    assert_env_rejects("SWING_FETCH_TIMEOUT", "0", "SWING_FETCH_TIMEOUT");
}

#[test]
fn env_overrides_toml() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("file-key").into()),
            relays: Some(vec!["wss://from-file".into()]),
            mirror_set: Some("from-file-set".into()),
            site_event_kind: Some(1111),
            replica_event_kind: Some(2222),
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_NOSTR_SECRET_KEY" => Some("env-key".into()),
        "SWING_NOSTR_RELAYS" => Some("wss://a,wss://b".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.nostr.secret_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
    assert_eq!(cfg.nostr.relays, vec!["wss://a", "wss://b"]);
    assert_eq!(cfg.nostr.mirror_set, "from-file-set");
    assert_eq!(cfg.nostr.site_event_kind, 1111);
    assert_eq!(cfg.nostr.replica_event_kind, 2222);
}

#[test]
fn defaults_applied_when_nothing_set() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |_| None).unwrap();
    assert_eq!(cfg.nostr.mirror_set, "swing");
    assert_eq!(cfg.nostr.site_event_kind, 35980);
    assert_eq!(cfg.nostr.replica_event_kind, 35981);
    assert_eq!(cfg.ipfs.api, IpfsApi::Managed);
    assert_eq!(cfg.policy.max_total_storage, 100 * (1u64 << 30));
    assert_eq!(cfg.policy.max_per_site, 10 * (1u64 << 30));
    assert_eq!(cfg.policy.max_per_account, 20 * (1u64 << 30));
    assert_eq!(cfg.policy.max_sites_per_account, 10);
    assert_eq!(cfg.policy.max_update_size, 2 * (1u64 << 30));
    assert_eq!(cfg.policy.keep_versions, 5);
    assert_eq!(cfg.policy.keep_days, 365);
    assert_eq!(cfg.policy.min_update_interval, 3600);
    assert!(cfg.policy.remove_on_unfollow);
    assert_eq!(cfg.policy.nip05, CheckMode::Warn);
    assert_eq!(cfg.policy.nip05_cache_ttl, 86_400);
    assert_eq!(cfg.agent.poll_interval, Duration::from_secs(300));
    assert_eq!(cfg.agent.fetch_timeout, Duration::from_secs(900));
    assert_eq!(cfg.agent.fetch_idle_timeout, Duration::from_secs(120));
    assert_eq!(cfg.agent.concurrency, 4);
    assert_eq!(cfg.agent.report_ttl, Duration::from_secs(3 * 86_400));
    assert_eq!(cfg.ipfs.mfs_root, "/swing");
    assert_eq!(cfg.publish.keep_versions, 5);
    assert!(cfg.kubo.managed);
    assert_eq!(cfg.kubo.binary, None);
    assert_eq!(cfg.kubo.repo, PathBuf::from("./data").join("kubo"));
    assert_eq!(cfg.kubo.storage_max, 100 * (1u64 << 30));
    assert_eq!(cfg.kubo.provide_strategy, "pinned+mfs");
    assert_eq!(
        cfg.kubo.gateway_listen,
        SocketAddr::from(([127, 0, 0, 1], 8080))
    );
    assert_eq!(cfg.kubo.swarm_port, None);
    assert_eq!(cfg.gateway.listen, Listen::Off);
    assert!(cfg.gateway.hosts.is_empty());
    assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:8080");
}

#[test]
fn default_relays_used_when_none_configured() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |_| None).unwrap();
    assert_eq!(
        cfg.nostr.relays,
        DEFAULT_RELAYS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn nip05_mode_env_overrides_file() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        policy: PolicyFile {
            nip05: Some("require".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_NIP05" => Some("off".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.policy.nip05, CheckMode::Off);
}

#[test]
fn nip05_mode_rejects_garbage() {
    assert_env_rejects("SWING_NIP05", "maybe", "invalid SWING_NIP05");
}

#[test]
fn publish_nip05_defaults_to_warn() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |_| None).unwrap();
    assert_eq!(cfg.publish.nip05, CheckMode::Warn);
}

#[test]
fn publish_nip05_env_overrides_file() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        publish: PublishFile {
            nip05: Some("require".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_PUBLISH_NIP05" => Some("off".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.publish.nip05, CheckMode::Off);
}

#[test]
fn publish_checks_default_to_require_warn_require_with_the_default_allow_list() {
    let cfg = build_config(minimal_file(), None, |_| None).unwrap();
    assert_eq!(cfg.publish.check_dotfiles, CheckMode::Require);
    assert_eq!(cfg.publish.check_size, CheckMode::Warn);
    assert_eq!(cfg.publish.check_unchanged, CheckMode::Require);
    assert_eq!(cfg.publish.dotfiles_allow, DEFAULT_DOTFILES_ALLOW);
    assert_eq!(
        cfg.source_of("publish.dotfiles_allow"),
        Some(Source::Default)
    );
}

#[test]
fn publish_check_env_overrides_file_which_overrides_default() {
    let mut file = minimal_file();
    file.publish = PublishFile {
        check_dotfiles: Some("warn".into()),
        check_size: Some("require".into()),
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_PUBLISH_CHECK_DOTFILES" => Some("off".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.publish.check_dotfiles, CheckMode::Off);
    assert_eq!(cfg.source_of("publish.check_dotfiles"), Some(Source::Env));
    assert_eq!(cfg.publish.check_size, CheckMode::Require);
    assert_eq!(cfg.source_of("publish.check_size"), Some(Source::File));
    assert_eq!(cfg.publish.check_unchanged, CheckMode::Require);
    assert_eq!(
        cfg.source_of("publish.check_unchanged"),
        Some(Source::Default)
    );
}

#[test]
fn publish_check_modes_reject_garbage() {
    assert_env_rejects(
        "SWING_PUBLISH_CHECK_SIZE",
        "maybe",
        "invalid SWING_PUBLISH_CHECK_SIZE",
    );
    let mut file = minimal_file();
    file.publish.check_unchanged = Some("sometimes".into());
    let err = build_config(file, None, |_| None).unwrap_err();
    assert!(
        err.to_string().contains("[publish].check_unchanged"),
        "{err}"
    );
}

#[test]
fn dotfiles_allow_replaces_the_default_list() {
    let cfg = build_config(minimal_file(), None, |k| {
        (k == "SWING_PUBLISH_DOTFILES_ALLOW").then(|| " .htaccess , .nojekyll ".to_string())
    })
    .unwrap();
    assert_eq!(cfg.publish.dotfiles_allow, vec![".htaccess", ".nojekyll"]);

    let mut file = minimal_file();
    file.publish.dotfiles_allow = Some(Vec::new());
    let cfg = build_config(file, None, |_| None).unwrap();
    assert!(cfg.publish.dotfiles_allow.is_empty());
    assert_eq!(cfg.source_of("publish.dotfiles_allow"), Some(Source::File));
}

#[test]
fn dotfiles_allow_rejects_entries_that_are_not_single_dot_names() {
    for bad in ["nojekyll", ".well-known/nostr.json", ".", ".."] {
        let mut file = minimal_file();
        file.publish.dotfiles_allow = Some(vec![bad.to_string()]);
        let err = build_config(file, None, |_| None).unwrap_err();
        assert!(
            format!("{err:#}").contains("invalid dotfile name"),
            "{bad}: {err:#}"
        );
    }
}

#[test]
fn publish_nip05_rejects_garbage() {
    assert_env_rejects(
        "SWING_PUBLISH_NIP05",
        "maybe",
        "invalid SWING_PUBLISH_NIP05",
    );
}

#[test]
fn dashboard_defaults_to_localhost_8082_with_default_gateway() {
    let cfg = build_config(minimal_file(), None, |_| None).unwrap();
    assert_eq!(
        cfg.dashboard.listen,
        SocketAddr::from(([127, 0, 0, 1], 8082))
    );
    assert!(cfg.dashboard.ui);
    assert!(cfg.dashboard.allowed_hosts.is_empty());
    assert_eq!(
        cfg.dashboard.gateway.as_deref(),
        Some("http://localhost:8080")
    );
    assert_eq!(cfg.dashboard.custom_css, None);
    assert_eq!(cfg.dashboard.desktop_page, None);
    assert_eq!(cfg.dashboard.desktop_page_css, None);
    assert_eq!(cfg.dashboard.desktop_banner, None);
    assert_eq!(cfg.dashboard.mascots_dir, None);
    assert_eq!(cfg.dashboard.max_upload, 2 * (1u64 << 30));
}

#[test]
fn dashboard_max_upload_env_overrides_file() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        dashboard: DashboardFile {
            max_upload: Some("4GB".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_DASHBOARD_MAX_UPLOAD" => Some("512MB".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.dashboard.max_upload, 512 * (1u64 << 20));
}

#[test]
fn dashboard_max_upload_zero_is_rejected() {
    assert_env_rejects(
        "SWING_DASHBOARD_MAX_UPLOAD",
        "0",
        "SWING_DASHBOARD_MAX_UPLOAD must be greater than 0",
    );
}

#[test]
fn dashboard_listen_off_is_not_a_valid_address() {
    assert_env_rejects("SWING_DASHBOARD_LISTEN", "off", "SWING_DASHBOARD_LISTEN");
}

#[test]
fn dashboard_listen_rejects_garbage() {
    assert_env_rejects(
        "SWING_DASHBOARD_LISTEN",
        "not-an-address",
        "SWING_DASHBOARD_LISTEN",
    );
}

#[test]
fn dashboard_ui_defaults_to_true_and_can_be_disabled() {
    let cfg = build_config(minimal_file(), None, |_| None).unwrap();
    assert!(cfg.dashboard.ui);

    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_DASHBOARD_UI" => Some("false".into()),
        _ => None,
    })
    .unwrap();
    assert!(!cfg.dashboard.ui);
}

#[test]
fn dashboard_ui_file_value_is_used_when_env_unset() {
    let file = ConfigFile {
        dashboard: DashboardFile {
            ui: Some(false),
            ..Default::default()
        },
        ..minimal_file()
    };
    let cfg = build_config(file, None, |_| None).unwrap();
    assert!(!cfg.dashboard.ui);
}

#[test]
fn dashboard_env_overrides_file() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        dashboard: DashboardFile {
            listen: Some("127.0.0.1:9000".into()),
            ui: Some(false),
            allowed_hosts: Some(vec!["example.com".into()]),
            public_url: None,
            gateway: Some("http://gateway.example".into()),
            custom_css: Some("/etc/swing/custom.css".into()),
            desktop_page: Some("/etc/swing/page.html".into()),
            desktop_page_css: Some("/etc/swing/page.css".into()),
            desktop_banner: Some("/etc/swing/banner.png".into()),
            mascots_dir: Some("/etc/swing/mascots".into()),
            max_upload: Some("4GB".into()),
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_DASHBOARD_LISTEN" => Some("0.0.0.0:8082".into()),
        "SWING_DASHBOARD_ALLOWED_HOSTS" => Some("a.example, b.example".into()),
        "SWING_DASHBOARD_GATEWAY" => Some("http://env-gateway.example".into()),
        "SWING_DASHBOARD_CUSTOM_CSS" => Some("/env/custom.css".into()),
        "SWING_DASHBOARD_DESKTOP_PAGE" => Some("/env/page.html".into()),
        "SWING_DASHBOARD_DESKTOP_PAGE_CSS" => Some("/env/page.css".into()),
        "SWING_DASHBOARD_DESKTOP_BANNER" => Some("/env/banner.gif".into()),
        "SWING_DASHBOARD_MASCOTS_DIR" => Some("/env/mascots".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.dashboard.listen, SocketAddr::from(([0, 0, 0, 0], 8082)));
    assert!(!cfg.dashboard.ui);
    assert_eq!(
        cfg.dashboard.allowed_hosts,
        vec!["a.example".to_string(), "b.example".to_string()]
    );
    assert_eq!(
        cfg.dashboard.gateway.as_deref(),
        Some("http://env-gateway.example")
    );
    assert_eq!(
        cfg.dashboard.custom_css,
        Some(PathBuf::from("/env/custom.css"))
    );
    assert_eq!(
        cfg.dashboard.desktop_page,
        Some(PathBuf::from("/env/page.html"))
    );
    assert_eq!(
        cfg.dashboard.desktop_page_css,
        Some(PathBuf::from("/env/page.css"))
    );
    assert_eq!(
        cfg.dashboard.desktop_banner,
        Some(PathBuf::from("/env/banner.gif"))
    );
    assert_eq!(
        cfg.dashboard.mascots_dir,
        Some(PathBuf::from("/env/mascots"))
    );
}

#[test]
fn dashboard_gateway_empty_string_in_file_disables_links() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        dashboard: DashboardFile {
            gateway: Some(String::new()),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |_| None).unwrap();
    assert_eq!(cfg.dashboard.gateway, None);
}

#[test]
fn max_total_storage_env_is_a_size_string() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            relays: Some(vec!["wss://r".into()]),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_MAX_TOTAL_STORAGE" => Some("20GB".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.policy.max_total_storage, 20 * (1u64 << 30));
}

#[test]
fn managed_kubo_rejects_explicit_ipfs_api_from_file() {
    let file = ConfigFile {
        ipfs: IpfsFile {
            api: Some("http://127.0.0.1:5001".into()),
            ..Default::default()
        },
        ..minimal_file()
    };
    let err = build_config(file, None, |_| None).unwrap_err();
    assert!(err.to_string().contains("[ipfs].api conflicts"));
}

#[test]
fn managed_kubo_rejects_explicit_ipfs_api_from_env() {
    assert_env_rejects(
        "SWING_IPFS_API",
        "http://127.0.0.1:5001",
        "SWING_IPFS_API conflicts with [kubo].managed",
    );
}

#[test]
fn unmanaged_kubo_uses_ipfs_api() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_MANAGED" => Some("false".into()),
        "SWING_IPFS_API" => Some("http://127.0.0.1:15001".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.ipfs.api,
        IpfsApi::Url("http://127.0.0.1:15001".to_string())
    );
    assert!(!cfg.kubo.managed);
}

#[test]
fn unmanaged_kubo_defaults_ipfs_api() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_MANAGED" => Some("false".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.ipfs.api,
        IpfsApi::Url("http://127.0.0.1:5001".to_string())
    );
}

#[test]
fn kubo_repo_defaults_under_state_dir() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_STATE_DIR" => Some("/var/lib/swing".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.kubo.repo, PathBuf::from("/var/lib/swing/kubo"));
}

#[test]
fn kubo_repo_env_overrides_default() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_REPO" => Some("/data/kubo-repo".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.kubo.repo, PathBuf::from("/data/kubo-repo"));
}

#[test]
fn kubo_binary_env_overrides_file() {
    let file = ConfigFile {
        kubo: KuboFile {
            binary: Some("/opt/kubo/ipfs".into()),
            ..Default::default()
        },
        ..minimal_file()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_KUBO_BINARY" => Some("/usr/local/bin/ipfs".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.kubo.binary, Some(PathBuf::from("/usr/local/bin/ipfs")));
}

#[test]
fn kubo_storage_max_defaults_to_max_total_storage() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_MAX_TOTAL_STORAGE" => Some("50GB".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.kubo.storage_max, 50 * (1u64 << 30));
}

#[test]
fn kubo_storage_max_can_differ_from_max_total_storage() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_MAX_TOTAL_STORAGE" => Some("50GB".into()),
        "SWING_KUBO_STORAGE_MAX" => Some("80GB".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.policy.max_total_storage, 50 * (1u64 << 30));
    assert_eq!(cfg.kubo.storage_max, 80 * (1u64 << 30));
}

#[test]
fn kubo_provide_strategy_empty_is_rejected() {
    let file = ConfigFile {
        kubo: KuboFile {
            provide_strategy: Some(String::new()),
            ..Default::default()
        },
        ..minimal_file()
    };
    let err = build_config(file, None, |_| None).unwrap_err();
    assert!(err.to_string().contains("provide_strategy"));
}

#[test]
fn kubo_gateway_listen_env_overrides_file() {
    let file = ConfigFile {
        kubo: KuboFile {
            gateway_listen: Some("127.0.0.1:9090".into()),
            ..Default::default()
        },
        ..minimal_file()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:8181".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.kubo.gateway_listen,
        SocketAddr::from(([127, 0, 0, 1], 8181))
    );
}

#[test]
fn kubo_gateway_listen_rejects_garbage() {
    assert_env_rejects(
        "SWING_KUBO_GATEWAY_LISTEN",
        "not-an-address",
        "SWING_KUBO_GATEWAY_LISTEN",
    );
}

#[test]
fn kubo_swarm_port_defaults_to_unset() {
    let cfg = build_config(minimal_file(), None, |_| None).unwrap();
    assert_eq!(cfg.kubo.swarm_port, None);
}

#[test]
fn kubo_swarm_port_zero_is_rejected() {
    assert_env_rejects(
        "SWING_KUBO_SWARM_PORT",
        "0",
        "SWING_KUBO_SWARM_PORT must be between",
    );
}

#[test]
fn kubo_swarm_port_in_range_is_accepted() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_SWARM_PORT" => Some("4001".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.kubo.swarm_port, Some(4001));
}

#[test]
fn gateway_listen_enabled_requires_hosts() {
    assert_env_rejects("SWING_GATEWAY_LISTEN", "127.0.0.1:8081", "[gateway].hosts");
}

#[test]
fn gateway_listen_enabled_with_hosts_is_accepted() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_GATEWAY_LISTEN" => Some("127.0.0.1:8081".into()),
        "SWING_GATEWAY_HOSTS" => Some("example.com".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.gateway.listen,
        Listen::Addr(([127, 0, 0, 1], 8081).into())
    );
    assert_eq!(cfg.gateway.hosts, vec!["example.com".to_string()]);
}

#[test]
fn gateway_hosts_must_not_be_the_dashboard_listen_ip() {
    let build = |listen: &str| {
        let listen = listen.to_string();
        build_config(minimal_file(), None, move |k| match k {
            "SWING_GATEWAY_HOSTS" => Some("192.168.1.5".into()),
            "SWING_DASHBOARD_LISTEN" => Some(listen.clone()),
            _ => None,
        })
    };
    let err = build("192.168.1.5:8082").unwrap_err();
    assert!(err.to_string().contains("also a dashboard host"), "{err}");
    assert!(build("192.168.1.6:8082").is_ok());
    assert!(build("0.0.0.0:8082").is_ok());
}

#[test]
fn gateway_hosts_must_not_overlap_dashboard_hosts() {
    let err = build_config(minimal_file(), None, |k| match k {
        "SWING_GATEWAY_HOSTS" => Some("example.com,dash.example".into()),
        "SWING_DASHBOARD_ALLOWED_HOSTS" => Some("Dash.Example".into()),
        _ => None,
    })
    .unwrap_err();
    assert!(err.to_string().contains("dash.example"), "{err}");
    assert_env_rejects("SWING_GATEWAY_HOSTS", "localhost", "also a dashboard host");
    assert_env_rejects("SWING_GATEWAY_HOSTS", "127.0.0.1", "also a dashboard host");
    assert!(
        build_config(minimal_file(), None, |k| match k {
            "SWING_GATEWAY_HOSTS" => Some("example.com".into()),
            "SWING_DASHBOARD_ALLOWED_HOSTS" => Some("dash.example".into()),
            _ => None,
        })
        .is_ok()
    );
}

#[test]
fn dashboard_gateway_must_be_an_http_origin() {
    assert_env_rejects(
        "SWING_DASHBOARD_GATEWAY",
        "javascript:alert(1)",
        "invalid SWING_DASHBOARD_GATEWAY",
    );
    assert_env_rejects(
        "SWING_DASHBOARD_GATEWAY",
        "http://gw.example/sub",
        "invalid SWING_DASHBOARD_GATEWAY",
    );
    let cfg = build_config(minimal_file(), None, |k| {
        (k == "SWING_DASHBOARD_GATEWAY").then(|| "https://gw.example/".to_string())
    })
    .unwrap();
    assert_eq!(cfg.dashboard.gateway.as_deref(), Some("https://gw.example"));
    let cfg = build_config(minimal_file(), None, |k| {
        (k == "SWING_DASHBOARD_GATEWAY").then(String::new)
    })
    .unwrap();
    assert_eq!(cfg.dashboard.gateway, None);
}

#[test]
fn gateway_hosts_rejects_invalid_hostnames() {
    assert_env_rejects(
        "SWING_GATEWAY_HOSTS",
        "Example.com",
        "invalid SWING_GATEWAY_HOSTS entry",
    );
}

#[test]
fn gateway_hosts_env_overrides_file_and_trims_entries() {
    let file = ConfigFile {
        gateway: GatewayFile {
            hosts: Some(vec!["from-file.example".into()]),
            ..Default::default()
        },
        ..minimal_file()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_GATEWAY_HOSTS" => Some(" a.example , b.example ".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.gateway.hosts,
        vec!["a.example".to_string(), "b.example".to_string()]
    );
}

#[test]
fn gateway_upstream_defaults_to_managed_kubo_gateway_listen() {
    let cfg = build_config(minimal_file(), None, |_| None).unwrap();
    assert!(cfg.kubo.managed);
    assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:8080");

    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:9999".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:9999");
}

#[test]
fn gateway_upstream_defaults_to_localhost_when_unmanaged() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_MANAGED" => Some("false".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:8080");
}

#[test]
fn gateway_upstream_env_overrides_default() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_GATEWAY_UPSTREAM" => Some("http://ipfs:8080".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.gateway.upstream, "http://ipfs:8080");
}

#[test]
fn source_tracking_distinguishes_env_file_and_default() {
    let file = ConfigFile {
        policy: PolicyFile {
            max_per_site: Some("5GB".into()),
            ..Default::default()
        },
        ..minimal_file()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_MAX_TOTAL_STORAGE" => Some("10GB".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.source_of("policy.max_total_storage"), Some(Source::Env));
    assert_eq!(cfg.source_of("policy.max_per_site"), Some(Source::File));
    assert_eq!(
        cfg.source_of("policy.max_per_account"),
        Some(Source::Default)
    );
    assert_eq!(cfg.source_of("kubo.storage_max"), Some(Source::Default));
}

#[test]
fn source_tracking_covers_secret_key_and_managed_ipfs_api() {
    let cfg = build_config(minimal_file(), None, |_| None).unwrap();
    assert_eq!(cfg.source_of("nostr.secret_key"), Some(Source::File));
    assert_eq!(cfg.source_of("ipfs.api"), Some(Source::Default));

    let cfg = build_config(ConfigFile::default(), None, |_| None).unwrap();
    assert_eq!(cfg.source_of("nostr.secret_key"), Some(Source::Default));
}

#[test]
fn public_url_comes_from_env_or_file() {
    let config = build_config_from_str(
        "[dashboard]\npublic_url = \"http://127.0.0.1:18082/\"\n",
        |_| None,
    )
    .unwrap();
    assert_eq!(
        config.dashboard.public_url.as_deref(),
        Some("http://127.0.0.1:18082")
    );
    let config = build_config_from_str("", |k| {
        (k == "SWING_DASHBOARD_PUBLIC_URL").then(|| "http://localhost:9000".to_string())
    })
    .unwrap();
    assert_eq!(
        config.dashboard.public_url.as_deref(),
        Some("http://localhost:9000")
    );
    assert!(
        build_config_from_str("", |_| None)
            .unwrap()
            .dashboard
            .public_url
            .is_none()
    );
    assert!(
        build_config_from_str("[dashboard]\npublic_url = \"http://x/sub\"\n", |_| None).is_err()
    );
}

#[test]
fn errors_name_the_source_the_value_came_from() {
    let mut file = minimal_file();
    file.agent.fetch_timeout = Some("0".into());
    let err = build_config(file, None, |_| None).unwrap_err();
    assert_eq!(
        err.to_string(),
        "[agent].fetch_timeout must be greater than 0"
    );

    let mut file = minimal_file();
    file.kubo.swarm_port = Some(0);
    let err = build_config(file, None, |_| None).unwrap_err();
    assert_eq!(
        err.to_string(),
        "[kubo].swarm_port must be between 1 and 65535"
    );

    let mut file = minimal_file();
    file.policy.max_per_site = Some("lots".into());
    let err = build_config(file.clone(), None, |_| None).unwrap_err();
    assert_eq!(err.to_string(), "invalid [policy].max_per_site");
    let err = build_config(file, None, |k| {
        (k == "SWING_MAX_PER_SITE").then(|| "many".to_string())
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "invalid SWING_MAX_PER_SITE");

    let err = build_config(minimal_file(), None, |k| {
        (k == "SWING_KEEP_DAYS").then(|| "x".to_string())
    })
    .unwrap_err();
    assert!(
        format!("{err:#}").starts_with("invalid SWING_KEEP_DAYS: expected integer: "),
        "{err:#}"
    );
}

#[test]
fn dashboard_gateway_default_follows_the_managed_kubo_gateway_port() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:8081".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.dashboard.gateway.as_deref(),
        Some("http://localhost:8081")
    );

    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:8081".into()),
        "SWING_KUBO_MANAGED" => Some("false".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.dashboard.gateway.as_deref(),
        Some("http://localhost:8080")
    );

    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:8081".into()),
        "SWING_DASHBOARD_GATEWAY" => Some("https://gw.example".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.dashboard.gateway.as_deref(), Some("https://gw.example"));
}

#[test]
fn ipfs_api_and_gateway_upstream_must_be_http_origins() {
    for bad in [
        "127.0.0.1:5001",
        "ftp://ipfs:5001",
        "http://ipfs:5001/api",
        "http://",
    ] {
        let err = build_config(minimal_file(), None, |k| match k {
            "SWING_KUBO_MANAGED" => Some("false".into()),
            "SWING_IPFS_API" => Some(bad.into()),
            _ => None,
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("invalid SWING_IPFS_API"),
            "{bad}: {err}"
        );
        assert_env_rejects(
            "SWING_GATEWAY_UPSTREAM",
            bad,
            "invalid SWING_GATEWAY_UPSTREAM",
        );
    }
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_MANAGED" => Some("false".into()),
        "SWING_IPFS_API" => Some("https://ipfs.example:5001/".into()),
        "SWING_GATEWAY_UPSTREAM" => Some("http://[::1]:8080".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        cfg.ipfs.api,
        IpfsApi::Url("https://ipfs.example:5001".into())
    );
    assert_eq!(cfg.gateway.upstream, "http://[::1]:8080");
}

#[test]
fn relays_must_be_websocket_urls() {
    let mut file = minimal_file();
    file.nostr.relays = Some(vec!["wss://ok.example".into(), "wss//typo".into()]);
    let err = build_config(file, None, |_| None).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("invalid [nostr].relays entry wss//typo"),
        "{err}"
    );

    let mut file = minimal_file();
    file.nostr.relays = Some(vec!["https://relay.example".into()]);
    assert!(build_config(file, None, |_| None).is_err());

    let err = build_config(minimal_file(), None, |k| {
        (k == "SWING_NOSTR_RELAYS").then(|| "wss://a,not a url".to_string())
    })
    .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("invalid SWING_NOSTR_RELAYS entry"),
        "{err}"
    );

    let mut file = minimal_file();
    file.nostr.relays = Some(vec![
        "ws://127.0.0.1:7777".into(),
        "wss://r.example/path".into(),
    ]);
    assert!(build_config(file, None, |_| None).is_ok());
}

#[test]
fn mirror_set_must_be_a_usable_d_tag() {
    let mut file = minimal_file();
    file.nostr.mirror_set = Some(String::new());
    let err = build_config(file, None, |_| None).unwrap_err();
    assert_eq!(err.to_string(), "invalid [nostr].mirror_set");

    let mut file = minimal_file();
    file.nostr.mirror_set = Some("set\u{202E}".into());
    assert!(build_config(file, None, |_| None).is_err());

    let err = build_config(minimal_file(), None, |k| {
        (k == "SWING_MIRROR_SET").then(String::new)
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "invalid SWING_MIRROR_SET");
}
