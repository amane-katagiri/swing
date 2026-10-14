use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use nostr_sdk::prelude::Keys;
use toml_edit::{Array, DocumentMut, Item, Table, Value};
use zeroize::Zeroizing;

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
        Kind::Mode => {
            let s = value.as_str(desc.key)?;
            config::parse_check_mode(s)?;
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
        Ok(text) => Zeroizing::new(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Zeroizing::default(),
        Err(e) => {
            return Err(EditError::Io(
                anyhow::Error::new(e).context(format!("reading config file {}", path.display())),
            ));
        }
    };
    text.parse::<DocumentMut>()
        .map_err(|e| crate::config::toml_error(&text, e.span(), e.message()))
        .context("parsing existing config file")
        .map_err(EditError::Invalid)
}

// Renaming over a symlink would replace the link itself, so write to the file it points at.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let target = match std::fs::canonicalize(path) {
        Ok(real) => real,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("resolving {}", path.display())));
        }
    };
    if let Some(dir) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::auth::create_private_dir_all(dir)?;
    }
    crate::auth::write_private_file(&target, contents)
}

fn validate_and_write(path: &Path, rendered: &str) -> Result<Config, EditError> {
    config::build_config_for_file(rendered, path, config::env_var)
        .context("edited configuration is invalid")
        .map_err(EditError::Invalid)?;
    write_atomic(path, rendered).map_err(EditError::Io)?;
    Config::load(Some(path)).map_err(EditError::Io)
}

pub fn update(current: &Config, items: &BTreeMap<String, InputValue>) -> Result<Config, EditError> {
    check_not_env_sourced(current, items).map_err(EditError::Invalid)?;
    let mut doc = load_document(&current.config_path)?;
    apply_items(&mut doc, items).map_err(EditError::Invalid)?;
    validate_and_write(&current.config_path, &Zeroizing::new(doc.to_string()))
}

fn insert_addrs(doc: &mut DocumentMut, addrs: &[(&str, SocketAddr)]) -> Result<()> {
    for (key, addr) in addrs {
        let desc = find(key)
            .filter(|s| s.kind == Kind::SocketAddr)
            .with_context(|| format!("{key} is not an address setting"))?;
        ensure_table(doc, desc.section)?
            .insert(desc.field, Item::Value(Value::from(addr.to_string())));
    }
    Ok(())
}

pub fn pin_addrs(current: &Config, addrs: &[(&str, SocketAddr)]) -> Result<Config, EditError> {
    let mut doc = load_document(&current.config_path)?;
    insert_addrs(&mut doc, addrs).map_err(EditError::Invalid)?;
    validate_and_write(&current.config_path, &Zeroizing::new(doc.to_string()))
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
        Item::Value(Value::from(
            Zeroizing::new(keys.secret_key().to_secret_hex()).as_str(),
        )),
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
    validate_and_write(&current.config_path, &Zeroizing::new(doc.to_string()))
}

#[cfg(test)]
mod tests;
