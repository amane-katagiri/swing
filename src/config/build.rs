use std::str::FromStr;

use super::*;
use crate::settings;

enum Picked<T> {
    Env(String),
    File(T),
    Default,
}

struct Resolver<E> {
    sources: BTreeMap<String, Source>,
    get_env: E,
}

impl<E: Fn(&str) -> Option<String>> Resolver<E> {
    fn new(get_env: E) -> Self {
        Self {
            sources: BTreeMap::new(),
            get_env,
        }
    }

    fn pick<T>(&mut self, key: &str, file_val: Option<T>) -> Picked<T> {
        let picked = match (self.get_env)(settings::env_of(key)) {
            Some(v) => Picked::Env(v),
            None => match file_val {
                Some(v) => Picked::File(v),
                None => Picked::Default,
            },
        };
        let source = match picked {
            Picked::Env(_) => Source::Env,
            Picked::File(_) => Source::File,
            Picked::Default => Source::Default,
        };
        self.sources.insert(key.to_string(), source);
        picked
    }

    fn source(&self, key: &str) -> Option<Source> {
        self.sources.get(key).copied()
    }

    fn name(&self, key: &str) -> String {
        let setting =
            settings::find(key).unwrap_or_else(|| panic!("config: no such catalog key: {key}"));
        match self.source(key) {
            Some(Source::Env) => setting.env.to_string(),
            _ => format!("[{}].{}", setting.section, setting.field),
        }
    }

    fn invalid(&self, key: &str) -> String {
        format!("invalid {}", self.name(key))
    }

    fn parse<T>(
        &mut self,
        key: &str,
        file_val: Option<String>,
        parse: impl Fn(&str) -> Result<T>,
        default: T,
    ) -> Result<T> {
        Ok(self.opt(key, file_val, parse)?.unwrap_or(default))
    }

    fn opt<T>(
        &mut self,
        key: &str,
        file_val: Option<String>,
        parse: impl Fn(&str) -> Result<T>,
    ) -> Result<Option<T>> {
        match self.pick(key, file_val) {
            Picked::Env(v) | Picked::File(v) => {
                parse(&v).map(Some).with_context(|| self.invalid(key))
            }
            Picked::Default => Ok(None),
        }
    }

    fn typed<T>(
        &mut self,
        key: &str,
        file_val: Option<T>,
        parse: impl Fn(&str) -> Result<T>,
        default: T,
    ) -> Result<T> {
        Ok(self.opt_typed(key, file_val, parse)?.unwrap_or(default))
    }

    fn opt_typed<T>(
        &mut self,
        key: &str,
        file_val: Option<T>,
        parse: impl Fn(&str) -> Result<T>,
    ) -> Result<Option<T>> {
        match self.pick(key, file_val) {
            Picked::Env(v) => parse(&v).map(Some).with_context(|| self.invalid(key)),
            Picked::File(v) => Ok(Some(v)),
            Picked::Default => Ok(None),
        }
    }

    fn opt_string(&mut self, key: &str, file_val: Option<String>) -> Option<String> {
        match self.pick(key, file_val) {
            Picked::Env(v) | Picked::File(v) => Some(v),
            Picked::Default => None,
        }
    }

    fn string(&mut self, key: &str, file_val: Option<String>, default: &str) -> String {
        self.opt_string(key, file_val)
            .unwrap_or_else(|| default.to_string())
    }

    fn rebase(&self, key: &str, base: Option<&Path>, path: PathBuf) -> PathBuf {
        match base {
            Some(base) if self.source(key) != Some(Source::Env) && path.is_relative() => {
                base.join(path.strip_prefix(".").unwrap_or(&path))
            }
            _ => path,
        }
    }

    fn opt_path(
        &mut self,
        key: &str,
        file_val: Option<String>,
        base: Option<&Path>,
    ) -> Option<PathBuf> {
        match self.pick(key, file_val) {
            Picked::Env(v) => Some(PathBuf::from(v)),
            Picked::File(v) => Some(self.rebase(key, base, PathBuf::from(v))),
            Picked::Default => None,
        }
    }

    fn list(
        &mut self,
        key: &str,
        file_val: Option<Vec<String>>,
        clean_file_items: bool,
        default: &[&str],
    ) -> Vec<String> {
        fn clean<'a>(items: impl Iterator<Item = &'a str>) -> Vec<String> {
            items
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        match self.pick(key, file_val) {
            Picked::Env(v) => clean(v.split(',')),
            Picked::File(items) if clean_file_items => clean(items.iter().map(String::as_str)),
            Picked::File(items) => items,
            Picked::Default => default.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn positive<T: PartialEq + Default>(&self, key: &str, value: T) -> Result<T> {
        if value == T::default() {
            bail!("{} must be greater than 0", self.name(key));
        }
        Ok(value)
    }
}

fn integer<T: FromStr>(v: &str) -> Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    v.parse().context("expected integer")
}

fn u16_value(v: &str) -> Result<u16> {
    v.parse().context("expected u16")
}

fn resolve_nostr<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: NostrFile,
) -> Result<NostrConfig> {
    let secret_key = r.opt_string("nostr.secret_key", file.secret_key);

    let mut relays = r.list("nostr.relays", file.relays, false, &DEFAULT_RELAYS);
    if relays.is_empty() {
        relays = DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect();
        r.sources
            .insert("nostr.relays".to_string(), Source::Default);
    }

    Ok(NostrConfig {
        secret_key: secret_key.map(NostrSecretKey::from),
        relays,
        mirror_set: r.string("nostr.mirror_set", file.mirror_set, "swing"),
        site_event_kind: r.typed(
            "nostr.site_event_kind",
            file.site_event_kind,
            u16_value,
            35980,
        )?,
        replica_event_kind: r.typed(
            "nostr.replica_event_kind",
            file.replica_event_kind,
            u16_value,
            35981,
        )?,
    })
}

fn resolve_ipfs<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: IpfsFile,
    kubo_managed: bool,
) -> Result<IpfsConfig> {
    let api = match r.opt_string("ipfs.api", file.api) {
        Some(_) if kubo_managed => bail!(
            "{} conflicts with {} = true",
            r.name("ipfs.api"),
            r.name("kubo.managed")
        ),
        None if kubo_managed => IpfsApi::Managed,
        Some(url) => IpfsApi::Url(url),
        None => IpfsApi::Url("http://127.0.0.1:5001".to_string()),
    };

    let mfs_root = r.parse(
        "ipfs.mfs_root",
        file.mfs_root,
        parse_mfs_root,
        "/swing".to_string(),
    )?;

    Ok(IpfsConfig { api, mfs_root })
}

fn resolve_policy<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: PolicyFile,
) -> Result<PolicyConfig> {
    let max_total_storage = r.parse(
        "policy.max_total_storage",
        file.max_total_storage,
        parse_size,
        100 * (1u64 << 30),
    )?;
    let max_per_site = r.parse(
        "policy.max_per_site",
        file.max_per_site,
        parse_size,
        10 * (1u64 << 30),
    )?;
    let max_per_account = r.parse(
        "policy.max_per_account",
        file.max_per_account,
        parse_size,
        20 * (1u64 << 30),
    )?;
    let max_sites_per_account = r.typed(
        "policy.max_sites_per_account",
        file.max_sites_per_account,
        integer,
        10,
    )?;
    let max_sites_per_account =
        r.positive("policy.max_sites_per_account", max_sites_per_account)?;
    let max_update_size = r.parse(
        "policy.max_update_size",
        file.max_update_size,
        parse_size,
        DEFAULT_MAX_UPDATE_SIZE,
    )?;

    Ok(PolicyConfig {
        max_total_storage,
        max_per_site,
        max_per_account,
        max_sites_per_account,
        max_update_size,
        keep_versions: r.typed("policy.keep_versions", file.keep_versions, integer, 5)?,
        keep_days: r.typed("policy.keep_days", file.keep_days, integer, 365)?,
        min_update_interval: r.parse(
            "policy.min_update_interval",
            file.min_update_interval,
            parse_duration_secs,
            3600,
        )?,
        remove_on_unfollow: r.typed(
            "policy.remove_on_unfollow",
            file.remove_on_unfollow,
            parse_bool,
            true,
        )?,
        nip05: r.parse(
            "policy.nip05",
            file.nip05,
            parse_check_mode,
            CheckMode::Warn,
        )?,
        nip05_cache_ttl: r.parse(
            "policy.nip05_cache_ttl",
            file.nip05_cache_ttl,
            parse_duration_secs,
            86_400,
        )?,
    })
}

fn resolve_agent<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: AgentFile,
    base: Option<&Path>,
) -> Result<AgentConfig> {
    let state_dir = r.string("agent.state_dir", file.state_dir, "./data");
    let state_dir = r.rebase("agent.state_dir", base, PathBuf::from(state_dir));

    let poll_interval = r.parse(
        "agent.poll_interval",
        file.poll_interval,
        parse_duration_secs,
        300,
    )?;
    let poll_interval = r.positive("agent.poll_interval", poll_interval)?;

    let fetch_timeout = r.parse(
        "agent.fetch_timeout",
        file.fetch_timeout,
        parse_duration_secs,
        900,
    )?;
    let fetch_timeout = r.positive("agent.fetch_timeout", fetch_timeout)?;

    let fetch_idle_timeout = r.parse(
        "agent.fetch_idle_timeout",
        file.fetch_idle_timeout,
        parse_duration_secs,
        120,
    )?;
    let fetch_idle_timeout = r.positive("agent.fetch_idle_timeout", fetch_idle_timeout)?;

    let concurrency = r.typed("agent.concurrency", file.concurrency, integer, 4)?;
    let concurrency = r.positive("agent.concurrency", concurrency)?;

    let report_ttl = r.parse(
        "agent.report_ttl",
        file.report_ttl,
        parse_duration_secs,
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
        state_dir,
        poll_interval: Duration::from_secs(poll_interval),
        fetch_timeout: Duration::from_secs(fetch_timeout),
        fetch_idle_timeout: Duration::from_secs(fetch_idle_timeout),
        concurrency,
        report_ttl: Duration::from_secs(report_ttl),
    })
}

fn resolve_publish<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: PublishFile,
) -> Result<PublishConfig> {
    let nip05 = r.parse(
        "publish.nip05",
        file.nip05,
        parse_check_mode,
        CheckMode::Warn,
    )?;

    let keep_versions = r.typed("publish.keep_versions", file.keep_versions, integer, 5)?;
    let keep_versions = r.positive("publish.keep_versions", keep_versions)?;

    let check_dotfiles = r.parse(
        "publish.check_dotfiles",
        file.check_dotfiles,
        parse_check_mode,
        CheckMode::Require,
    )?;
    let check_size = r.parse(
        "publish.check_size",
        file.check_size,
        parse_check_mode,
        CheckMode::Warn,
    )?;
    let check_unchanged = r.parse(
        "publish.check_unchanged",
        file.check_unchanged,
        parse_check_mode,
        CheckMode::Require,
    )?;

    let dotfiles_allow = r.list(
        "publish.dotfiles_allow",
        file.dotfiles_allow,
        true,
        &DEFAULT_DOTFILES_ALLOW,
    );
    for name in &dotfiles_allow {
        validate_dotfile_name(name).with_context(|| r.invalid("publish.dotfiles_allow"))?;
    }

    Ok(PublishConfig {
        nip05,
        keep_versions,
        check_dotfiles,
        check_size,
        check_unchanged,
        dotfiles_allow,
    })
}

fn resolve_dashboard<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: DashboardFile,
    base: Option<&Path>,
) -> Result<DashboardConfig> {
    let listen = r.parse(
        "dashboard.listen",
        file.listen,
        parse_dashboard_listen,
        SocketAddr::from(([127, 0, 0, 1], 8082)),
    )?;
    let ui = r.typed("dashboard.ui", file.ui, parse_bool, true)?;
    let allowed_hosts = r.list("dashboard.allowed_hosts", file.allowed_hosts, false, &[]);
    let public_url = r.opt("dashboard.public_url", file.public_url, parse_public_url)?;
    let gateway = r.parse(
        "dashboard.gateway",
        file.gateway,
        parse_dashboard_gateway,
        "http://localhost:8080".to_string(),
    )?;
    let gateway = Some(gateway).filter(|s| !s.is_empty());

    let custom_css = r.opt_path("dashboard.custom_css", file.custom_css, base);
    let desktop_page = r.opt_path("dashboard.desktop_page", file.desktop_page, base);
    let desktop_page_css = r.opt_path("dashboard.desktop_page_css", file.desktop_page_css, base);
    let desktop_banner = r.opt_path("dashboard.desktop_banner", file.desktop_banner, base);
    let mascots_dir = r.opt_path("dashboard.mascots_dir", file.mascots_dir, base);

    let max_upload = r.parse(
        "dashboard.max_upload",
        file.max_upload,
        parse_size,
        2 * (1u64 << 30),
    )?;
    let max_upload = r.positive("dashboard.max_upload", max_upload)?;

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
        mascots_dir,
        max_upload,
    })
}

fn resolve_kubo<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: KuboFile,
    state_dir: &Path,
    max_total_storage: u64,
    base: Option<&Path>,
) -> Result<KuboConfig> {
    let managed = r.typed("kubo.managed", file.managed, parse_bool, true)?;
    let binary = r.opt_path("kubo.binary", file.binary, base);

    let repo = match r.pick("kubo.repo", file.repo) {
        Picked::Env(v) => PathBuf::from(v.trim()),
        Picked::File(v) => r.rebase("kubo.repo", base, PathBuf::from(v.trim())),
        Picked::Default => state_dir.join("kubo"),
    };

    let storage_max = r.parse(
        "kubo.storage_max",
        file.storage_max,
        parse_size,
        max_total_storage,
    )?;

    let provide_strategy = r.string("kubo.provide_strategy", file.provide_strategy, "pinned+mfs");
    if provide_strategy.trim().is_empty() {
        bail!("{} must not be empty", r.name("kubo.provide_strategy"));
    }

    let gateway_listen = r.parse(
        "kubo.gateway_listen",
        file.gateway_listen,
        |v| {
            v.trim()
                .parse::<SocketAddr>()
                .with_context(|| format!("invalid gateway listen address: {v}"))
        },
        SocketAddr::from(([127, 0, 0, 1], 8080)),
    )?;

    let swarm_port = r.opt_typed("kubo.swarm_port", file.swarm_port, |v| u16_value(v.trim()))?;
    if swarm_port == Some(0) {
        bail!("{} must be between 1 and 65535", r.name("kubo.swarm_port"));
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

fn resolve_gateway<E: Fn(&str) -> Option<String>>(
    r: &mut Resolver<E>,
    file: GatewayFile,
    kubo_managed: bool,
    kubo_gateway_listen: SocketAddr,
) -> Result<GatewayConfig> {
    let listen = r.parse("gateway.listen", file.listen, parse_listen, Listen::Off)?;

    let hosts = r.list("gateway.hosts", file.hosts, true, &[]);
    for host in &hosts {
        if !is_valid_gateway_host(host) {
            bail!("invalid {} entry: {host}", r.name("gateway.hosts"));
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
    let upstream = r.string("gateway.upstream", file.upstream, &upstream_default);

    Ok(GatewayConfig {
        listen,
        hosts,
        upstream,
    })
}

// Peer HTML served by the gateway must never be same-site with the dashboard (cookies ignore ports).
fn check_gateway_hosts_apart_from_dashboard(
    gateway_hosts: &[String],
    allowed_hosts: &[String],
) -> Result<()> {
    for host in gateway_hosts {
        if ["localhost", "127.0.0.1"].contains(&host.as_str())
            || allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
        {
            bail!(
                "[gateway].hosts entry {host} is also a dashboard host; serve the dashboard and the gateway under different host names"
            );
        }
    }
    Ok(())
}

pub(super) fn build_config(
    file: ConfigFile,
    base: Option<&Path>,
    get_env: impl Fn(&str) -> Option<String>,
) -> Result<Config> {
    let mut r = Resolver::new(get_env);

    let nostr = resolve_nostr(&mut r, file.nostr)?;
    let policy = resolve_policy(&mut r, file.policy)?;
    let agent = resolve_agent(&mut r, file.agent, base)?;
    let kubo = resolve_kubo(
        &mut r,
        file.kubo,
        &agent.state_dir,
        policy.max_total_storage,
        base,
    )?;
    let ipfs = resolve_ipfs(&mut r, file.ipfs, kubo.managed)?;
    let publish = resolve_publish(&mut r, file.publish)?;
    let dashboard = resolve_dashboard(&mut r, file.dashboard, base)?;
    let gateway = resolve_gateway(&mut r, file.gateway, kubo.managed, kubo.gateway_listen)?;
    check_gateway_hosts_apart_from_dashboard(&gateway.hosts, &dashboard.allowed_hosts)?;

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
        sources: r.sources,
    })
}

#[cfg(test)]
mod tests;
