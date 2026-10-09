use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Source};

mod catalog;
mod edit;
mod example;

pub use catalog::SETTINGS;
pub(crate) use edit::write_atomic;
pub use edit::{EditError, pin_addrs, setup, setup_keys, update};
pub use example::{render_env_example, render_toml_example};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Size,
    Duration,
    Bool,
    Integer,
    String,
    List,
    Mode,
    Path,
    SocketAddr,
    Port,
    Url,
    Secret,
    Listen,
}

#[derive(Debug, Clone, Copy)]
pub struct Text {
    pub en: &'static str,
    pub ja: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum Example {
    Value(&'static str),
    Commented(&'static str),
    Derived(&'static str),
}

impl Example {
    fn literal(self) -> &'static str {
        match self {
            Example::Value(v) | Example::Commented(v) | Example::Derived(v) => v,
        }
    }

    fn is_active(self) -> bool {
        matches!(self, Example::Value(_))
    }
}

pub struct Setting {
    pub key: &'static str,
    pub section: &'static str,
    pub field: &'static str,
    pub env: &'static str,
    pub kind: Kind,
    pub example: Example,
    pub editable: bool,
    pub description: Text,
}

pub const SECTION_ORDER: [&str; 8] = [
    "nostr",
    "ipfs",
    "policy",
    "agent",
    "publish",
    "dashboard",
    "kubo",
    "gateway",
];

pub fn find(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

pub fn env_of(key: &str) -> &'static str {
    find(key)
        .unwrap_or_else(|| panic!("settings::env_of: no such catalog key: {key}"))
        .env
}

pub fn is_editable(config: &Config, key: &str) -> bool {
    find(key).is_some_and(|s| s.editable) && config.source_of(key) != Some(Source::Env)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum InputValue {
    Str(String),
    List(Vec<String>),
}

impl InputValue {
    fn as_str(&self, key: &str) -> Result<&str> {
        match self {
            InputValue::Str(s) => Ok(s.as_str()),
            InputValue::List(_) => bail!("{key} expects a single string value, not a list"),
        }
    }

    fn as_list(&self, key: &str) -> Result<&[String]> {
        match self {
            InputValue::List(v) => Ok(v.as_slice()),
            InputValue::Str(_) => bail!("{key} expects a list of strings"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum RawValue {
    Str(String),
    List(Vec<String>),
}

pub fn raw_value(config: &Config, key: &str) -> Option<RawValue> {
    let value = match key {
        "nostr.relays" => RawValue::List(config.nostr.relays.clone()),
        "nostr.mirror_set" => RawValue::Str(config.nostr.mirror_set.clone()),
        "policy.max_total_storage" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_total_storage))
        }
        "policy.max_per_site" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_per_site))
        }
        "policy.max_per_account" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_per_account))
        }
        "policy.max_update_size" => {
            RawValue::Str(crate::format::format_bytes(config.policy.max_update_size))
        }
        "policy.max_sites_per_account" => {
            RawValue::Str(config.policy.max_sites_per_account.to_string())
        }
        "policy.keep_versions" => RawValue::Str(config.policy.keep_versions.to_string()),
        "policy.keep_days" => RawValue::Str(config.policy.keep_days.to_string()),
        "policy.min_update_interval" => RawValue::Str(crate::format::format_duration_secs(
            config.policy.min_update_interval,
        )),
        "policy.nip05_cache_ttl" => RawValue::Str(crate::format::format_duration_secs(
            config.policy.nip05_cache_ttl,
        )),
        "agent.poll_interval" => RawValue::Str(crate::format::format_duration_secs(
            config.agent.poll_interval.as_secs(),
        )),
        "agent.report_ttl" => RawValue::Str(crate::format::format_duration_secs(
            config.agent.report_ttl.as_secs(),
        )),
        "policy.remove_on_unfollow" => RawValue::Str(config.policy.remove_on_unfollow.to_string()),
        "policy.nip05" => RawValue::Str(config.policy.nip05.name().to_string()),
        "publish.nip05" => RawValue::Str(config.publish.nip05.name().to_string()),
        "agent.concurrency" => RawValue::Str(config.agent.concurrency.to_string()),
        "publish.keep_versions" => RawValue::Str(config.publish.keep_versions.to_string()),
        "publish.check_dotfiles" => RawValue::Str(config.publish.check_dotfiles.name().to_string()),
        "publish.dotfiles_allow" => RawValue::List(config.publish.dotfiles_allow.clone()),
        "publish.check_size" => RawValue::Str(config.publish.check_size.name().to_string()),
        "publish.check_links" => RawValue::Str(config.publish.check_links.name().to_string()),
        "publish.check_unchanged" => {
            RawValue::Str(config.publish.check_unchanged.name().to_string())
        }
        "kubo.storage_max" => RawValue::Str(crate::format::format_bytes(config.kubo.storage_max)),
        "dashboard.gateway" => RawValue::Str(config.dashboard.gateway.clone().unwrap_or_default()),
        _ => return None,
    };
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_exactly_the_keys_build_config_resolves() {
        let cfg = crate::config::build_config_from_str("", |_| None).unwrap();
        let catalog_keys: std::collections::BTreeSet<&str> =
            SETTINGS.iter().map(|s| s.key).collect();
        let source_keys: std::collections::BTreeSet<&str> =
            cfg.sources.keys().map(String::as_str).collect();
        assert_eq!(catalog_keys, source_keys);
    }

    #[test]
    fn catalog_has_exactly_25_editable_keys() {
        assert_eq!(SETTINGS.iter().filter(|s| s.editable).count(), 25);
    }

    // raw_value() lists editable keys by hand, so a key missing there would silently read as None.
    #[test]
    fn raw_value_covers_every_editable_key() {
        let cfg = crate::config::build_config_from_str("", |_| None).unwrap();
        for setting in SETTINGS.iter().filter(|s| s.editable) {
            assert!(
                raw_value(&cfg, setting.key).is_some(),
                "raw_value() has no arm for editable key {}",
                setting.key
            );
        }
    }
}
