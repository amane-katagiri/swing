use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use nostr_sdk::prelude::Keys;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use super::*;
use crate::config::{self, Config};

fn set_item(table: &mut Table, desc: &Setting, value: &InputValue) -> Result<()> {
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
        other => bail!("{}: kind {other:?} is not editable", desc.key),
    }
    Ok(())
}

fn find_editable(key: &str) -> Option<&'static Setting> {
    find(key).filter(|s| s.editable)
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, section: &str) -> Result<&'a mut Table> {
    if doc.get(section).and_then(Item::as_table).is_none() {
        doc[section] = Item::Table(Table::new());
    }
    doc[section]
        .as_table_mut()
        .with_context(|| format!("[{section}] is not a table in the config file"))
}

fn apply_items(doc: &mut DocumentMut, items: &BTreeMap<String, InputValue>) -> Result<()> {
    for (key, value) in items {
        let Some(desc) = find_editable(key) else {
            bail!("unknown or non-editable key: {key}");
        };
        let table = ensure_table(doc, desc.section)?;
        set_item(table, desc, value)?;
    }
    Ok(())
}

fn check_not_env_sourced(current: &Config, items: &BTreeMap<String, InputValue>) -> Result<()> {
    for key in items.keys() {
        if find_editable(key).is_none() {
            bail!("unknown or non-editable key: {key}");
        }
        if current.source_of(key) == Some(config::Source::Env) {
            bail!("{key} is set via an environment variable and cannot be edited here");
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum EditError {
    Invalid(anyhow::Error),
    Io(anyhow::Error),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) | Self::Io(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for EditError {}

fn load_document(path: &Path) -> Result<DocumentMut, EditError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(EditError::Io(
                anyhow::Error::new(e).context(format!("reading config file {}", path.display())),
            ));
        }
    };
    text.parse::<DocumentMut>()
        .context("parsing existing config file")
        .map_err(EditError::Invalid)
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::auth::create_private_dir_all(dir)?;
    }
    crate::auth::write_private_file(path, contents)
}

fn validate_and_write(path: &Path, rendered: &str) -> Result<Config, EditError> {
    config::build_config_from_str(rendered, config::env_var)
        .context("edited configuration is invalid")
        .map_err(EditError::Invalid)?;
    write_atomic(path, rendered).map_err(EditError::Io)?;
    Config::load(Some(path)).map_err(EditError::Io)
}

pub fn update(current: &Config, items: &BTreeMap<String, InputValue>) -> Result<Config, EditError> {
    check_not_env_sourced(current, items).map_err(EditError::Invalid)?;
    let mut doc = load_document(&current.config_path)?;
    apply_items(&mut doc, items).map_err(EditError::Invalid)?;
    validate_and_write(&current.config_path, &doc.to_string())
}

pub fn setup_keys(secret_key_input: Option<&str>) -> Result<Keys> {
    match secret_key_input {
        Some(raw) if !raw.trim().is_empty() => {
            Keys::parse(raw.trim()).context("invalid secret key")
        }
        _ => Ok(Keys::generate()),
    }
}

fn insert_secret_key(doc: &mut DocumentMut, keys: &Keys) -> Result<()> {
    ensure_table(doc, "nostr")?.insert(
        "secret_key",
        Item::Value(Value::from(keys.secret_key().to_secret_hex())),
    );
    Ok(())
}

pub fn setup(
    current: &Config,
    keys: Option<&Keys>,
    items: &BTreeMap<String, InputValue>,
) -> Result<Config, EditError> {
    if current.nostr.secret_key.is_some() {
        return Err(EditError::Invalid(anyhow!(
            "setup is only available before a Nostr key is configured"
        )));
    }
    check_not_env_sourced(current, items).map_err(EditError::Invalid)?;
    let mut doc = load_document(&current.config_path)?;
    apply_items(&mut doc, items).map_err(EditError::Invalid)?;
    if let Some(keys) = keys {
        insert_secret_key(&mut doc, keys).map_err(EditError::Invalid)?;
    }
    validate_and_write(&current.config_path, &doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config(dir: &Path) -> Config {
        let path = dir.join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        Config::load(Some(&path)).unwrap()
    }

    fn config_at(
        text: &str,
        get_env: impl Fn(&str) -> Option<String>,
        path: std::path::PathBuf,
        exists: bool,
    ) -> Config {
        let mut cfg = config::build_config_from_str(text, get_env).unwrap();
        cfg.config_path = path;
        cfg.config_exists = exists;
        cfg
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
        let cfg = config_at(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |k| match k {
                "SWING_MAX_TOTAL_STORAGE" => Some("10GB".to_string()),
                _ => None,
            },
            path,
            true,
        );
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
        let cfg = config_at(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |_| None,
            path.clone(),
            false,
        );
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

    #[cfg(unix)]
    #[test]
    fn missing_parent_directory_is_created_as_0700() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("nested");
        let path = parent.join("swing.toml");
        let cfg = config_at(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |_| None,
            path.clone(),
            false,
        );
        let mut items = BTreeMap::new();
        items.insert(
            "nostr.mirror_set".to_string(),
            InputValue::Str("newset".to_string()),
        );
        update(&cfg, &items).unwrap();
        let mode = std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn existing_file_permissions_are_tightened_to_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(
            &path,
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        let mut items = BTreeMap::new();
        items.insert(
            "nostr.mirror_set".to_string(),
            InputValue::Str("newset".to_string()),
        );
        update(&cfg, &items).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
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
        let cfg = config_at("", |_| None, path.clone(), false);
        assert!(cfg.nostr.secret_key.is_none());
        let items = BTreeMap::new();
        let keys = setup_keys(None).unwrap();
        let reloaded = setup(&cfg, Some(&keys), &items).unwrap();
        assert_eq!(
            reloaded
                .nostr
                .secret_key
                .as_ref()
                .map(|k| k.expose_secret()),
            Some(keys.secret_key().to_secret_hex().as_str())
        );
        assert!(path.exists());
    }

    #[test]
    fn setup_for_a_signer_app_writes_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        let cfg = config_at("", |_| None, path.clone(), false);
        let mut items = BTreeMap::new();
        items.insert(
            "nostr.relays".to_string(),
            InputValue::List(vec!["wss://relay.example".to_string()]),
        );
        let reloaded = setup(&cfg, None, &items).unwrap();
        assert!(reloaded.nostr.secret_key.is_none());
        assert_eq!(reloaded.nostr.relays, vec!["wss://relay.example"]);
    }
}
