use super::*;
use crate::settings;

#[allow(clippy::too_many_arguments)]
fn resolve<T>(
    sources: &mut BTreeMap<String, Source>,
    key: &str,
    get_env: &impl Fn(&str) -> Option<String>,
    env_key: &str,
    file_val: Option<String>,
    parse: impl Fn(&str) -> Result<T>,
    env_ctx: &str,
    file_ctx: &str,
    default: T,
) -> Result<T> {
    match get_env(env_key) {
        Some(v) => {
            sources.insert(key.to_string(), Source::Env);
            parse(&v).context(env_ctx.to_string())
        }
        None => match file_val {
            Some(v) => {
                sources.insert(key.to_string(), Source::File);
                parse(&v).context(file_ctx.to_string())
            }
            None => {
                sources.insert(key.to_string(), Source::Default);
                Ok(default)
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_typed<T>(
    sources: &mut BTreeMap<String, Source>,
    key: &str,
    get_env: &impl Fn(&str) -> Option<String>,
    env_key: &str,
    file_val: Option<T>,
    parse: impl Fn(&str) -> Result<T>,
    env_ctx: &str,
    default: T,
) -> Result<T> {
    match get_env(env_key) {
        Some(v) => {
            sources.insert(key.to_string(), Source::Env);
            parse(&v).context(env_ctx.to_string())
        }
        None => match file_val {
            Some(v) => {
                sources.insert(key.to_string(), Source::File);
                Ok(v)
            }
            None => {
                sources.insert(key.to_string(), Source::Default);
                Ok(default)
            }
        },
    }
}

fn resolve_opt_path(
    sources: &mut BTreeMap<String, Source>,
    key: &str,
    get_env: &impl Fn(&str) -> Option<String>,
    env_key: &str,
    file_val: Option<String>,
) -> Option<PathBuf> {
    let env_val = get_env(env_key);
    sources.insert(
        key.to_string(),
        if env_val.is_some() {
            Source::Env
        } else if file_val.is_some() {
            Source::File
        } else {
            Source::Default
        },
    );
    match env_val {
        Some(v) => Some(PathBuf::from(v)),
        None => file_val.map(PathBuf::from),
    }
}

/// One function instead of three: the callers differ only in whether file
/// entries get trimmed and whether an empty result falls back to a default.
fn resolve_csv_list(
    sources: &mut BTreeMap<String, Source>,
    key: &str,
    get_env: &impl Fn(&str) -> Option<String>,
    env_key: &str,
    file_val: Option<Vec<String>>,
    clean_file_items: bool,
    default_when_empty: Option<&[&str]>,
) -> Vec<String> {
    fn split_csv(v: &str) -> Vec<String> {
        v.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
    fn clean(items: Vec<String>) -> Vec<String> {
        items
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    let (mut list, mut source) = match get_env(env_key) {
        Some(v) => (split_csv(&v), Source::Env),
        None => {
            let has_file_val = file_val.is_some();
            let raw = file_val.unwrap_or_default();
            let cleaned = if clean_file_items { clean(raw) } else { raw };
            let source = if has_file_val {
                Source::File
            } else {
                Source::Default
            };
            (cleaned, source)
        }
    };

    if list.is_empty()
        && let Some(default) = default_when_empty
    {
        list = default.iter().map(|s| s.to_string()).collect();
        source = Source::Default;
    }

    sources.insert(key.to_string(), source);
    list
}

fn resolve_nostr(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: NostrFile,
) -> Result<NostrConfig> {
    let secret_key_env = get_env(settings::env_of("nostr.secret_key"));
    sources.insert(
        "nostr.secret_key".to_string(),
        if secret_key_env.is_some() {
            Source::Env
        } else if file.secret_key.is_some() {
            Source::File
        } else {
            Source::Default
        },
    );
    let secret_key = secret_key_env.or(file.secret_key);

    let relays = resolve_csv_list(
        sources,
        "nostr.relays",
        get_env,
        settings::env_of("nostr.relays"),
        file.relays,
        false,
        Some(&DEFAULT_RELAYS),
    );

    let mirror_set = resolve(
        sources,
        "nostr.mirror_set",
        get_env,
        settings::env_of("nostr.mirror_set"),
        file.mirror_set,
        |s| Ok(s.to_string()),
        "invalid SWING_MIRROR_SET",
        "invalid [nostr].mirror_set",
        "swing".to_string(),
    )?;

    let site_event_kind = resolve_typed(
        sources,
        "nostr.site_event_kind",
        get_env,
        settings::env_of("nostr.site_event_kind"),
        file.site_event_kind,
        |v| v.parse().context("expected u16"),
        "invalid SWING_SITE_EVENT_KIND: expected u16",
        35980,
    )?;

    let replica_event_kind = resolve_typed(
        sources,
        "nostr.replica_event_kind",
        get_env,
        settings::env_of("nostr.replica_event_kind"),
        file.replica_event_kind,
        |v| v.parse().context("expected u16"),
        "invalid SWING_REPLICA_EVENT_KIND: expected u16",
        35981,
    )?;

    Ok(NostrConfig {
        secret_key: secret_key.map(NostrSecretKey::from),
        relays,
        mirror_set,
        site_event_kind,
        replica_event_kind,
    })
}

fn resolve_ipfs(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: IpfsFile,
    kubo_managed: bool,
) -> Result<IpfsConfig> {
    let ipfs_api_env = get_env(settings::env_of("ipfs.api"));
    let ipfs_api_explicit_source = if ipfs_api_env.is_some() {
        Some(Source::Env)
    } else if file.api.is_some() {
        Some(Source::File)
    } else {
        None
    };
    let ipfs_api_file = ipfs_api_env.or(file.api);
    if kubo_managed && ipfs_api_file.is_some() {
        bail!("[ipfs].api conflicts with [kubo].managed = true");
    }
    let api = if kubo_managed {
        IpfsApi::Managed
    } else {
        IpfsApi::Url(ipfs_api_file.unwrap_or_else(|| "http://127.0.0.1:5001".to_string()))
    };
    sources.insert(
        "ipfs.api".to_string(),
        if kubo_managed {
            Source::Default
        } else {
            ipfs_api_explicit_source.unwrap_or(Source::Default)
        },
    );

    let mfs_root = resolve(
        sources,
        "ipfs.mfs_root",
        get_env,
        settings::env_of("ipfs.mfs_root"),
        file.mfs_root,
        parse_mfs_root,
        "invalid SWING_MFS_ROOT",
        "invalid [ipfs].mfs_root",
        "/swing".to_string(),
    )?;

    Ok(IpfsConfig { api, mfs_root })
}

fn resolve_policy(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: PolicyFile,
) -> Result<PolicyConfig> {
    let max_total_storage = resolve(
        sources,
        "policy.max_total_storage",
        get_env,
        settings::env_of("policy.max_total_storage"),
        file.max_total_storage,
        parse_size,
        "invalid SWING_MAX_TOTAL_STORAGE",
        "invalid [policy].max_total_storage",
        100 * (1u64 << 30),
    )?;

    let max_per_site = resolve(
        sources,
        "policy.max_per_site",
        get_env,
        settings::env_of("policy.max_per_site"),
        file.max_per_site,
        parse_size,
        "invalid SWING_MAX_PER_SITE",
        "invalid [policy].max_per_site",
        10 * (1u64 << 30),
    )?;

    let max_per_account = resolve(
        sources,
        "policy.max_per_account",
        get_env,
        settings::env_of("policy.max_per_account"),
        file.max_per_account,
        parse_size,
        "invalid SWING_MAX_PER_ACCOUNT",
        "invalid [policy].max_per_account",
        20 * (1u64 << 30),
    )?;

    let max_sites_per_account = resolve_typed(
        sources,
        "policy.max_sites_per_account",
        get_env,
        settings::env_of("policy.max_sites_per_account"),
        file.max_sites_per_account,
        |v| v.parse().context("expected integer"),
        "invalid SWING_MAX_SITES_PER_ACCOUNT: expected integer",
        10,
    )?;
    if max_sites_per_account == 0 {
        bail!("max_sites_per_account must be greater than 0");
    }

    let max_update_size = resolve(
        sources,
        "policy.max_update_size",
        get_env,
        settings::env_of("policy.max_update_size"),
        file.max_update_size,
        parse_size,
        "invalid SWING_MAX_UPDATE_SIZE",
        "invalid [policy].max_update_size",
        2 * (1u64 << 30),
    )?;

    let keep_versions = resolve_typed(
        sources,
        "policy.keep_versions",
        get_env,
        settings::env_of("policy.keep_versions"),
        file.keep_versions,
        |v| v.parse().context("expected integer"),
        "invalid SWING_KEEP_VERSIONS: expected integer",
        5,
    )?;

    let keep_days = resolve_typed(
        sources,
        "policy.keep_days",
        get_env,
        settings::env_of("policy.keep_days"),
        file.keep_days,
        |v| v.parse().context("expected integer"),
        "invalid SWING_KEEP_DAYS: expected integer",
        365,
    )?;

    let min_update_interval = resolve(
        sources,
        "policy.min_update_interval",
        get_env,
        settings::env_of("policy.min_update_interval"),
        file.min_update_interval,
        parse_duration_secs,
        "invalid SWING_MIN_UPDATE_INTERVAL",
        "invalid [policy].min_update_interval",
        3600,
    )?;

    let remove_on_unfollow = resolve_typed(
        sources,
        "policy.remove_on_unfollow",
        get_env,
        settings::env_of("policy.remove_on_unfollow"),
        file.remove_on_unfollow,
        parse_bool,
        "invalid SWING_REMOVE_ON_UNFOLLOW",
        true,
    )?;

    let nip05 = resolve(
        sources,
        "policy.nip05",
        get_env,
        settings::env_of("policy.nip05"),
        file.nip05,
        parse_nip05_mode,
        "invalid SWING_NIP05",
        "invalid [policy].nip05",
        Nip05Mode::Warn,
    )?;

    let nip05_cache_ttl = resolve(
        sources,
        "policy.nip05_cache_ttl",
        get_env,
        settings::env_of("policy.nip05_cache_ttl"),
        file.nip05_cache_ttl,
        parse_duration_secs,
        "invalid SWING_NIP05_CACHE_TTL",
        "invalid [policy].nip05_cache_ttl",
        86_400,
    )?;

    Ok(PolicyConfig {
        max_total_storage,
        max_per_site,
        max_per_account,
        max_sites_per_account,
        max_update_size,
        keep_versions,
        keep_days,
        min_update_interval,
        remove_on_unfollow,
        nip05,
        nip05_cache_ttl,
    })
}

fn resolve_agent(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: AgentFile,
) -> Result<AgentConfig> {
    let state_dir = resolve(
        sources,
        "agent.state_dir",
        get_env,
        settings::env_of("agent.state_dir"),
        file.state_dir,
        |s| Ok(s.to_string()),
        "invalid SWING_STATE_DIR",
        "invalid [agent].state_dir",
        "./data".to_string(),
    )?;

    let poll_interval = resolve(
        sources,
        "agent.poll_interval",
        get_env,
        settings::env_of("agent.poll_interval"),
        file.poll_interval,
        parse_duration_secs,
        "invalid SWING_POLL_INTERVAL",
        "invalid [agent].poll_interval",
        300,
    )?;
    if poll_interval == 0 {
        bail!("poll_interval must be greater than 0");
    }

    let fetch_timeout = resolve(
        sources,
        "agent.fetch_timeout",
        get_env,
        settings::env_of("agent.fetch_timeout"),
        file.fetch_timeout,
        parse_duration_secs,
        "invalid SWING_FETCH_TIMEOUT",
        "invalid [agent].fetch_timeout",
        900,
    )?;
    if fetch_timeout == 0 {
        bail!("SWING_FETCH_TIMEOUT must be greater than 0");
    }

    let fetch_idle_timeout = resolve(
        sources,
        "agent.fetch_idle_timeout",
        get_env,
        settings::env_of("agent.fetch_idle_timeout"),
        file.fetch_idle_timeout,
        parse_duration_secs,
        "invalid SWING_FETCH_IDLE_TIMEOUT",
        "invalid [agent].fetch_idle_timeout",
        120,
    )?;
    if fetch_idle_timeout == 0 {
        bail!("SWING_FETCH_IDLE_TIMEOUT must be greater than 0");
    }

    let concurrency = resolve_typed(
        sources,
        "agent.concurrency",
        get_env,
        settings::env_of("agent.concurrency"),
        file.concurrency,
        |v| v.parse().context("expected integer"),
        "invalid SWING_CONCURRENCY: expected integer",
        4,
    )?;
    if concurrency == 0 {
        bail!("concurrency must be greater than 0");
    }

    let report_ttl = resolve(
        sources,
        "agent.report_ttl",
        get_env,
        settings::env_of("agent.report_ttl"),
        file.report_ttl,
        parse_duration_secs,
        "invalid SWING_REPORT_TTL",
        "invalid [agent].report_ttl",
        3 * 86_400,
    )?;
    if report_ttl / 2 <= poll_interval {
        bail!("report_ttl must be more than twice poll_interval");
    }
    if report_ttl > crate::nostr::MAX_REPORT_AGE {
        bail!(
            "report_ttl must be at most {}",
            crate::format::format_duration_secs(crate::nostr::MAX_REPORT_AGE)
        );
    }

    Ok(AgentConfig {
        state_dir: PathBuf::from(state_dir),
        poll_interval: Duration::from_secs(poll_interval),
        fetch_timeout: Duration::from_secs(fetch_timeout),
        fetch_idle_timeout: Duration::from_secs(fetch_idle_timeout),
        concurrency,
        report_ttl: Duration::from_secs(report_ttl),
    })
}

fn resolve_publish(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: PublishFile,
) -> Result<PublishConfig> {
    let nip05 = resolve(
        sources,
        "publish.nip05",
        get_env,
        settings::env_of("publish.nip05"),
        file.nip05,
        parse_nip05_mode,
        "invalid SWING_PUBLISH_NIP05",
        "invalid [publish].nip05",
        Nip05Mode::Warn,
    )?;

    let keep_versions = resolve_typed(
        sources,
        "publish.keep_versions",
        get_env,
        settings::env_of("publish.keep_versions"),
        file.keep_versions,
        |v| v.parse().context("expected integer"),
        "invalid SWING_PUBLISH_KEEP_VERSIONS: expected integer",
        5,
    )?;
    if keep_versions == 0 {
        bail!("publish keep_versions must be greater than 0");
    }

    Ok(PublishConfig {
        nip05,
        keep_versions,
    })
}

fn resolve_dashboard(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: DashboardFile,
) -> Result<DashboardConfig> {
    let listen = resolve(
        sources,
        "dashboard.listen",
        get_env,
        settings::env_of("dashboard.listen"),
        file.listen,
        parse_dashboard_listen,
        "invalid SWING_DASHBOARD_LISTEN",
        "invalid [dashboard].listen",
        SocketAddr::from(([127, 0, 0, 1], 8082)),
    )?;

    let ui = resolve_typed(
        sources,
        "dashboard.ui",
        get_env,
        settings::env_of("dashboard.ui"),
        file.ui,
        parse_bool,
        "invalid SWING_DASHBOARD_UI",
        true,
    )?;

    let allowed_hosts = resolve_csv_list(
        sources,
        "dashboard.allowed_hosts",
        get_env,
        settings::env_of("dashboard.allowed_hosts"),
        file.allowed_hosts,
        false,
        None,
    );

    let public_url_env = get_env(settings::env_of("dashboard.public_url"));
    sources.insert(
        "dashboard.public_url".to_string(),
        if public_url_env.is_some() {
            Source::Env
        } else if file.public_url.is_some() {
            Source::File
        } else {
            Source::Default
        },
    );
    let public_url = match public_url_env {
        Some(v) => Some(parse_public_url(&v).context("invalid SWING_DASHBOARD_PUBLIC_URL")?),
        None => file
            .public_url
            .as_deref()
            .map(parse_public_url)
            .transpose()
            .context("invalid [dashboard].public_url")?,
    };

    let gateway_raw = resolve(
        sources,
        "dashboard.gateway",
        get_env,
        settings::env_of("dashboard.gateway"),
        file.gateway,
        |s| Ok(s.to_string()),
        "invalid SWING_DASHBOARD_GATEWAY",
        "invalid [dashboard].gateway",
        "http://localhost:8080".to_string(),
    )?;
    let gateway = Some(gateway_raw).filter(|s| !s.is_empty());

    let custom_css = resolve_opt_path(
        sources,
        "dashboard.custom_css",
        get_env,
        settings::env_of("dashboard.custom_css"),
        file.custom_css,
    );
    let desktop_page = resolve_opt_path(
        sources,
        "dashboard.desktop_page",
        get_env,
        settings::env_of("dashboard.desktop_page"),
        file.desktop_page,
    );
    let desktop_page_css = resolve_opt_path(
        sources,
        "dashboard.desktop_page_css",
        get_env,
        settings::env_of("dashboard.desktop_page_css"),
        file.desktop_page_css,
    );
    let desktop_banner = resolve_opt_path(
        sources,
        "dashboard.desktop_banner",
        get_env,
        settings::env_of("dashboard.desktop_banner"),
        file.desktop_banner,
    );

    let max_upload = resolve(
        sources,
        "dashboard.max_upload",
        get_env,
        settings::env_of("dashboard.max_upload"),
        file.max_upload,
        parse_size,
        "invalid SWING_DASHBOARD_MAX_UPLOAD",
        "invalid [dashboard].max_upload",
        2 * (1u64 << 30),
    )?;
    if max_upload == 0 {
        bail!("dashboard max_upload must be greater than 0");
    }

    Ok(DashboardConfig {
        listen,
        ui,
        allowed_hosts,
        public_url,
        gateway,
        custom_css,
        desktop_page,
        desktop_page_css,
        desktop_banner,
        max_upload,
    })
}

fn resolve_kubo(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: KuboFile,
    state_dir: &Path,
    max_total_storage: u64,
) -> Result<KuboConfig> {
    let managed = resolve_typed(
        sources,
        "kubo.managed",
        get_env,
        settings::env_of("kubo.managed"),
        file.managed,
        parse_bool,
        "invalid SWING_KUBO_MANAGED",
        true,
    )?;

    let binary = resolve_opt_path(
        sources,
        "kubo.binary",
        get_env,
        settings::env_of("kubo.binary"),
        file.binary,
    );

    let repo = resolve(
        sources,
        "kubo.repo",
        get_env,
        settings::env_of("kubo.repo"),
        file.repo,
        |v| Ok(PathBuf::from(v.trim())),
        "invalid SWING_KUBO_REPO",
        "invalid [kubo].repo",
        state_dir.join("kubo"),
    )?;

    let storage_max = resolve(
        sources,
        "kubo.storage_max",
        get_env,
        settings::env_of("kubo.storage_max"),
        file.storage_max,
        parse_size,
        "invalid SWING_KUBO_STORAGE_MAX",
        "invalid [kubo].storage_max",
        max_total_storage,
    )?;

    let provide_strategy = resolve(
        sources,
        "kubo.provide_strategy",
        get_env,
        settings::env_of("kubo.provide_strategy"),
        file.provide_strategy,
        |s| Ok(s.to_string()),
        "invalid SWING_KUBO_PROVIDE_STRATEGY",
        "invalid [kubo].provide_strategy",
        "pinned+mfs".to_string(),
    )?;
    if provide_strategy.trim().is_empty() {
        bail!("[kubo].provide_strategy must not be empty");
    }

    let gateway_listen = resolve(
        sources,
        "kubo.gateway_listen",
        get_env,
        settings::env_of("kubo.gateway_listen"),
        file.gateway_listen,
        |v| {
            v.trim()
                .parse::<SocketAddr>()
                .with_context(|| format!("invalid gateway listen address: {v}"))
        },
        "invalid SWING_KUBO_GATEWAY_LISTEN",
        "invalid [kubo].gateway_listen",
        SocketAddr::from(([127, 0, 0, 1], 8080)),
    )?;

    let kubo_swarm_port_env = get_env(settings::env_of("kubo.swarm_port"));
    sources.insert(
        "kubo.swarm_port".to_string(),
        if kubo_swarm_port_env.is_some() {
            Source::Env
        } else if file.swarm_port.is_some() {
            Source::File
        } else {
            Source::Default
        },
    );
    let swarm_port = match kubo_swarm_port_env {
        Some(v) => Some(
            v.trim()
                .parse::<u16>()
                .context("invalid SWING_KUBO_SWARM_PORT: expected u16")?,
        ),
        None => file.swarm_port,
    };
    if swarm_port == Some(0) {
        bail!("[kubo].swarm_port must be between 1 and 65535");
    }

    Ok(KuboConfig {
        managed,
        binary,
        repo,
        storage_max,
        provide_strategy,
        gateway_listen,
        swarm_port,
    })
}

fn resolve_gateway(
    sources: &mut BTreeMap<String, Source>,
    get_env: &impl Fn(&str) -> Option<String>,
    file: GatewayFile,
    kubo_managed: bool,
    kubo_gateway_listen: SocketAddr,
) -> Result<GatewayConfig> {
    let listen = resolve(
        sources,
        "gateway.listen",
        get_env,
        settings::env_of("gateway.listen"),
        file.listen,
        parse_listen,
        "invalid SWING_GATEWAY_LISTEN",
        "invalid [gateway].listen",
        Listen::Off,
    )?;

    let hosts = resolve_csv_list(
        sources,
        "gateway.hosts",
        get_env,
        settings::env_of("gateway.hosts"),
        file.hosts,
        true,
        None,
    );
    for host in &hosts {
        if !is_valid_gateway_host(host) {
            bail!("invalid [gateway].hosts entry: {host}");
        }
    }
    if !matches!(listen, Listen::Off) && hosts.is_empty() {
        bail!("[gateway].hosts must not be empty when [gateway].listen is enabled");
    }

    let upstream_default = if kubo_managed {
        format!("http://{kubo_gateway_listen}")
    } else {
        "http://127.0.0.1:8080".to_string()
    };
    let upstream = resolve(
        sources,
        "gateway.upstream",
        get_env,
        settings::env_of("gateway.upstream"),
        file.upstream,
        |s| Ok(s.to_string()),
        "invalid SWING_GATEWAY_UPSTREAM",
        "invalid [gateway].upstream",
        upstream_default,
    )?;

    Ok(GatewayConfig {
        listen,
        hosts,
        upstream,
    })
}

pub(super) fn build_config(
    file: ConfigFile,
    get_env: impl Fn(&str) -> Option<String>,
) -> Result<Config> {
    let mut sources: BTreeMap<String, Source> = BTreeMap::new();
    let get_env = &get_env;

    let nostr = resolve_nostr(&mut sources, get_env, file.nostr)?;
    let policy = resolve_policy(&mut sources, get_env, file.policy)?;
    let agent = resolve_agent(&mut sources, get_env, file.agent)?;
    let kubo = resolve_kubo(
        &mut sources,
        get_env,
        file.kubo,
        &agent.state_dir,
        policy.max_total_storage,
    )?;
    let ipfs = resolve_ipfs(&mut sources, get_env, file.ipfs, kubo.managed)?;
    let publish = resolve_publish(&mut sources, get_env, file.publish)?;
    let dashboard = resolve_dashboard(&mut sources, get_env, file.dashboard)?;
    let gateway = resolve_gateway(
        &mut sources,
        get_env,
        file.gateway,
        kubo.managed,
        kubo.gateway_listen,
    )?;

    Ok(Config {
        nostr,
        ipfs,
        policy,
        agent,
        publish,
        dashboard,
        kubo,
        gateway,
        config_path: PathBuf::new(),
        config_exists: false,
        sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_file() -> ConfigFile {
        ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn assert_env_rejects(env_key: &'static str, value: &'static str, expected_substring: &str) {
        let err = build_config(minimal_file(), move |k| {
            (k == env_key).then(|| value.to_string())
        })
        .unwrap_err();
        assert!(
            err.to_string().contains(expected_substring),
            "expected error containing {expected_substring:?}, got: {err}"
        );
    }

    #[test]
    fn zero_poll_interval_is_rejected() {
        assert_env_rejects("SWING_POLL_INTERVAL", "0s", "poll_interval");
    }

    #[test]
    fn zero_concurrency_and_idle_timeout_are_rejected() {
        assert_env_rejects("SWING_CONCURRENCY", "0", "concurrency");
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
        let err = build_config(minimal_file(), env("20m")).unwrap_err();
        assert!(err.to_string().contains("report_ttl"));
        let cfg = build_config(minimal_file(), env("21m")).unwrap();
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
        let cfg = build_config(minimal_file(), env("7d")).unwrap();
        assert_eq!(
            cfg.agent.report_ttl,
            Duration::from_secs(crate::nostr::MAX_REPORT_AGE)
        );
        let err = build_config(minimal_file(), env("604801s")).unwrap_err();
        assert!(
            err.to_string().contains("report_ttl must be at most 7d"),
            "{err}"
        );
    }

    #[test]
    fn zero_max_sites_per_account_is_rejected() {
        assert_env_rejects("SWING_MAX_SITES_PER_ACCOUNT", "0", "max_sites_per_account");
    }

    #[test]
    fn mfs_root_env_override_is_normalized() {
        let cfg = build_config(minimal_file(), |k| match k {
            "SWING_MFS_ROOT" => Some("/mirror/".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.ipfs.mfs_root, "/mirror");
    }

    #[test]
    fn zero_publish_keep_versions_is_rejected() {
        assert_env_rejects("SWING_PUBLISH_KEEP_VERSIONS", "0", "keep_versions");
    }

    #[test]
    fn zero_fetch_timeout_is_rejected() {
        assert_env_rejects("SWING_FETCH_TIMEOUT", "0", "SWING_FETCH_TIMEOUT");
    }

    #[test]
    fn env_overrides_toml() {
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("file-key".into()),
                relays: Some(vec!["wss://from-file".into()]),
                mirror_set: Some("from-file-set".into()),
                site_event_kind: Some(1111),
                replica_event_kind: Some(2222),
            },
            ..Default::default()
        };
        let cfg = build_config(file, |k| match k {
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
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |_| None).unwrap();
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
        assert_eq!(cfg.policy.nip05, Nip05Mode::Warn);
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
                secret_key: Some("k".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |_| None).unwrap();
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
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            policy: PolicyFile {
                nip05: Some("require".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |k| match k {
            "SWING_NIP05" => Some("off".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.policy.nip05, Nip05Mode::Off);
    }

    #[test]
    fn nip05_mode_rejects_garbage() {
        assert_env_rejects("SWING_NIP05", "maybe", "invalid SWING_NIP05");
    }

    #[test]
    fn publish_nip05_defaults_to_warn() {
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |_| None).unwrap();
        assert_eq!(cfg.publish.nip05, Nip05Mode::Warn);
    }

    #[test]
    fn publish_nip05_env_overrides_file() {
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            publish: PublishFile {
                nip05: Some("require".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |k| match k {
            "SWING_PUBLISH_NIP05" => Some("off".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.publish.nip05, Nip05Mode::Off);
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
        let cfg = build_config(minimal_file(), |_| None).unwrap();
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
        assert_eq!(cfg.dashboard.max_upload, 2 * (1u64 << 30));
    }

    #[test]
    fn dashboard_max_upload_env_overrides_file() {
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            dashboard: DashboardFile {
                max_upload: Some("4GB".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |k| match k {
            "SWING_DASHBOARD_MAX_UPLOAD" => Some("512MB".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.dashboard.max_upload, 512 * (1u64 << 20));
    }

    #[test]
    fn dashboard_max_upload_zero_is_rejected() {
        assert_env_rejects("SWING_DASHBOARD_MAX_UPLOAD", "0", "max_upload");
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
        let cfg = build_config(minimal_file(), |_| None).unwrap();
        assert!(cfg.dashboard.ui);

        let cfg = build_config(minimal_file(), |k| match k {
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
        let cfg = build_config(file, |_| None).unwrap();
        assert!(!cfg.dashboard.ui);
    }

    #[test]
    fn dashboard_env_overrides_file() {
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
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
                max_upload: Some("4GB".into()),
            },
            ..Default::default()
        };
        let cfg = build_config(file, |k| match k {
            "SWING_DASHBOARD_LISTEN" => Some("0.0.0.0:8082".into()),
            "SWING_DASHBOARD_ALLOWED_HOSTS" => Some("a.example, b.example".into()),
            "SWING_DASHBOARD_GATEWAY" => Some("http://env-gateway.example".into()),
            "SWING_DASHBOARD_CUSTOM_CSS" => Some("/env/custom.css".into()),
            "SWING_DASHBOARD_DESKTOP_PAGE" => Some("/env/page.html".into()),
            "SWING_DASHBOARD_DESKTOP_PAGE_CSS" => Some("/env/page.css".into()),
            "SWING_DASHBOARD_DESKTOP_BANNER" => Some("/env/banner.gif".into()),
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
    }

    #[test]
    fn dashboard_gateway_empty_string_in_file_disables_links() {
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            dashboard: DashboardFile {
                gateway: Some(String::new()),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |_| None).unwrap();
        assert_eq!(cfg.dashboard.gateway, None);
    }

    #[test]
    fn max_total_storage_env_is_a_size_string() {
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = build_config(file, |k| match k {
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
        let err = build_config(file, |_| None).unwrap_err();
        assert!(err.to_string().contains("[ipfs].api conflicts"));
    }

    #[test]
    fn managed_kubo_rejects_explicit_ipfs_api_from_env() {
        assert_env_rejects(
            "SWING_IPFS_API",
            "http://127.0.0.1:5001",
            "[ipfs].api conflicts",
        );
    }

    #[test]
    fn unmanaged_kubo_uses_ipfs_api() {
        let cfg = build_config(minimal_file(), |k| match k {
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
        let cfg = build_config(minimal_file(), |k| match k {
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
        let cfg = build_config(minimal_file(), |k| match k {
            "SWING_STATE_DIR" => Some("/var/lib/swing".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.kubo.repo, PathBuf::from("/var/lib/swing/kubo"));
    }

    #[test]
    fn kubo_repo_env_overrides_default() {
        let cfg = build_config(minimal_file(), |k| match k {
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
        let cfg = build_config(file, |k| match k {
            "SWING_KUBO_BINARY" => Some("/usr/local/bin/ipfs".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.kubo.binary, Some(PathBuf::from("/usr/local/bin/ipfs")));
    }

    #[test]
    fn kubo_storage_max_defaults_to_max_total_storage() {
        let cfg = build_config(minimal_file(), |k| match k {
            "SWING_MAX_TOTAL_STORAGE" => Some("50GB".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.kubo.storage_max, 50 * (1u64 << 30));
    }

    #[test]
    fn kubo_storage_max_can_differ_from_max_total_storage() {
        let cfg = build_config(minimal_file(), |k| match k {
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
        let err = build_config(file, |_| None).unwrap_err();
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
        let cfg = build_config(file, |k| match k {
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
        let cfg = build_config(minimal_file(), |_| None).unwrap();
        assert_eq!(cfg.kubo.swarm_port, None);
    }

    #[test]
    fn kubo_swarm_port_zero_is_rejected() {
        assert_env_rejects("SWING_KUBO_SWARM_PORT", "0", "swarm_port");
    }

    #[test]
    fn kubo_swarm_port_in_range_is_accepted() {
        let cfg = build_config(minimal_file(), |k| match k {
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
        let cfg = build_config(minimal_file(), |k| match k {
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
    fn gateway_hosts_rejects_invalid_hostnames() {
        assert_env_rejects(
            "SWING_GATEWAY_HOSTS",
            "Example.com",
            "invalid [gateway].hosts entry",
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
        let cfg = build_config(file, |k| match k {
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
        let cfg = build_config(minimal_file(), |_| None).unwrap();
        assert!(cfg.kubo.managed);
        assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:8080");

        let cfg = build_config(minimal_file(), |k| match k {
            "SWING_KUBO_GATEWAY_LISTEN" => Some("127.0.0.1:9999".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:9999");
    }

    #[test]
    fn gateway_upstream_defaults_to_localhost_when_unmanaged() {
        let cfg = build_config(minimal_file(), |k| match k {
            "SWING_KUBO_MANAGED" => Some("false".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.gateway.upstream, "http://127.0.0.1:8080");
    }

    #[test]
    fn gateway_upstream_env_overrides_default() {
        let cfg = build_config(minimal_file(), |k| match k {
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
        let cfg = build_config(file, |k| match k {
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
        let cfg = build_config(minimal_file(), |_| None).unwrap();
        assert_eq!(cfg.source_of("nostr.secret_key"), Some(Source::File));
        assert_eq!(cfg.source_of("ipfs.api"), Some(Source::Default));

        let cfg = build_config(ConfigFile::default(), |_| None).unwrap();
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
            build_config_from_str("[dashboard]\npublic_url = \"http://x/sub\"\n", |_| None)
                .is_err()
        );
    }
}
