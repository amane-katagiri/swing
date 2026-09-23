use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const DEFAULT_RELAYS: [&str; 5] = [
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
    "wss://yabu.me",
    "wss://relay-jp.nostr.wirednet.jp",
];

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct NostrFile {
    pub secret_key: Option<String>,
    pub relays: Option<Vec<String>>,
    pub mirror_set: Option<String>,
    pub site_event_kind: Option<u16>,
    pub replica_event_kind: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct IpfsFile {
    pub api: Option<String>,
    pub mfs_root: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PolicyFile {
    pub max_total_storage: Option<String>,
    pub max_per_site: Option<String>,
    pub max_per_account: Option<String>,
    pub max_sites_per_account: Option<usize>,
    pub max_update_size: Option<String>,
    pub keep_versions: Option<usize>,
    pub keep_days: Option<u64>,
    pub min_update_interval: Option<String>,
    pub remove_on_unfollow: Option<bool>,
    pub nip05: Option<String>,
    pub nip05_cache_ttl: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nip05Mode {
    Off,
    Warn,
    Require,
}

pub fn parse_nip05_mode(input: &str) -> Result<Nip05Mode> {
    match input.trim().to_ascii_lowercase().as_str() {
        "off" => Ok(Nip05Mode::Off),
        "warn" => Ok(Nip05Mode::Warn),
        "require" => Ok(Nip05Mode::Require),
        other => bail!("invalid nip05 mode: {other} (expected off, warn, or require)"),
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AgentFile {
    pub state_dir: Option<String>,
    pub poll_interval: Option<String>,
    pub concurrency: Option<usize>,
    pub report_ttl: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PublishFile {
    pub nip05: Option<String>,
    pub keep_versions: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct KuboFile {
    pub managed: Option<bool>,
    pub binary: Option<String>,
    pub repo: Option<String>,
    pub storage_max: Option<String>,
    pub provide_strategy: Option<String>,
    pub gateway_listen: Option<String>,
    pub swarm_port: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct GatewayFile {
    pub listen: Option<String>,
    pub hosts: Option<Vec<String>>,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct DashboardFile {
    pub listen: Option<String>,
    pub ui: Option<bool>,
    pub allowed_hosts: Option<Vec<String>>,
    pub gateway: Option<String>,
    pub custom_css: Option<String>,
    pub desktop_page: Option<String>,
    pub desktop_page_css: Option<String>,
    pub desktop_banner: Option<String>,
    pub max_upload: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ConfigFile {
    pub nostr: NostrFile,
    pub ipfs: IpfsFile,
    pub policy: PolicyFile,
    pub agent: AgentFile,
    pub publish: PublishFile,
    pub dashboard: DashboardFile,
    pub kubo: KuboFile,
    pub gateway: GatewayFile,
}

#[derive(Clone)]
pub struct NostrSecretKey(String);

impl NostrSecretKey {
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for NostrSecretKey {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for NostrSecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl From<String> for NostrSecretKey {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone)]
pub struct NostrConfig {
    pub secret_key: NostrSecretKey,
    pub relays: Vec<String>,
    pub mirror_set: String,
    pub site_event_kind: u16,
    pub replica_event_kind: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpfsApi {
    Url(String),
    Managed,
}

#[derive(Debug, Clone)]
pub struct IpfsConfig {
    pub api: IpfsApi,
    pub mfs_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KuboConfig {
    pub managed: bool,
    pub binary: Option<PathBuf>,
    pub repo: PathBuf,
    pub storage_max: u64,
    pub provide_strategy: String,
    pub gateway_listen: SocketAddr,
    pub swarm_port: Option<u16>,
}

pub fn is_valid_gateway_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyConfig {
    pub max_total_storage: u64,
    pub max_per_site: u64,
    pub max_per_account: u64,
    pub max_sites_per_account: usize,
    pub max_update_size: u64,
    pub keep_versions: usize,
    pub keep_days: u64,
    pub min_update_interval: u64,
    pub remove_on_unfollow: bool,
    pub nip05: Nip05Mode,
    pub nip05_cache_ttl: u64,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub state_dir: PathBuf,
    pub poll_interval: Duration,
    pub fetch_timeout: Duration,
    pub fetch_idle_timeout: Duration,
    pub concurrency: usize,
    pub report_ttl: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishConfig {
    pub nip05: Nip05Mode,
    pub keep_versions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listen {
    Off,
    Addr(SocketAddr),
}

pub fn parse_listen(input: &str) -> Result<Listen> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("off") {
        return Ok(Listen::Off);
    }
    trimmed
        .parse::<SocketAddr>()
        .map(Listen::Addr)
        .with_context(|| format!("invalid listen address: {trimmed}"))
}

pub fn parse_dashboard_listen(input: &str) -> Result<SocketAddr> {
    let trimmed = input.trim();
    trimmed
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid listen address: {trimmed}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayConfig {
    pub listen: Listen,
    pub hosts: Vec<String>,
    pub upstream: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardConfig {
    pub listen: SocketAddr,
    pub ui: bool,
    pub allowed_hosts: Vec<String>,
    pub gateway: Option<String>,
    pub custom_css: Option<PathBuf>,
    pub desktop_page: Option<PathBuf>,
    pub desktop_page_css: Option<PathBuf>,
    pub desktop_banner: Option<PathBuf>,
    pub max_upload: u64,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub nostr: NostrConfig,
    pub ipfs: IpfsConfig,
    pub policy: PolicyConfig,
    pub agent: AgentConfig,
    pub publish: PublishConfig,
    pub dashboard: DashboardConfig,
    pub kubo: KuboConfig,
    pub gateway: GatewayConfig,
    pub config_path: Option<PathBuf>,
}

pub fn resolve_config_path(cli_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = cli_path {
        return Some(p.to_path_buf());
    }
    if let Ok(p) = env::var("SWING_CONFIG") {
        return Some(PathBuf::from(p));
    }
    let default = PathBuf::from("./swing.toml");
    if default.exists() {
        return Some(default);
    }
    None
}

fn load_file(cli_path: Option<&Path>) -> Result<(ConfigFile, Option<PathBuf>)> {
    match resolve_config_path(cli_path) {
        Some(path) => {
            if !path.exists() {
                if cli_path.is_some() || env::var("SWING_CONFIG").is_ok() {
                    bail!("config file not found: {}", path.display());
                }
                return Ok((ConfigFile::default(), None));
            }
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading config file {}", path.display()))?;
            let file: ConfigFile = toml::from_str(&text)
                .with_context(|| format!("parsing config file {}", path.display()))?;
            Ok((file, Some(path)))
        }
        None => Ok((ConfigFile::default(), None)),
    }
}

fn env_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

/// Resolves a value whose TOML representation is a raw string needing the
/// same `parse` as the environment variable, trying the environment first.
fn resolve<T>(
    get_env: &impl Fn(&str) -> Option<String>,
    env_key: &str,
    file_val: Option<String>,
    parse: impl Fn(&str) -> Result<T>,
    env_ctx: &str,
    file_ctx: &str,
    default: T,
) -> Result<T> {
    match get_env(env_key) {
        Some(v) => parse(&v).context(env_ctx.to_string()),
        None => match file_val {
            Some(v) => parse(&v).context(file_ctx.to_string()),
            None => Ok(default),
        },
    }
}

/// Resolves a value whose TOML representation is already the target type,
/// only the environment variable needs `parse`.
fn resolve_typed<T>(
    get_env: &impl Fn(&str) -> Option<String>,
    env_key: &str,
    file_val: Option<T>,
    parse: impl Fn(&str) -> Result<T>,
    env_ctx: &str,
    default: T,
) -> Result<T> {
    match get_env(env_key) {
        Some(v) => parse(&v).context(env_ctx.to_string()),
        None => Ok(file_val.unwrap_or(default)),
    }
}

pub fn parse_size(input: &str) -> Result<u64> {
    let s = input.trim();
    if s.is_empty() {
        bail!("empty size value");
    }
    let upper = s.to_ascii_uppercase();
    let (num_part, mult): (&str, u64) = if let Some(p) = upper.strip_suffix("TB") {
        (p, 1u64 << 40)
    } else if let Some(p) = upper.strip_suffix("GB") {
        (p, 1u64 << 30)
    } else if let Some(p) = upper.strip_suffix("MB") {
        (p, 1u64 << 20)
    } else if let Some(p) = upper.strip_suffix("KB") {
        (p, 1u64 << 10)
    } else if let Some(p) = upper.strip_suffix('B') {
        (p, 1)
    } else {
        (upper.as_str(), 1)
    };
    let num_part = num_part.trim();
    if !num_part.chars().all(|c| c.is_ascii_digit() || c == '.') {
        bail!("invalid size value: {input}");
    }
    let value: f64 = num_part
        .parse()
        .with_context(|| format!("invalid size value: {input}"))?;
    let bytes = value * mult as f64;
    if bytes >= u64::MAX as f64 {
        bail!("size value too large: {input}");
    }
    Ok(bytes as u64)
}

pub fn parse_duration_secs(input: &str) -> Result<u64> {
    let s = input.trim();
    if s.is_empty() {
        bail!("empty duration value");
    }
    let lower = s.to_ascii_lowercase();
    let (num_part, mult): (&str, u64) = if let Some(p) = lower.strip_suffix('d') {
        (p, 86_400)
    } else if let Some(p) = lower.strip_suffix('h') {
        (p, 3_600)
    } else if let Some(p) = lower.strip_suffix('m') {
        (p, 60)
    } else if let Some(p) = lower.strip_suffix('s') {
        (p, 1)
    } else {
        (lower.as_str(), 1)
    };
    let value: u64 = num_part
        .trim()
        .parse()
        .with_context(|| format!("invalid duration value: {input}"))?;
    value
        .checked_mul(mult)
        .with_context(|| format!("duration value too large: {input}"))
}

pub fn parse_mfs_root(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    let Some(rest) = trimmed.strip_prefix('/') else {
        bail!("MFS root must be an absolute path: {input}");
    };
    if rest
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        bail!("MFS root must be a non-root path without empty, . or .. segments: {input}");
    }
    Ok(trimmed.to_string())
}

fn parse_bool(input: &str) -> Result<bool> {
    match input.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => bail!("invalid boolean value: {other}"),
    }
}

fn build_config(file: ConfigFile, get_env: impl Fn(&str) -> Option<String>) -> Result<Config> {
    let secret_key = get_env("SWING_NOSTR_SECRET_KEY")
        .or(file.nostr.secret_key)
        .context("missing Nostr secret key: set SWING_NOSTR_SECRET_KEY or [nostr].secret_key")?;

    let relays = match get_env("SWING_NOSTR_RELAYS") {
        Some(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => file.nostr.relays.unwrap_or_default(),
    };
    let relays = if relays.is_empty() {
        DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
    } else {
        relays
    };

    let mirror_set = get_env("SWING_MIRROR_SET")
        .or(file.nostr.mirror_set)
        .unwrap_or_else(|| "swing".to_string());

    let site_event_kind = resolve_typed(
        &get_env,
        "SWING_SITE_EVENT_KIND",
        file.nostr.site_event_kind,
        |v| v.parse().context("expected u16"),
        "invalid SWING_SITE_EVENT_KIND: expected u16",
        35980,
    )?;

    let replica_event_kind = resolve_typed(
        &get_env,
        "SWING_REPLICA_EVENT_KIND",
        file.nostr.replica_event_kind,
        |v| v.parse().context("expected u16"),
        "invalid SWING_REPLICA_EVENT_KIND: expected u16",
        35981,
    )?;

    let kubo_managed = resolve_typed(
        &get_env,
        "SWING_KUBO_MANAGED",
        file.kubo.managed,
        parse_bool,
        "invalid SWING_KUBO_MANAGED",
        true,
    )?;

    let ipfs_api_file = get_env("SWING_IPFS_API").or(file.ipfs.api);
    if kubo_managed && ipfs_api_file.is_some() {
        bail!("[ipfs].api conflicts with [kubo].managed = true");
    }
    let ipfs_api = if kubo_managed {
        IpfsApi::Managed
    } else {
        IpfsApi::Url(ipfs_api_file.unwrap_or_else(|| "http://127.0.0.1:5001".to_string()))
    };

    let mfs_root = resolve(
        &get_env,
        "SWING_MFS_ROOT",
        file.ipfs.mfs_root,
        parse_mfs_root,
        "invalid SWING_MFS_ROOT",
        "invalid [ipfs].mfs_root",
        "/swing".to_string(),
    )?;

    let max_total_storage = resolve(
        &get_env,
        "SWING_MAX_TOTAL_STORAGE",
        file.policy.max_total_storage,
        parse_size,
        "invalid SWING_MAX_TOTAL_STORAGE",
        "invalid [policy].max_total_storage",
        100 * (1u64 << 30),
    )?;

    let max_per_site = resolve(
        &get_env,
        "SWING_MAX_PER_SITE",
        file.policy.max_per_site,
        parse_size,
        "invalid SWING_MAX_PER_SITE",
        "invalid [policy].max_per_site",
        10 * (1u64 << 30),
    )?;

    let max_per_account = resolve(
        &get_env,
        "SWING_MAX_PER_ACCOUNT",
        file.policy.max_per_account,
        parse_size,
        "invalid SWING_MAX_PER_ACCOUNT",
        "invalid [policy].max_per_account",
        20 * (1u64 << 30),
    )?;

    let max_sites_per_account = resolve_typed(
        &get_env,
        "SWING_MAX_SITES_PER_ACCOUNT",
        file.policy.max_sites_per_account,
        |v| v.parse().context("expected integer"),
        "invalid SWING_MAX_SITES_PER_ACCOUNT: expected integer",
        10,
    )?;
    if max_sites_per_account == 0 {
        bail!("max_sites_per_account must be greater than 0");
    }

    let max_update_size = resolve(
        &get_env,
        "SWING_MAX_UPDATE_SIZE",
        file.policy.max_update_size,
        parse_size,
        "invalid SWING_MAX_UPDATE_SIZE",
        "invalid [policy].max_update_size",
        2 * (1u64 << 30),
    )?;

    let keep_versions = resolve_typed(
        &get_env,
        "SWING_KEEP_VERSIONS",
        file.policy.keep_versions,
        |v| v.parse().context("expected integer"),
        "invalid SWING_KEEP_VERSIONS: expected integer",
        5,
    )?;

    let keep_days = resolve_typed(
        &get_env,
        "SWING_KEEP_DAYS",
        file.policy.keep_days,
        |v| v.parse().context("expected integer"),
        "invalid SWING_KEEP_DAYS: expected integer",
        365,
    )?;

    let min_update_interval = resolve(
        &get_env,
        "SWING_MIN_UPDATE_INTERVAL",
        file.policy.min_update_interval,
        parse_duration_secs,
        "invalid SWING_MIN_UPDATE_INTERVAL",
        "invalid [policy].min_update_interval",
        3600,
    )?;

    let remove_on_unfollow = resolve_typed(
        &get_env,
        "SWING_REMOVE_ON_UNFOLLOW",
        file.policy.remove_on_unfollow,
        parse_bool,
        "invalid SWING_REMOVE_ON_UNFOLLOW",
        true,
    )?;

    let nip05 = resolve(
        &get_env,
        "SWING_NIP05",
        file.policy.nip05,
        parse_nip05_mode,
        "invalid SWING_NIP05",
        "invalid [policy].nip05",
        Nip05Mode::Warn,
    )?;

    let nip05_cache_ttl = resolve(
        &get_env,
        "SWING_NIP05_CACHE_TTL",
        file.policy.nip05_cache_ttl,
        parse_duration_secs,
        "invalid SWING_NIP05_CACHE_TTL",
        "invalid [policy].nip05_cache_ttl",
        86_400,
    )?;

    let state_dir = get_env("SWING_STATE_DIR")
        .or(file.agent.state_dir)
        .unwrap_or_else(|| "./data".to_string());

    let poll_interval = resolve(
        &get_env,
        "SWING_POLL_INTERVAL",
        file.agent.poll_interval,
        parse_duration_secs,
        "invalid SWING_POLL_INTERVAL",
        "invalid [agent].poll_interval",
        300,
    )?;
    if poll_interval == 0 {
        bail!("poll_interval must be greater than 0");
    }

    let fetch_timeout = resolve_typed(
        &get_env,
        "SWING_FETCH_TIMEOUT",
        None,
        parse_duration_secs,
        "invalid SWING_FETCH_TIMEOUT",
        900,
    )?;
    if fetch_timeout == 0 {
        bail!("SWING_FETCH_TIMEOUT must be greater than 0");
    }

    let fetch_idle_timeout = resolve_typed(
        &get_env,
        "SWING_FETCH_IDLE_TIMEOUT",
        None,
        parse_duration_secs,
        "invalid SWING_FETCH_IDLE_TIMEOUT",
        120,
    )?;
    if fetch_idle_timeout == 0 {
        bail!("SWING_FETCH_IDLE_TIMEOUT must be greater than 0");
    }

    let concurrency = resolve_typed(
        &get_env,
        "SWING_CONCURRENCY",
        file.agent.concurrency,
        |v| v.parse().context("expected integer"),
        "invalid SWING_CONCURRENCY: expected integer",
        4,
    )?;
    if concurrency == 0 {
        bail!("concurrency must be greater than 0");
    }

    let report_ttl = resolve(
        &get_env,
        "SWING_REPORT_TTL",
        file.agent.report_ttl,
        parse_duration_secs,
        "invalid SWING_REPORT_TTL",
        "invalid [agent].report_ttl",
        3 * 86_400,
    )?;
    if report_ttl / 2 <= poll_interval {
        bail!("report_ttl must be more than twice poll_interval");
    }

    let publish_nip05 = resolve(
        &get_env,
        "SWING_PUBLISH_NIP05",
        file.publish.nip05,
        parse_nip05_mode,
        "invalid SWING_PUBLISH_NIP05",
        "invalid [publish].nip05",
        Nip05Mode::Warn,
    )?;

    let publish_keep_versions = resolve_typed(
        &get_env,
        "SWING_PUBLISH_KEEP_VERSIONS",
        file.publish.keep_versions,
        |v| v.parse().context("expected integer"),
        "invalid SWING_PUBLISH_KEEP_VERSIONS: expected integer",
        5,
    )?;
    if publish_keep_versions == 0 {
        bail!("publish keep_versions must be greater than 0");
    }

    let dashboard_listen = resolve(
        &get_env,
        "SWING_DASHBOARD_LISTEN",
        file.dashboard.listen,
        parse_dashboard_listen,
        "invalid SWING_DASHBOARD_LISTEN",
        "invalid [dashboard].listen",
        SocketAddr::from(([127, 0, 0, 1], 8082)),
    )?;

    let dashboard_ui = resolve_typed(
        &get_env,
        "SWING_DASHBOARD_UI",
        file.dashboard.ui,
        parse_bool,
        "invalid SWING_DASHBOARD_UI",
        true,
    )?;

    let dashboard_allowed_hosts = match get_env("SWING_DASHBOARD_ALLOWED_HOSTS") {
        Some(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => file.dashboard.allowed_hosts.unwrap_or_default(),
    };

    let dashboard_gateway = match get_env("SWING_DASHBOARD_GATEWAY") {
        Some(v) => Some(v),
        None => Some(
            file.dashboard
                .gateway
                .unwrap_or_else(|| "http://localhost:8080".to_string()),
        ),
    }
    .filter(|s| !s.is_empty());

    let dashboard_custom_css = match get_env("SWING_DASHBOARD_CUSTOM_CSS") {
        Some(v) => Some(PathBuf::from(v)),
        None => file.dashboard.custom_css.map(PathBuf::from),
    };

    let dashboard_desktop_page = match get_env("SWING_DASHBOARD_DESKTOP_PAGE") {
        Some(v) => Some(PathBuf::from(v)),
        None => file.dashboard.desktop_page.map(PathBuf::from),
    };

    let dashboard_desktop_page_css = match get_env("SWING_DASHBOARD_DESKTOP_PAGE_CSS") {
        Some(v) => Some(PathBuf::from(v)),
        None => file.dashboard.desktop_page_css.map(PathBuf::from),
    };

    let dashboard_desktop_banner = match get_env("SWING_DASHBOARD_DESKTOP_BANNER") {
        Some(v) => Some(PathBuf::from(v)),
        None => file.dashboard.desktop_banner.map(PathBuf::from),
    };

    let dashboard_max_upload = resolve(
        &get_env,
        "SWING_DASHBOARD_MAX_UPLOAD",
        file.dashboard.max_upload,
        parse_size,
        "invalid SWING_DASHBOARD_MAX_UPLOAD",
        "invalid [dashboard].max_upload",
        2 * (1u64 << 30),
    )?;
    if dashboard_max_upload == 0 {
        bail!("dashboard max_upload must be greater than 0");
    }

    let kubo_repo_default = PathBuf::from(&state_dir).join("kubo");
    let kubo_repo = resolve(
        &get_env,
        "SWING_KUBO_REPO",
        file.kubo.repo,
        |v| Ok(PathBuf::from(v.trim())),
        "invalid SWING_KUBO_REPO",
        "invalid [kubo].repo",
        kubo_repo_default,
    )?;

    let kubo_binary = match get_env("SWING_KUBO_BINARY") {
        Some(v) => Some(PathBuf::from(v)),
        None => file.kubo.binary.map(PathBuf::from),
    };

    let kubo_storage_max = resolve(
        &get_env,
        "SWING_KUBO_STORAGE_MAX",
        file.kubo.storage_max,
        parse_size,
        "invalid SWING_KUBO_STORAGE_MAX",
        "invalid [kubo].storage_max",
        max_total_storage,
    )?;

    let kubo_provide_strategy = get_env("SWING_KUBO_PROVIDE_STRATEGY")
        .or(file.kubo.provide_strategy)
        .unwrap_or_else(|| "pinned+mfs".to_string());
    if kubo_provide_strategy.trim().is_empty() {
        bail!("[kubo].provide_strategy must not be empty");
    }

    let kubo_gateway_listen = resolve(
        &get_env,
        "SWING_KUBO_GATEWAY_LISTEN",
        file.kubo.gateway_listen,
        |v| {
            v.trim()
                .parse::<SocketAddr>()
                .with_context(|| format!("invalid gateway listen address: {v}"))
        },
        "invalid SWING_KUBO_GATEWAY_LISTEN",
        "invalid [kubo].gateway_listen",
        SocketAddr::from(([127, 0, 0, 1], 8080)),
    )?;

    let kubo_swarm_port = match get_env("SWING_KUBO_SWARM_PORT") {
        Some(v) => Some(
            v.trim()
                .parse::<u16>()
                .context("invalid SWING_KUBO_SWARM_PORT: expected u16")?,
        ),
        None => file.kubo.swarm_port,
    };
    if kubo_swarm_port == Some(0) {
        bail!("[kubo].swarm_port must be between 1 and 65535");
    }

    let gateway_listen = resolve(
        &get_env,
        "SWING_GATEWAY_LISTEN",
        file.gateway.listen,
        parse_listen,
        "invalid SWING_GATEWAY_LISTEN",
        "invalid [gateway].listen",
        Listen::Off,
    )?;

    let gateway_hosts: Vec<String> = match get_env("SWING_GATEWAY_HOSTS") {
        Some(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => file
            .gateway
            .hosts
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    };
    for host in &gateway_hosts {
        if !is_valid_gateway_host(host) {
            bail!("invalid [gateway].hosts entry: {host}");
        }
    }
    if !matches!(gateway_listen, Listen::Off) && gateway_hosts.is_empty() {
        bail!("[gateway].hosts must not be empty when [gateway].listen is enabled");
    }

    let gateway_upstream_default = if kubo_managed {
        format!("http://{kubo_gateway_listen}")
    } else {
        "http://127.0.0.1:8080".to_string()
    };
    let gateway_upstream = get_env("SWING_GATEWAY_UPSTREAM")
        .or(file.gateway.upstream)
        .unwrap_or(gateway_upstream_default);

    Ok(Config {
        nostr: NostrConfig {
            secret_key: secret_key.into(),
            relays,
            mirror_set,
            site_event_kind,
            replica_event_kind,
        },
        ipfs: IpfsConfig {
            api: ipfs_api,
            mfs_root,
        },
        policy: PolicyConfig {
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
        },
        agent: AgentConfig {
            state_dir: PathBuf::from(state_dir),
            poll_interval: Duration::from_secs(poll_interval),
            fetch_timeout: Duration::from_secs(fetch_timeout),
            fetch_idle_timeout: Duration::from_secs(fetch_idle_timeout),
            concurrency,
            report_ttl: Duration::from_secs(report_ttl),
        },
        publish: PublishConfig {
            nip05: publish_nip05,
            keep_versions: publish_keep_versions,
        },
        dashboard: DashboardConfig {
            listen: dashboard_listen,
            ui: dashboard_ui,
            allowed_hosts: dashboard_allowed_hosts,
            gateway: dashboard_gateway,
            custom_css: dashboard_custom_css,
            desktop_page: dashboard_desktop_page,
            desktop_page_css: dashboard_desktop_page_css,
            desktop_banner: dashboard_desktop_banner,
            max_upload: dashboard_max_upload,
        },
        kubo: KuboConfig {
            managed: kubo_managed,
            binary: kubo_binary,
            repo: kubo_repo,
            storage_max: kubo_storage_max,
            provide_strategy: kubo_provide_strategy,
            gateway_listen: kubo_gateway_listen,
            swarm_port: kubo_swarm_port,
        },
        gateway: GatewayConfig {
            listen: gateway_listen,
            hosts: gateway_hosts,
            upstream: gateway_upstream,
        },
        config_path: None,
    })
}

impl Config {
    pub fn ipfs_api_url(&self) -> Result<String> {
        match &self.ipfs.api {
            IpfsApi::Url(url) => Ok(url.clone()),
            IpfsApi::Managed => crate::kubo::api_url_from_repo(&self.kubo.repo),
        }
    }

    pub fn load(cli_path: Option<&Path>) -> Result<Self> {
        let (file, config_path) = load_file(cli_path)?;
        let mut config = build_config(file, env_var)?;
        config.config_path = config_path;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_units() {
        assert_eq!(parse_size("100GB").unwrap(), 100 * (1u64 << 30));
        assert_eq!(parse_size("512MB").unwrap(), 512 * (1u64 << 20));
        assert_eq!(parse_size("1TB").unwrap(), 1u64 << 40);
        assert_eq!(parse_size("2KB").unwrap(), 2 * (1u64 << 10));
        assert_eq!(parse_size("12345").unwrap(), 12345);
        assert_eq!(parse_size("100gb").unwrap(), 100 * (1u64 << 30));
        assert_eq!(parse_size("100Gb").unwrap(), 100 * (1u64 << 30));
        assert_eq!(parse_size(" 100GB ").unwrap(), 100 * (1u64 << 30));
    }

    #[test]
    fn parse_size_rejects_garbage() {
        assert!(parse_size("").is_err());
        assert!(parse_size("GB").is_err());
        assert!(parse_size("-5GB").is_err());
        assert!(parse_size("inf").is_err());
        assert!(parse_size("NaN").is_err());
        assert!(parse_size("1e3").is_err());
        assert!(parse_size("+5GB").is_err());
        assert!(parse_size("99999999999TB").is_err());
    }

    #[test]
    fn parse_size_accepts_fraction() {
        assert_eq!(parse_size("1.5KB").unwrap(), 1536);
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

    #[test]
    fn zero_poll_interval_is_rejected() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_POLL_INTERVAL" => Some("0s".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("poll_interval"));
    }

    #[test]
    fn zero_concurrency_and_idle_timeout_are_rejected() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_CONCURRENCY" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("concurrency"));
        let err = build_config(minimal_file(), |k| match k {
            "SWING_FETCH_IDLE_TIMEOUT" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("SWING_FETCH_IDLE_TIMEOUT"));
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
    fn zero_max_sites_per_account_is_rejected() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_MAX_SITES_PER_ACCOUNT" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("max_sites_per_account"));
    }

    #[test]
    fn mfs_root_is_normalized_and_validated() {
        assert_eq!(parse_mfs_root("/swing").unwrap(), "/swing");
        assert_eq!(parse_mfs_root(" /a/b/ ").unwrap(), "/a/b");
        for bad in ["", "/", "swing", "/a//b", "/a/./b", "/a/../b"] {
            assert!(parse_mfs_root(bad).is_err(), "{bad:?} should be rejected");
        }
        let cfg = build_config(minimal_file(), |k| match k {
            "SWING_MFS_ROOT" => Some("/mirror/".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.ipfs.mfs_root, "/mirror");
    }

    #[test]
    fn zero_publish_keep_versions_is_rejected() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_PUBLISH_KEEP_VERSIONS" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("keep_versions"));
    }

    #[test]
    fn zero_fetch_timeout_is_rejected() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_FETCH_TIMEOUT" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("SWING_FETCH_TIMEOUT"));
    }

    #[test]
    fn missing_config_file_is_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        let err = load_file(Some(&missing)).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn missing_secret_key_is_clear_error() {
        let err = build_config(ConfigFile::default(), |_| None).unwrap_err();
        assert!(err.to_string().contains("secret key"));
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
        assert_eq!(cfg.nostr.secret_key.expose_secret(), "env-key");
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
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = build_config(file, |k| match k {
            "SWING_NIP05" => Some("maybe".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("invalid SWING_NIP05"));
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
        let file = ConfigFile {
            nostr: NostrFile {
                secret_key: Some("k".into()),
                relays: Some(vec!["wss://r".into()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = build_config(file, |k| match k {
            "SWING_PUBLISH_NIP05" => Some("maybe".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("invalid SWING_PUBLISH_NIP05"));
    }

    #[test]
    fn secret_key_debug_is_redacted() {
        let key = NostrSecretKey::from("super-secret-nsec".to_string());
        assert_eq!(format!("{key:?}"), "<redacted>");
        assert_eq!(key.expose_secret(), "super-secret-nsec");
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
        let err = build_config(minimal_file(), |k| match k {
            "SWING_DASHBOARD_MAX_UPLOAD" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("max_upload"));
    }

    #[test]
    fn dashboard_listen_off_is_not_a_valid_address() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_DASHBOARD_LISTEN" => Some("off".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("SWING_DASHBOARD_LISTEN"));
    }

    #[test]
    fn dashboard_listen_rejects_garbage() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_DASHBOARD_LISTEN" => Some("not-an-address".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("SWING_DASHBOARD_LISTEN"));
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
    fn config_load_remembers_the_config_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        assert_eq!(cfg.config_path.as_deref(), Some(path.as_path()));
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
        let err = build_config(minimal_file(), |k| match k {
            "SWING_IPFS_API" => Some("http://127.0.0.1:5001".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("[ipfs].api conflicts"));
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
        let err = build_config(minimal_file(), |k| match k {
            "SWING_KUBO_GATEWAY_LISTEN" => Some("not-an-address".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("SWING_KUBO_GATEWAY_LISTEN"));
    }

    #[test]
    fn kubo_swarm_port_defaults_to_unset() {
        let cfg = build_config(minimal_file(), |_| None).unwrap();
        assert_eq!(cfg.kubo.swarm_port, None);
    }

    #[test]
    fn kubo_swarm_port_zero_is_rejected() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_KUBO_SWARM_PORT" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("swarm_port"));
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
        let err = build_config(minimal_file(), |k| match k {
            "SWING_GATEWAY_LISTEN" => Some("127.0.0.1:8081".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("[gateway].hosts"));
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
        let err = build_config(minimal_file(), |k| match k {
            "SWING_GATEWAY_HOSTS" => Some("Example.com".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("invalid [gateway].hosts entry"));
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
    fn parse_listen_off_and_addr() {
        assert_eq!(parse_listen("off").unwrap(), Listen::Off);
        assert_eq!(parse_listen("OFF").unwrap(), Listen::Off);
        assert_eq!(
            parse_listen("127.0.0.1:8081").unwrap(),
            Listen::Addr(([127, 0, 0, 1], 8081).into())
        );
        assert!(parse_listen("not-an-address").is_err());
    }

    #[test]
    fn resolve_config_path_prefers_cli_over_env_and_default() {
        let cli = PathBuf::from("/tmp/from-cli.toml");
        assert_eq!(resolve_config_path(Some(&cli)), Some(cli));
    }
}
