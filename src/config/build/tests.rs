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
fn kubo_repo_follows_the_state_dir_unless_set() {
    let base = base_dir();
    let cases = [
        (
            "file state_dir rebased",
            file_with_paths("data"),
            Some(base.as_path()),
            vec![],
            base.join("data"),
            base.join("data").join("kubo"),
        ),
        (
            "default state_dir rebased",
            minimal_file(),
            Some(base.as_path()),
            vec![],
            base.join("data"),
            base.join("data").join("kubo"),
        ),
        (
            "env state_dir",
            minimal_file(),
            None,
            vec![("SWING_STATE_DIR", "/var/lib/swing")],
            PathBuf::from("/var/lib/swing"),
            PathBuf::from("/var/lib/swing/kubo"),
        ),
        (
            "env repo",
            minimal_file(),
            None,
            vec![("SWING_KUBO_REPO", "/data/kubo-repo")],
            PathBuf::from("./data"),
            PathBuf::from("/data/kubo-repo"),
        ),
    ];
    for (name, file, base, env, state_dir, repo) in cases {
        let cfg = build_config(file, base, |k| {
            env.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        })
        .unwrap();
        assert_eq!(cfg.agent.state_dir, state_dir, "{name}");
        assert_eq!(cfg.kubo.repo, repo, "{name}");
        let source = if env.iter().any(|(k, _)| *k == "SWING_KUBO_REPO") {
            Source::Env
        } else {
            Source::Default
        };
        assert_eq!(cfg.source_of("kubo.repo"), Some(source), "{name}");
    }
}

#[test]
fn relative_file_paths_stay_as_written_without_a_base_dir() {
    let cfg = build_config(file_with_paths("./data"), None, |_| None).unwrap();
    assert_eq!(cfg.agent.state_dir, PathBuf::from("./data"));
}

#[test]
fn out_of_range_numbers_are_rejected() {
    for (key, value, expected) in [
        ("SWING_POLL_INTERVAL", "0s", "must be greater than 0"),
        ("SWING_FETCH_TIMEOUT", "0", "must be greater than 0"),
        ("SWING_FETCH_IDLE_TIMEOUT", "0", "must be greater than 0"),
        ("SWING_CONCURRENCY", "0", "must be greater than 0"),
        ("SWING_MAX_SITES_PER_ACCOUNT", "0", "must be greater than 0"),
        ("SWING_PUBLISH_KEEP_VERSIONS", "0", "must be greater than 0"),
        ("SWING_DASHBOARD_MAX_UPLOAD", "0", "must be greater than 0"),
        ("SWING_KUBO_SWARM_PORT", "0", "must be between 1 and 65535"),
        ("SWING_ANCHOR_MAX_PEERS", "17", "must be between 0 and 16"),
    ] {
        assert_env_rejects(key, value, &format!("{key} {expected}"));
    }
    let cfg = build_config(minimal_file(), None, |k| {
        (k == "SWING_KUBO_SWARM_PORT").then(|| "4001".to_string())
    })
    .unwrap();
    assert_eq!(cfg.kubo.swarm_port, Some(4001));
}

#[test]
fn anchor_max_peers_is_limited() {
    let mut file = minimal_file();
    file.anchor.max_peers = Some(17);
    let err = build_config(file, None, |_| None).unwrap_err();
    assert!(err.to_string().contains("[anchor].max_peers"), "{err}");
    for ok in [0, 16] {
        let mut file = minimal_file();
        file.anchor.max_peers = Some(ok);
        assert_eq!(
            build_config(file, None, |_| None).unwrap().anchor.max_peers,
            ok
        );
    }
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
fn mfs_root_env_override_is_normalized() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_MFS_ROOT" => Some("/mirror/".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.ipfs.mfs_root, "/mirror");
}

#[test]
fn env_overrides_file_for_each_resolver_helper() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("file-key").into()),
            relays: Some(vec!["wss://from-file".into()]),
            mirror_set: Some("from-file-set".into()),
            site_event_kind: Some(1111),
            replica_event_kind: Some(2222),
        },
        policy: PolicyFile {
            nip05: Some("require".into()),
            ..Default::default()
        },
        publish: PublishFile {
            nip05: Some("require".into()),
            ..Default::default()
        },
        kubo: KuboFile {
            binary: Some("/opt/kubo/ipfs".into()),
            gateway_listen: Some("127.0.0.1:9090".into()),
            ..Default::default()
        },
        gateway: GatewayFile {
            hosts: Some(vec!["from-file.example".into()]),
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
        "SWING_NOSTR_SECRET_KEY" => Some("env-key".into()),
        "SWING_NOSTR_RELAYS" => Some("wss://a,wss://b".into()),
        "SWING_NIP05" => Some("off".into()),
        "SWING_PUBLISH_NIP05" => Some("off".into()),
        "SWING_KUBO_BINARY" => Some("/usr/local/bin/ipfs".into()),
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:8181".into()),
        "SWING_GATEWAY_HOSTS" => Some(" a.example , b.example ".into()),
        "SWING_DASHBOARD_LISTEN" => Some("0.0.0.0:8082".into()),
        "SWING_DASHBOARD_ALLOWED_HOSTS" => Some("a.dash.example, b.dash.example".into()),
        "SWING_DASHBOARD_GATEWAY" => Some("http://env-gateway.example".into()),
        "SWING_DASHBOARD_CUSTOM_CSS" => Some("/env/custom.css".into()),
        "SWING_DASHBOARD_DESKTOP_PAGE" => Some("/env/page.html".into()),
        "SWING_DASHBOARD_DESKTOP_PAGE_CSS" => Some("/env/page.css".into()),
        "SWING_DASHBOARD_DESKTOP_BANNER" => Some("/env/banner.gif".into()),
        "SWING_DASHBOARD_MASCOTS_DIR" => Some("/env/mascots".into()),
        "SWING_DASHBOARD_MAX_UPLOAD" => Some("512MB".into()),
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
    assert_eq!(cfg.policy.nip05, CheckMode::Off);
    assert_eq!(cfg.publish.nip05, CheckMode::Off);
    assert_eq!(cfg.kubo.binary, Some(PathBuf::from("/usr/local/bin/ipfs")));
    assert_eq!(
        cfg.kubo.gateway_listen,
        SocketAddr::from(([127, 0, 0, 1], 8181))
    );
    assert_eq!(cfg.gateway.hosts, ["a.example", "b.example"]);
    assert_eq!(cfg.dashboard.listen, SocketAddr::from(([0, 0, 0, 0], 8082)));
    assert!(!cfg.dashboard.ui);
    assert_eq!(
        cfg.dashboard.allowed_hosts,
        ["a.dash.example", "b.dash.example"]
    );
    assert_eq!(
        cfg.dashboard.gateway.as_deref(),
        Some("http://env-gateway.example")
    );
    for (name, got, want) in [
        ("custom_css", &cfg.dashboard.custom_css, "/env/custom.css"),
        (
            "desktop_page",
            &cfg.dashboard.desktop_page,
            "/env/page.html",
        ),
        (
            "desktop_page_css",
            &cfg.dashboard.desktop_page_css,
            "/env/page.css",
        ),
        (
            "desktop_banner",
            &cfg.dashboard.desktop_banner,
            "/env/banner.gif",
        ),
        ("mascots_dir", &cfg.dashboard.mascots_dir, "/env/mascots"),
    ] {
        assert_eq!(got.as_deref(), Some(Path::new(want)), "{name}");
    }
    assert_eq!(cfg.dashboard.max_upload, 512 * (1u64 << 20));
}

#[test]
fn every_catalog_example_value_is_read_from_the_file() {
    for setting in crate::settings::SETTINGS {
        let crate::settings::Example::Value(literal) = setting.example else {
            continue;
        };
        let text = format!("[{}]\n{} = {literal}\n", setting.section, setting.field);
        let cfg = build_config_from_str(&text, |_| None)
            .unwrap_or_else(|e| panic!("{}: {e:#}", setting.key));
        assert_eq!(
            cfg.source_of(setting.key),
            Some(Source::File),
            "{}",
            setting.key
        );
    }
}

#[test]
fn defaults_applied_when_nothing_set() {
    let file = ConfigFile {
        nostr: NostrFile {
            secret_key: Some(String::from("k").into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let cfg = build_config(file, None, |_| None).unwrap();
    assert_eq!(cfg.nostr.relays, DEFAULT_RELAYS);
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
    assert!(cfg.anchor.advertise);
    assert_eq!(cfg.anchor.max_peers, 4);
    assert_eq!(cfg.ipfs.mfs_root, "/swing");
    assert_eq!(cfg.publish.keep_versions, 5);
    assert_eq!(cfg.publish.nip05, CheckMode::Warn);
    assert_eq!(cfg.publish.check_dotfiles, CheckMode::Require);
    assert_eq!(cfg.publish.check_size, CheckMode::Warn);
    assert_eq!(cfg.publish.check_links, CheckMode::Warn);
    assert_eq!(cfg.publish.check_unchanged, CheckMode::Require);
    assert_eq!(cfg.publish.dotfiles_allow, DEFAULT_DOTFILES_ALLOW);
    assert_eq!(
        cfg.source_of("publish.dotfiles_allow"),
        Some(Source::Default)
    );
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
    assert_eq!(
        cfg.dashboard.listen,
        SocketAddr::from(([127, 0, 0, 1], 8082))
    );
    assert!(cfg.dashboard.ui);
    assert!(cfg.dashboard.allowed_hosts.is_empty());
    assert_eq!(cfg.dashboard.public_url, None);
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
fn garbage_env_values_are_rejected() {
    for (key, value, expected) in [
        ("SWING_NIP05", "maybe", "invalid SWING_NIP05"),
        (
            "SWING_PUBLISH_NIP05",
            "maybe",
            "invalid SWING_PUBLISH_NIP05",
        ),
        (
            "SWING_PUBLISH_CHECK_SIZE",
            "maybe",
            "invalid SWING_PUBLISH_CHECK_SIZE",
        ),
        (
            "SWING_DASHBOARD_LISTEN",
            "off",
            "invalid SWING_DASHBOARD_LISTEN",
        ),
        (
            "SWING_DASHBOARD_LISTEN",
            "not-an-address",
            "invalid SWING_DASHBOARD_LISTEN",
        ),
        (
            "SWING_KUBO_GATEWAY_LISTEN",
            "not-an-address",
            "invalid SWING_KUBO_GATEWAY_LISTEN",
        ),
        (
            "SWING_GATEWAY_HOSTS",
            "Example.com",
            "invalid SWING_GATEWAY_HOSTS entry",
        ),
    ] {
        assert_env_rejects(key, value, expected);
    }
}

#[test]
fn publish_check_env_overrides_file_which_overrides_default() {
    let mut file = minimal_file();
    file.publish = PublishFile {
        check_dotfiles: Some("warn".into()),
        check_size: Some("require".into()),
        check_links: Some("off".into()),
        ..Default::default()
    };
    let cfg = build_config(file, None, |k| match k {
        "SWING_PUBLISH_CHECK_DOTFILES" => Some("off".into()),
        "SWING_PUBLISH_CHECK_LINKS" => Some("require".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.publish.check_dotfiles, CheckMode::Off);
    assert_eq!(cfg.source_of("publish.check_dotfiles"), Some(Source::Env));
    assert_eq!(cfg.publish.check_size, CheckMode::Require);
    assert_eq!(cfg.source_of("publish.check_size"), Some(Source::File));
    assert_eq!(cfg.publish.check_links, CheckMode::Require);
    assert_eq!(cfg.source_of("publish.check_links"), Some(Source::Env));
    assert_eq!(cfg.publish.check_unchanged, CheckMode::Require);
    assert_eq!(
        cfg.source_of("publish.check_unchanged"),
        Some(Source::Default)
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
fn dashboard_ui_can_be_disabled_from_env_or_file() {
    let cfg = build_config(minimal_file(), None, |k| {
        (k == "SWING_DASHBOARD_UI").then(|| "false".to_string())
    })
    .unwrap();
    assert!(!cfg.dashboard.ui, "env");

    let mut file = minimal_file();
    file.dashboard.ui = Some(false);
    let cfg = build_config(file, None, |_| None).unwrap();
    assert!(!cfg.dashboard.ui, "file");
}

#[test]
fn dashboard_gateway_empty_string_in_file_disables_links() {
    let file = ConfigFile {
        dashboard: DashboardFile {
            gateway: Some(String::new()),
            ..Default::default()
        },
        ..minimal_file()
    };
    let cfg = build_config(file, None, |_| None).unwrap();
    assert_eq!(cfg.dashboard.gateway, None);
}

#[test]
fn managed_kubo_rejects_an_explicit_ipfs_api() {
    let mut file = minimal_file();
    file.ipfs.api = Some("http://127.0.0.1:5001".into());
    let err = build_config(file, None, |_| None).unwrap_err();
    assert!(
        err.to_string().contains("[ipfs].api conflicts"),
        "file: {err}"
    );
    assert_env_rejects(
        "SWING_IPFS_API",
        "http://127.0.0.1:5001",
        "SWING_IPFS_API conflicts with [kubo].managed",
    );
}

#[test]
fn unmanaged_kubo_defaults_to_the_local_api_and_gateway() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_MANAGED" => Some("false".into()),
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:9999".into()),
        _ => None,
    })
    .unwrap();
    assert!(!cfg.kubo.managed);
    assert_eq!(
        cfg.ipfs.api,
        IpfsApi::Url("http://127.0.0.1:5001".to_string())
    );
    assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:8080");
}

#[test]
fn kubo_storage_max_defaults_to_max_total_storage() {
    for (name, storage_max, want) in [("unset", None, 50), ("set", Some("80GB"), 80)] {
        let cfg = build_config(minimal_file(), None, |k| match k {
            "SWING_MAX_TOTAL_STORAGE" => Some("50GB".into()),
            "SWING_KUBO_STORAGE_MAX" => storage_max.map(String::from),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.policy.max_total_storage, 50 * (1u64 << 30), "{name}");
        assert_eq!(cfg.kubo.storage_max, want * (1u64 << 30), "{name}");
    }
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
fn named_gateway_hosts_are_accepted_whatever_the_dashboard_listens_on() {
    for listen in [
        "0.0.0.0:8082",
        "[::]:8082",
        "192.168.1.5:8082",
        "127.0.0.1:8082",
    ] {
        let listen = listen.to_string();
        let built = build_config(minimal_file(), None, move |k| match k {
            "SWING_GATEWAY_HOSTS" => Some("example.com".into()),
            "SWING_DASHBOARD_LISTEN" => Some(listen.clone()),
            _ => None,
        });
        assert!(built.is_ok(), "{built:?}");
    }
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
fn gateway_upstream_defaults_to_managed_kubo_gateway_listen() {
    let cfg = build_config(minimal_file(), None, |k| match k {
        "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:9999".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:9999");
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
    file.publish.check_unchanged = Some("sometimes".into());
    let err = build_config(file, None, |_| None).unwrap_err();
    assert_eq!(err.to_string(), "invalid [publish].check_unchanged");

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
        format!("{err:#}").starts_with("invalid SWING_KEEP_DAYS: expected an integer: "),
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
