use std::env;
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
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct IpfsFile {
    pub api: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PolicyFile {
    pub max_total_storage: Option<String>,
    pub max_per_site: Option<String>,
    pub max_per_account: Option<String>,
    pub max_update_size: Option<String>,
    pub keep_versions: Option<usize>,
    pub keep_days: Option<u64>,
    pub min_update_interval: Option<String>,
    pub unpin_on_unfollow: Option<bool>,
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
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PublishFile {
    pub nip05: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ConfigFile {
    pub nostr: NostrFile,
    pub ipfs: IpfsFile,
    pub policy: PolicyFile,
    pub agent: AgentFile,
    pub publish: PublishFile,
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
}

#[derive(Debug, Clone)]
pub struct IpfsConfig {
    pub api: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyConfig {
    pub max_total_storage: u64,
    pub max_per_site: u64,
    pub max_per_account: u64,
    pub max_update_size: u64,
    pub keep_versions: usize,
    pub keep_days: u64,
    pub min_update_interval: u64,
    pub unpin_on_unfollow: bool,
    pub nip05: Nip05Mode,
    pub nip05_cache_ttl: u64,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub state_dir: PathBuf,
    pub poll_interval: Duration,
    pub pin_timeout: Duration,
    pub fetch_idle_timeout: Duration,
    pub concurrency: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishConfig {
    pub nip05: Nip05Mode,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub nostr: NostrConfig,
    pub ipfs: IpfsConfig,
    pub policy: PolicyConfig,
    pub agent: AgentConfig,
    pub publish: PublishConfig,
}

fn resolve_config_path(cli_path: Option<&Path>) -> Option<PathBuf> {
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

fn load_file(cli_path: Option<&Path>) -> Result<ConfigFile> {
    match resolve_config_path(cli_path) {
        Some(path) => {
            if !path.exists() {
                if cli_path.is_some() || env::var("SWING_CONFIG").is_ok() {
                    bail!("config file not found: {}", path.display());
                }
                return Ok(ConfigFile::default());
            }
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading config file {}", path.display()))?;
            let file: ConfigFile = toml::from_str(&text)
                .with_context(|| format!("parsing config file {}", path.display()))?;
            Ok(file)
        }
        None => Ok(ConfigFile::default()),
    }
}

fn env_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

fn process_env(name: &str) -> Option<String> {
    env_var(name)
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

    let site_event_kind = match get_env("SWING_SITE_EVENT_KIND") {
        Some(v) => v
            .parse()
            .context("invalid SWING_SITE_EVENT_KIND: expected u16")?,
        None => file.nostr.site_event_kind.unwrap_or(35980),
    };

    let ipfs_api = get_env("SWING_IPFS_API")
        .or(file.ipfs.api)
        .unwrap_or_else(|| "http://127.0.0.1:5001".to_string());

    let max_total_storage = match get_env("SWING_MAX_TOTAL_STORAGE") {
        Some(v) => parse_size(&v).context("invalid SWING_MAX_TOTAL_STORAGE")?,
        None => match file.policy.max_total_storage {
            Some(v) => parse_size(&v).context("invalid [policy].max_total_storage")?,
            None => 100 * (1u64 << 30),
        },
    };

    let max_per_site = match get_env("SWING_MAX_PER_SITE") {
        Some(v) => parse_size(&v).context("invalid SWING_MAX_PER_SITE")?,
        None => match file.policy.max_per_site {
            Some(v) => parse_size(&v).context("invalid [policy].max_per_site")?,
            None => 10 * (1u64 << 30),
        },
    };

    let max_per_account = match get_env("SWING_MAX_PER_ACCOUNT") {
        Some(v) => parse_size(&v).context("invalid SWING_MAX_PER_ACCOUNT")?,
        None => match file.policy.max_per_account {
            Some(v) => parse_size(&v).context("invalid [policy].max_per_account")?,
            None => 20 * (1u64 << 30),
        },
    };

    let max_update_size = match get_env("SWING_MAX_UPDATE_SIZE") {
        Some(v) => parse_size(&v).context("invalid SWING_MAX_UPDATE_SIZE")?,
        None => match file.policy.max_update_size {
            Some(v) => parse_size(&v).context("invalid [policy].max_update_size")?,
            None => 2 * (1u64 << 30),
        },
    };

    let keep_versions = match get_env("SWING_KEEP_VERSIONS") {
        Some(v) => v
            .parse()
            .context("invalid SWING_KEEP_VERSIONS: expected integer")?,
        None => file.policy.keep_versions.unwrap_or(5),
    };

    let keep_days = match get_env("SWING_KEEP_DAYS") {
        Some(v) => v
            .parse()
            .context("invalid SWING_KEEP_DAYS: expected integer")?,
        None => file.policy.keep_days.unwrap_or(365),
    };

    let min_update_interval = match get_env("SWING_MIN_UPDATE_INTERVAL") {
        Some(v) => parse_duration_secs(&v).context("invalid SWING_MIN_UPDATE_INTERVAL")?,
        None => match file.policy.min_update_interval {
            Some(v) => parse_duration_secs(&v).context("invalid [policy].min_update_interval")?,
            None => 600,
        },
    };

    let unpin_on_unfollow = match get_env("SWING_UNPIN_ON_UNFOLLOW") {
        Some(v) => parse_bool(&v).context("invalid SWING_UNPIN_ON_UNFOLLOW")?,
        None => file.policy.unpin_on_unfollow.unwrap_or(true),
    };

    let nip05 = match get_env("SWING_NIP05") {
        Some(v) => parse_nip05_mode(&v).context("invalid SWING_NIP05")?,
        None => match file.policy.nip05 {
            Some(v) => parse_nip05_mode(&v).context("invalid [policy].nip05")?,
            None => Nip05Mode::Warn,
        },
    };

    let nip05_cache_ttl = match get_env("SWING_NIP05_CACHE_TTL") {
        Some(v) => parse_duration_secs(&v).context("invalid SWING_NIP05_CACHE_TTL")?,
        None => match file.policy.nip05_cache_ttl {
            Some(v) => parse_duration_secs(&v).context("invalid [policy].nip05_cache_ttl")?,
            None => 86_400,
        },
    };

    let state_dir = get_env("SWING_STATE_DIR")
        .or(file.agent.state_dir)
        .unwrap_or_else(|| "./data".to_string());

    let poll_interval = match get_env("SWING_POLL_INTERVAL") {
        Some(v) => parse_duration_secs(&v).context("invalid SWING_POLL_INTERVAL")?,
        None => match file.agent.poll_interval {
            Some(v) => parse_duration_secs(&v).context("invalid [agent].poll_interval")?,
            None => 300,
        },
    };
    if poll_interval == 0 {
        bail!("poll_interval must be greater than 0");
    }

    let pin_timeout = match get_env("SWING_PIN_TIMEOUT") {
        Some(v) => parse_duration_secs(&v).context("invalid SWING_PIN_TIMEOUT")?,
        None => 900,
    };
    if pin_timeout == 0 {
        bail!("SWING_PIN_TIMEOUT must be greater than 0");
    }

    let fetch_idle_timeout = match get_env("SWING_FETCH_IDLE_TIMEOUT") {
        Some(v) => parse_duration_secs(&v).context("invalid SWING_FETCH_IDLE_TIMEOUT")?,
        None => 120,
    };
    if fetch_idle_timeout == 0 {
        bail!("SWING_FETCH_IDLE_TIMEOUT must be greater than 0");
    }

    let concurrency = match get_env("SWING_CONCURRENCY") {
        Some(v) => v
            .parse()
            .context("invalid SWING_CONCURRENCY: expected integer")?,
        None => file.agent.concurrency.unwrap_or(4),
    };
    if concurrency == 0 {
        bail!("concurrency must be greater than 0");
    }

    let publish_nip05 = match get_env("SWING_PUBLISH_NIP05") {
        Some(v) => parse_nip05_mode(&v).context("invalid SWING_PUBLISH_NIP05")?,
        None => match file.publish.nip05 {
            Some(v) => parse_nip05_mode(&v).context("invalid [publish].nip05")?,
            None => Nip05Mode::Warn,
        },
    };

    Ok(Config {
        nostr: NostrConfig {
            secret_key: secret_key.into(),
            relays,
            mirror_set,
            site_event_kind,
        },
        ipfs: IpfsConfig { api: ipfs_api },
        policy: PolicyConfig {
            max_total_storage,
            max_per_site,
            max_per_account,
            max_update_size,
            keep_versions,
            keep_days,
            min_update_interval,
            unpin_on_unfollow,
            nip05,
            nip05_cache_ttl,
        },
        agent: AgentConfig {
            state_dir: PathBuf::from(state_dir),
            poll_interval: Duration::from_secs(poll_interval),
            pin_timeout: Duration::from_secs(pin_timeout),
            fetch_idle_timeout: Duration::from_secs(fetch_idle_timeout),
            concurrency,
        },
        publish: PublishConfig {
            nip05: publish_nip05,
        },
    })
}

impl Config {
    pub fn load(cli_path: Option<&Path>) -> Result<Self> {
        let file = load_file(cli_path)?;
        build_config(file, process_env)
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
    fn zero_pin_timeout_is_rejected() {
        let err = build_config(minimal_file(), |k| match k {
            "SWING_PIN_TIMEOUT" => Some("0".into()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("SWING_PIN_TIMEOUT"));
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
        assert_eq!(cfg.ipfs.api, "http://127.0.0.1:5001");
        assert_eq!(cfg.policy.max_total_storage, 100 * (1u64 << 30));
        assert_eq!(cfg.policy.max_per_site, 10 * (1u64 << 30));
        assert_eq!(cfg.policy.max_per_account, 20 * (1u64 << 30));
        assert_eq!(cfg.policy.max_update_size, 2 * (1u64 << 30));
        assert_eq!(cfg.policy.keep_versions, 5);
        assert_eq!(cfg.policy.keep_days, 365);
        assert_eq!(cfg.policy.min_update_interval, 600);
        assert!(cfg.policy.unpin_on_unfollow);
        assert_eq!(cfg.policy.nip05, Nip05Mode::Warn);
        assert_eq!(cfg.policy.nip05_cache_ttl, 86_400);
        assert_eq!(cfg.agent.poll_interval, Duration::from_secs(300));
        assert_eq!(cfg.agent.pin_timeout, Duration::from_secs(900));
        assert_eq!(cfg.agent.fetch_idle_timeout, Duration::from_secs(120));
        assert_eq!(cfg.agent.concurrency, 4);
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
}
