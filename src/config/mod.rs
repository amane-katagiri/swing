use std::collections::BTreeMap;
use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

mod build;

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
pub enum CheckMode {
    Off,
    Warn,
    Require,
}

impl CheckMode {
    const ALL: [CheckMode; 3] = [CheckMode::Off, CheckMode::Warn, CheckMode::Require];

    pub const fn name(self) -> &'static str {
        match self {
            CheckMode::Off => "off",
            CheckMode::Warn => "warn",
            CheckMode::Require => "require",
        }
    }
}

pub const CHECK_MODE_NAMES: [&str; 3] = [
    CheckMode::Off.name(),
    CheckMode::Warn.name(),
    CheckMode::Require.name(),
];

pub fn parse_check_mode(input: &str) -> Result<CheckMode> {
    let trimmed = input.trim().to_ascii_lowercase();
    CheckMode::ALL
        .into_iter()
        .find(|mode| trimmed == mode.name())
        .with_context(|| format!("invalid mode: {trimmed} (expected off, warn, or require)"))
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AgentFile {
    pub state_dir: Option<String>,
    pub poll_interval: Option<String>,
    pub fetch_timeout: Option<String>,
    pub fetch_idle_timeout: Option<String>,
    pub concurrency: Option<usize>,
    pub report_ttl: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PublishFile {
    pub nip05: Option<String>,
    pub keep_versions: Option<usize>,
    pub check_dotfiles: Option<String>,
    pub check_size: Option<String>,
    pub check_unchanged: Option<String>,
    pub dotfiles_allow: Option<Vec<String>>,
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
    pub public_url: Option<String>,
    pub gateway: Option<String>,
    pub custom_css: Option<String>,
    pub desktop_page: Option<String>,
    pub desktop_page_css: Option<String>,
    pub desktop_banner: Option<String>,
    pub mascots_dir: Option<String>,
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
    pub secret_key: Option<NostrSecretKey>,
    pub relays: Vec<String>,
    pub mirror_set: String,
    pub site_event_kind: u16,
    pub replica_event_kind: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Env,
    File,
    Default,
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
    pub nip05: CheckMode,
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
    pub nip05: CheckMode,
    pub keep_versions: usize,
    pub check_dotfiles: CheckMode,
    pub check_size: CheckMode,
    pub check_unchanged: CheckMode,
    pub dotfiles_allow: Vec<String>,
}

pub const DEFAULT_DOTFILES_ALLOW: [&str; 5] =
    [".well-known", ".nojekyll", ".gitkeep", ".keep", ".domains"];

pub fn validate_dotfile_name(name: &str) -> Result<()> {
    if !name.starts_with('.') || name == "." || name == ".." || name.contains('/') {
        bail!(
            "invalid dotfile name: {name:?} (expected a single name starting with \".\", such as .nojekyll)"
        );
    }
    Ok(())
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
    pub public_url: Option<String>,
    pub gateway: Option<String>,
    pub custom_css: Option<PathBuf>,
    pub desktop_page: Option<PathBuf>,
    pub desktop_page_css: Option<PathBuf>,
    pub desktop_banner: Option<PathBuf>,
    pub mascots_dir: Option<PathBuf>,
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
    pub config_path: PathBuf,
    pub config_exists: bool,
    pub sources: BTreeMap<String, Source>,
}

pub fn resolve_config_path(cli_path: Option<&Path>) -> PathBuf {
    if let Some(p) = cli_path {
        return p.to_path_buf();
    }
    if let Some(p) = env_var("SWING_CONFIG") {
        return PathBuf::from(p);
    }
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join("swing.toml")
}

fn load_file(cli_path: Option<&Path>) -> Result<(ConfigFile, PathBuf, bool)> {
    let path = resolve_config_path(cli_path);
    if !path.exists() {
        if cli_path.is_some() || env_var("SWING_CONFIG").is_some() {
            bail!("config file not found: {}", path.display());
        }
        return Ok((ConfigFile::default(), path, false));
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading config file {}", path.display()))?;
    let file: ConfigFile =
        toml::from_str(&text).with_context(|| format!("parsing config file {}", path.display()))?;
    Ok((file, path, true))
}

pub(crate) fn env_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn parse_size(input: &str) -> Result<u64> {
    let s = input.trim();
    if s.is_empty() {
        bail!("empty size value");
    }
    let upper = s.to_ascii_uppercase();
    const UNITS: [(&str, u64); 8] = [
        ("TIB", 1u64 << 40),
        ("GIB", 1u64 << 30),
        ("MIB", 1u64 << 20),
        ("KIB", 1u64 << 10),
        ("TB", 1u64 << 40),
        ("GB", 1u64 << 30),
        ("MB", 1u64 << 20),
        ("KB", 1u64 << 10),
    ];
    let (num_part, mult): (&str, u64) = UNITS
        .iter()
        .find_map(|&(unit, mult)| upper.strip_suffix(unit).map(|p| (p, mult)))
        .or_else(|| upper.strip_suffix('B').map(|p| (p, 1)))
        .unwrap_or((upper.as_str(), 1));
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

pub fn parse_public_url(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    let authority = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"));
    match authority {
        Some(a) if !a.is_empty() && !a.contains(['/', '?', '#', ' ']) => Ok(trimmed.to_string()),
        _ => bail!("dashboard public URL must be http(s)://host[:port] without a path: {input}"),
    }
}

pub(crate) fn build_config_from_str(
    text: &str,
    get_env: impl Fn(&str) -> Option<String>,
) -> Result<Config> {
    let file: ConfigFile = toml::from_str(text).context("parsing config file")?;
    build::build_config(file, None, get_env)
}

pub fn parse_bool(input: &str) -> Result<bool> {
    match input.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => bail!("invalid boolean value: {other}"),
    }
}

impl Config {
    pub fn ipfs_api_url(&self) -> Result<String> {
        match &self.ipfs.api {
            IpfsApi::Url(url) => Ok(url.clone()),
            IpfsApi::Managed => crate::kubo::api_url_from_repo(&self.kubo.repo),
        }
    }

    pub fn load(cli_path: Option<&Path>) -> Result<Self> {
        let (file, config_path, config_exists) = load_file(cli_path)?;
        let base = if config_exists {
            let absolute = std::path::absolute(&config_path)
                .with_context(|| format!("resolving config file path {}", config_path.display()))?;
            absolute.parent().map(Path::to_path_buf)
        } else {
            None
        };
        let mut config = build::build_config(file, base.as_deref(), env_var)?;
        config.config_path = config_path;
        config.config_exists = config_exists;
        Ok(config)
    }

    pub fn source_of(&self, key: &str) -> Option<Source> {
        self.sources.get(key).copied()
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
    fn parse_size_binary_units_match_the_legacy_units() {
        assert_eq!(parse_size("100GiB").unwrap(), 100 * (1u64 << 30));
        assert_eq!(parse_size("100GiB").unwrap(), parse_size("100GB").unwrap());
        assert_eq!(parse_size("512MiB").unwrap(), 512 * (1u64 << 20));
        assert_eq!(parse_size("1TiB").unwrap(), 1u64 << 40);
        assert_eq!(parse_size("2KiB").unwrap(), 2 * (1u64 << 10));
        assert_eq!(parse_size("1.5KiB").unwrap(), 1536);
        assert_eq!(parse_size("100gib").unwrap(), 100 * (1u64 << 30));
        assert_eq!(parse_size("100 GIB").unwrap(), 100 * (1u64 << 30));
        assert!(parse_size("GiB").is_err());
        assert!(parse_size("5iB").is_err());
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
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
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

    #[test]
    fn resolve_config_path_prefers_cli_over_env_and_default() {
        let cli = PathBuf::from("/tmp/from-cli.toml");
        assert_eq!(resolve_config_path(Some(&cli)), cli);
    }

    #[test]
    fn resolve_config_path_defaults_to_swing_toml_in_cwd_even_when_absent() {
        let path = resolve_config_path(None);
        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap(), "swing.toml");
    }

    #[test]
    fn missing_config_file_without_explicit_path_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let orig = env::current_dir().unwrap();
        env::set_current_dir(dir.path()).unwrap();
        let cwd = env::current_dir().unwrap();
        let result = load_file(None);
        env::set_current_dir(orig).unwrap();
        let (file, path, exists) = result.unwrap();
        assert!(!exists);
        assert_eq!(path, cwd.join("swing.toml"));
        assert!(file.nostr.secret_key.is_none());
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
        ] {
            assert!(parse_public_url(bad).is_err(), "{bad}");
        }
    }
}
