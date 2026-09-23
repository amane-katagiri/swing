use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use nostr_sdk::prelude::Keys;
use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::config::{self, Config, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Size,
    Duration,
    Bool,
    Integer,
    String,
    List,
    Nip05,
}

pub struct EditableKey {
    pub key: &'static str,
    pub section: &'static str,
    pub field: &'static str,
    pub kind: Kind,
}

pub const EDITABLE_KEYS: &[EditableKey] = &[
    EditableKey {
        key: "nostr.relays",
        section: "nostr",
        field: "relays",
        kind: Kind::List,
    },
    EditableKey {
        key: "nostr.mirror_set",
        section: "nostr",
        field: "mirror_set",
        kind: Kind::String,
    },
    EditableKey {
        key: "policy.max_total_storage",
        section: "policy",
        field: "max_total_storage",
        kind: Kind::Size,
    },
    EditableKey {
        key: "policy.max_per_site",
        section: "policy",
        field: "max_per_site",
        kind: Kind::Size,
    },
    EditableKey {
        key: "policy.max_per_account",
        section: "policy",
        field: "max_per_account",
        kind: Kind::Size,
    },
    EditableKey {
        key: "policy.max_update_size",
        section: "policy",
        field: "max_update_size",
        kind: Kind::Size,
    },
    EditableKey {
        key: "policy.max_sites_per_account",
        section: "policy",
        field: "max_sites_per_account",
        kind: Kind::Integer,
    },
    EditableKey {
        key: "policy.keep_versions",
        section: "policy",
        field: "keep_versions",
        kind: Kind::Integer,
    },
    EditableKey {
        key: "policy.keep_days",
        section: "policy",
        field: "keep_days",
        kind: Kind::Integer,
    },
    EditableKey {
        key: "policy.min_update_interval",
        section: "policy",
        field: "min_update_interval",
        kind: Kind::Duration,
    },
    EditableKey {
        key: "policy.nip05_cache_ttl",
        section: "policy",
        field: "nip05_cache_ttl",
        kind: Kind::Duration,
    },
    EditableKey {
        key: "agent.poll_interval",
        section: "agent",
        field: "poll_interval",
        kind: Kind::Duration,
    },
    EditableKey {
        key: "agent.report_ttl",
        section: "agent",
        field: "report_ttl",
        kind: Kind::Duration,
    },
    EditableKey {
        key: "policy.remove_on_unfollow",
        section: "policy",
        field: "remove_on_unfollow",
        kind: Kind::Bool,
    },
    EditableKey {
        key: "policy.nip05",
        section: "policy",
        field: "nip05",
        kind: Kind::Nip05,
    },
    EditableKey {
        key: "publish.nip05",
        section: "publish",
        field: "nip05",
        kind: Kind::Nip05,
    },
    EditableKey {
        key: "agent.concurrency",
        section: "agent",
        field: "concurrency",
        kind: Kind::Integer,
    },
    EditableKey {
        key: "publish.keep_versions",
        section: "publish",
        field: "keep_versions",
        kind: Kind::Integer,
    },
    EditableKey {
        key: "kubo.storage_max",
        section: "kubo",
        field: "storage_max",
        kind: Kind::Size,
    },
    EditableKey {
        key: "dashboard.gateway",
        section: "dashboard",
        field: "gateway",
        kind: Kind::String,
    },
];

pub fn find(key: &str) -> Option<&'static EditableKey> {
    EDITABLE_KEYS.iter().find(|k| k.key == key)
}

pub fn is_editable(config: &Config, key: &str) -> bool {
    find(key).is_some() && config.source_of(key) != Some(Source::Env)
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
        "policy.max_total_storage" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_total_storage,
        )),
        "policy.max_per_site" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_per_site,
        )),
        "policy.max_per_account" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_per_account,
        )),
        "policy.max_update_size" => RawValue::Str(crate::dashboard::dto::format_bytes(
            config.policy.max_update_size,
        )),
        "policy.max_sites_per_account" => {
            RawValue::Str(config.policy.max_sites_per_account.to_string())
        }
        "policy.keep_versions" => RawValue::Str(config.policy.keep_versions.to_string()),
        "policy.keep_days" => RawValue::Str(config.policy.keep_days.to_string()),
        "policy.min_update_interval" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.policy.min_update_interval,
        )),
        "policy.nip05_cache_ttl" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.policy.nip05_cache_ttl,
        )),
        "agent.poll_interval" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.agent.poll_interval.as_secs(),
        )),
        "agent.report_ttl" => RawValue::Str(crate::dashboard::dto::format_duration_secs(
            config.agent.report_ttl.as_secs(),
        )),
        "policy.remove_on_unfollow" => RawValue::Str(config.policy.remove_on_unfollow.to_string()),
        "policy.nip05" => RawValue::Str(nip05_mode_name(config.policy.nip05).to_string()),
        "publish.nip05" => RawValue::Str(nip05_mode_name(config.publish.nip05).to_string()),
        "agent.concurrency" => RawValue::Str(config.agent.concurrency.to_string()),
        "publish.keep_versions" => RawValue::Str(config.publish.keep_versions.to_string()),
        "kubo.storage_max" => {
            RawValue::Str(crate::dashboard::dto::format_bytes(config.kubo.storage_max))
        }
        "dashboard.gateway" => RawValue::Str(config.dashboard.gateway.clone().unwrap_or_default()),
        _ => return None,
    };
    Some(value)
}

fn nip05_mode_name(mode: config::Nip05Mode) -> &'static str {
    match mode {
        config::Nip05Mode::Off => "off",
        config::Nip05Mode::Warn => "warn",
        config::Nip05Mode::Require => "require",
    }
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, section: &str) -> Result<&'a mut Table> {
    if doc.get(section).and_then(Item::as_table).is_none() {
        doc[section] = Item::Table(Table::new());
    }
    doc[section]
        .as_table_mut()
        .with_context(|| format!("[{section}] is not a table in the config file"))
}

fn set_item(table: &mut Table, desc: &EditableKey, value: &InputValue) -> Result<()> {
    match desc.kind {
        Kind::List => {
            let list = value.as_list(desc.key)?;
            if desc.key == "nostr.relays" && list.is_empty() {
                bail!("nostr.relays: at least one relay is required");
            }
            let mut arr = Array::new();
            for item in list {
                arr.push(item.as_str());
            }
            table.insert(desc.field, Item::Value(Value::Array(arr)));
        }
        Kind::Bool => {
            let s = value.as_str(desc.key)?;
            let parsed =
                config::parse_bool(s).with_context(|| format!("{}: invalid boolean", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(parsed)));
        }
        Kind::Integer => {
            let s = value.as_str(desc.key)?;
            let parsed: i64 = s
                .trim()
                .parse()
                .with_context(|| format!("{}: expected an integer", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(parsed)));
        }
        Kind::Size => {
            let s = value.as_str(desc.key)?;
            config::parse_size(s).with_context(|| format!("{}: invalid size", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(s.trim().to_string())));
        }
        Kind::Duration => {
            let s = value.as_str(desc.key)?;
            config::parse_duration_secs(s)
                .with_context(|| format!("{}: invalid duration", desc.key))?;
            table.insert(desc.field, Item::Value(Value::from(s.trim().to_string())));
        }
        Kind::Nip05 => {
            let s = value.as_str(desc.key)?;
            config::parse_nip05_mode(s)?;
            table.insert(desc.field, Item::Value(Value::from(s.trim().to_string())));
        }
        Kind::String => {
            let s = value.as_str(desc.key)?;
            table.insert(desc.field, Item::Value(Value::from(s.to_string())));
        }
    }
    Ok(())
}

fn apply_items(doc: &mut DocumentMut, items: &BTreeMap<String, InputValue>) -> Result<()> {
    for (key, value) in items {
        let Some(desc) = find(key) else {
            bail!("unknown or non-editable key: {key}");
        };
        let table = ensure_table(doc, desc.section)?;
        set_item(table, desc, value)?;
    }
    Ok(())
}

fn check_not_env_sourced(current: &Config, items: &BTreeMap<String, InputValue>) -> Result<()> {
    for key in items.keys() {
        if find(key).is_none() {
            bail!("unknown or non-editable key: {key}");
        }
        if current.source_of(key) == Some(Source::Env) {
            bail!("{key} is set via an environment variable and cannot be edited here");
        }
    }
    Ok(())
}

fn load_document(current: &Config) -> Result<DocumentMut> {
    let text = if current.config_exists {
        std::fs::read_to_string(&current.config_path)
            .with_context(|| format!("reading config file {}", current.config_path.display()))?
    } else {
        String::new()
    };
    text.parse::<DocumentMut>()
        .context("parsing existing config file")
}

#[cfg(unix)]
fn write_atomic(path: &Path, contents: &str, existed_before: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating config directory {}", dir.display()))?;
    }
    let mode = if existed_before {
        std::fs::metadata(path)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o600)
    } else {
        0o600
    };
    let tmp_path = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp_path, contents)
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting permissions on {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_atomic(path: &Path, contents: &str, _existed_before: bool) -> Result<()> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating config directory {}", dir.display()))?;
    }
    let tmp_path = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp_path, contents)
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
    Ok(())
}

pub fn update(current: &Config, items: &BTreeMap<String, InputValue>) -> Result<Config> {
    check_not_env_sourced(current, items)?;
    let mut doc = load_document(current)?;
    apply_items(&mut doc, items)?;
    let rendered = doc.to_string();
    config::build_config_from_str(&rendered, config::env_var)
        .context("edited configuration is invalid")?;
    write_atomic(&current.config_path, &rendered, current.config_exists)?;
    Config::load(Some(&current.config_path))
}

pub fn setup(
    current: &Config,
    secret_key_input: Option<&str>,
    items: &BTreeMap<String, InputValue>,
) -> Result<(Config, Keys)> {
    if current.nostr.secret_key.is_some() {
        bail!("setup is only available before a Nostr key is configured");
    }
    check_not_env_sourced(current, items)?;
    let keys = match secret_key_input {
        Some(raw) if !raw.trim().is_empty() => {
            Keys::parse(raw.trim()).context("invalid secret key")?
        }
        _ => Keys::generate(),
    };
    let secret_hex = keys.secret_key().to_secret_hex();

    let mut doc = load_document(current)?;
    apply_items(&mut doc, items)?;
    let nostr_table = ensure_table(&mut doc, "nostr")?;
    nostr_table.insert("secret_key", Item::Value(Value::from(secret_hex)));
    let rendered = doc.to_string();
    config::build_config_from_str(&rendered, config::env_var)
        .context("edited configuration is invalid")?;
    write_atomic(&current.config_path, &rendered, current.config_exists)?;
    let reloaded = Config::load(Some(&current.config_path))?;
    Ok((reloaded, keys))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn base_config(dir: &Path) -> Config {
        let path = dir.join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        Config::load(Some(&path)).unwrap()
    }

    #[test]
    fn rejects_unknown_key() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = base_config(dir.path());
        let mut items = BTreeMap::new();
        items.insert(
            "kubo.binary".to_string(),
            InputValue::Str("/bin/ipfs".to_string()),
        );
        let err = update(&cfg, &items).unwrap_err();
        assert!(err.to_string().contains("kubo.binary"));
    }

    #[test]
    fn rejects_env_sourced_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        let mut cfg = config::build_config_from_str(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |k| match k {
                "SWING_MAX_TOTAL_STORAGE" => Some("10GB".to_string()),
                _ => None,
            },
        )
        .unwrap();
        cfg.config_path = path;
        cfg.config_exists = true;
        let mut items = BTreeMap::new();
        items.insert(
            "policy.max_total_storage".to_string(),
            InputValue::Str("20GB".to_string()),
        );
        let err = update(&cfg, &items).unwrap_err();
        assert!(err.to_string().contains("environment variable"));
    }

    #[test]
    fn writer_preserves_comments_and_unrelated_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "# a comment\n[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\nmirror_set = \"keep-me\"\n",
        )
        .unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        let mut items = BTreeMap::new();
        items.insert(
            "policy.max_total_storage".to_string(),
            InputValue::Str("20GB".to_string()),
        );
        let updated = update(&cfg, &items).unwrap();
        assert_eq!(updated.policy.max_total_storage, 20 * (1u64 << 30));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# a comment"));
        assert!(text.contains("mirror_set = \"keep-me\""));
    }

    #[test]
    fn new_file_gets_owner_only_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("swing.toml");
        let mut cfg = config::build_config_from_str(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |_| None,
        )
        .unwrap();
        cfg.config_path = path.clone();
        cfg.config_exists = false;
        let mut items = BTreeMap::new();
        items.insert(
            "nostr.mirror_set".to_string(),
            InputValue::Str("newset".to_string()),
        );
        update(&cfg, &items).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn validation_failure_leaves_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        let mut items = BTreeMap::new();
        items.insert(
            "policy.max_total_storage".to_string(),
            InputValue::Str("not-a-size".to_string()),
        );
        assert!(update(&cfg, &items).is_err());
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn empty_relay_list_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = base_config(dir.path());
        let mut items = BTreeMap::new();
        items.insert("nostr.relays".to_string(), InputValue::List(vec![]));
        let err = update(&cfg, &items).unwrap_err();
        assert!(err.to_string().contains("at least one relay"));
    }

    #[test]
    fn setup_writes_generated_key_and_leaves_setup_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        let mut cfg = config::build_config_from_str("", |_| None).unwrap();
        cfg.config_path = path.clone();
        cfg.config_exists = false;
        assert!(cfg.nostr.secret_key.is_none());
        let items = BTreeMap::new();
        let (reloaded, _keys) = setup(&cfg, None, &items).unwrap();
        assert!(reloaded.nostr.secret_key.is_some());
        assert!(path.exists());
    }
}
