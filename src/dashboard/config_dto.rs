use serde::Serialize;

use crate::config;
use crate::format::{format_bytes, format_duration_secs};

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Str(String),
    Bool(bool),
    Num(u64),
    List(Vec<String>),
}

#[derive(Debug, Serialize)]
pub struct DescriptionDto {
    pub en: &'static str,
    pub ja: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ConfigItemDto {
    pub key: String,
    pub env: String,
    pub value: ConfigValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    pub source: &'static str,
    pub editable: bool,
    pub kind: crate::settings::Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<crate::settings::RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<&'static str>>,
    pub description: DescriptionDto,
}

fn source_str(source: config::Source) -> &'static str {
    match source {
        config::Source::Env => "env",
        config::Source::File => "file",
        config::Source::Default => "default",
    }
}

fn config_item_dto(
    config: &config::Config,
    desc: &'static crate::settings::Setting,
) -> ConfigItemDto {
    let (value, display) = config_value(config, desc);
    let source = config
        .source_of(desc.key)
        .unwrap_or(config::Source::Default);
    let options = if desc.kind == crate::settings::Kind::Mode {
        Some(config::CHECK_MODE_NAMES.to_vec())
    } else {
        None
    };
    ConfigItemDto {
        key: desc.field.to_string(),
        env: desc.env.to_string(),
        value,
        display,
        source: source_str(source),
        editable: crate::settings::is_editable(config, desc.key),
        kind: desc.kind,
        raw: crate::settings::raw_value(config, desc.key),
        options,
        description: DescriptionDto {
            en: desc.description.en,
            ja: desc.description.ja,
        },
    }
}

fn opt_path_str(p: &Option<std::path::PathBuf>) -> String {
    p.as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn config_value(
    config: &config::Config,
    desc: &crate::settings::Setting,
) -> (ConfigValue, Option<String>) {
    match desc.key {
        "nostr.secret_key" => (
            ConfigValue::Str(
                if config.nostr.secret_key.is_some() {
                    "(set, hidden)"
                } else {
                    "(not set)"
                }
                .to_string(),
            ),
            None,
        ),
        "nostr.relays" => (ConfigValue::List(config.nostr.relays.clone()), None),
        "nostr.mirror_set" => (ConfigValue::Str(config.nostr.mirror_set.clone()), None),
        "nostr.site_event_kind" => (ConfigValue::Num(config.nostr.site_event_kind as u64), None),
        "nostr.replica_event_kind" => (
            ConfigValue::Num(config.nostr.replica_event_kind as u64),
            None,
        ),
        "ipfs.api" => (ConfigValue::Str(ipfs_api_str(&config.ipfs.api)), None),
        "ipfs.mfs_root" => (ConfigValue::Str(config.ipfs.mfs_root.clone()), None),
        "policy.max_total_storage" => (
            ConfigValue::Num(config.policy.max_total_storage),
            Some(format_bytes(config.policy.max_total_storage)),
        ),
        "policy.max_per_site" => (
            ConfigValue::Num(config.policy.max_per_site),
            Some(format_bytes(config.policy.max_per_site)),
        ),
        "policy.max_per_account" => (
            ConfigValue::Num(config.policy.max_per_account),
            Some(format_bytes(config.policy.max_per_account)),
        ),
        "policy.max_sites_per_account" => (
            ConfigValue::Num(config.policy.max_sites_per_account as u64),
            None,
        ),
        "policy.max_update_size" => (
            ConfigValue::Num(config.policy.max_update_size),
            Some(format_bytes(config.policy.max_update_size)),
        ),
        "policy.keep_versions" => (ConfigValue::Num(config.policy.keep_versions as u64), None),
        "policy.keep_days" => (ConfigValue::Num(config.policy.keep_days), None),
        "policy.min_update_interval" => (
            ConfigValue::Num(config.policy.min_update_interval),
            Some(format_duration_secs(config.policy.min_update_interval)),
        ),
        "policy.remove_on_unfollow" => (ConfigValue::Bool(config.policy.remove_on_unfollow), None),
        "policy.nip05" => (
            ConfigValue::Str(config.policy.nip05.name().to_string()),
            None,
        ),
        "policy.nip05_cache_ttl" => (
            ConfigValue::Num(config.policy.nip05_cache_ttl),
            Some(format_duration_secs(config.policy.nip05_cache_ttl)),
        ),
        "agent.state_dir" => (
            ConfigValue::Str(config.agent.state_dir.display().to_string()),
            None,
        ),
        "agent.poll_interval" => (
            ConfigValue::Num(config.agent.poll_interval.as_secs()),
            Some(format_duration_secs(config.agent.poll_interval.as_secs())),
        ),
        "agent.fetch_timeout" => (
            ConfigValue::Num(config.agent.fetch_timeout.as_secs()),
            Some(format_duration_secs(config.agent.fetch_timeout.as_secs())),
        ),
        "agent.fetch_idle_timeout" => (
            ConfigValue::Num(config.agent.fetch_idle_timeout.as_secs()),
            Some(format_duration_secs(
                config.agent.fetch_idle_timeout.as_secs(),
            )),
        ),
        "agent.concurrency" => (ConfigValue::Num(config.agent.concurrency as u64), None),
        "agent.report_ttl" => (
            ConfigValue::Num(config.agent.report_ttl.as_secs()),
            Some(format_duration_secs(config.agent.report_ttl.as_secs())),
        ),
        "publish.nip05" => (
            ConfigValue::Str(config.publish.nip05.name().to_string()),
            None,
        ),
        "publish.keep_versions" => (ConfigValue::Num(config.publish.keep_versions as u64), None),
        "publish.check_dotfiles" => (
            ConfigValue::Str(config.publish.check_dotfiles.name().to_string()),
            None,
        ),
        "publish.dotfiles_allow" => (
            ConfigValue::List(config.publish.dotfiles_allow.clone()),
            None,
        ),
        "publish.check_size" => (
            ConfigValue::Str(config.publish.check_size.name().to_string()),
            None,
        ),
        "publish.check_unchanged" => (
            ConfigValue::Str(config.publish.check_unchanged.name().to_string()),
            None,
        ),
        "dashboard.listen" => (ConfigValue::Str(config.dashboard.listen.to_string()), None),
        "dashboard.ui" => (ConfigValue::Bool(config.dashboard.ui), None),
        "dashboard.allowed_hosts" => (
            ConfigValue::List(config.dashboard.allowed_hosts.clone()),
            None,
        ),
        "dashboard.public_url" => (
            ConfigValue::Str(config.dashboard.public_url.clone().unwrap_or_default()),
            None,
        ),
        "dashboard.gateway" => (
            ConfigValue::Str(config.dashboard.gateway.clone().unwrap_or_default()),
            None,
        ),
        "dashboard.custom_css" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.custom_css)),
            None,
        ),
        "dashboard.desktop_page" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.desktop_page)),
            None,
        ),
        "dashboard.desktop_page_css" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.desktop_page_css)),
            None,
        ),
        "dashboard.desktop_banner" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.desktop_banner)),
            None,
        ),
        "dashboard.mascots_dir" => (
            ConfigValue::Str(opt_path_str(&config.dashboard.mascots_dir)),
            None,
        ),
        "dashboard.max_upload" => (
            ConfigValue::Num(config.dashboard.max_upload),
            Some(format_bytes(config.dashboard.max_upload)),
        ),
        "kubo.managed" => (ConfigValue::Bool(config.kubo.managed), None),
        "kubo.binary" => (ConfigValue::Str(opt_path_str(&config.kubo.binary)), None),
        "kubo.repo" => (
            ConfigValue::Str(config.kubo.repo.display().to_string()),
            None,
        ),
        "kubo.storage_max" => (
            ConfigValue::Num(config.kubo.storage_max),
            Some(format_bytes(config.kubo.storage_max)),
        ),
        "kubo.provide_strategy" => (ConfigValue::Str(config.kubo.provide_strategy.clone()), None),
        "kubo.gateway_listen" => (
            ConfigValue::Str(config.kubo.gateway_listen.to_string()),
            None,
        ),
        "kubo.swarm_port" => (
            ConfigValue::Str(
                config
                    .kubo
                    .swarm_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            None,
        ),
        "gateway.listen" => (ConfigValue::Str(listen_str(&config.gateway.listen)), None),
        "gateway.hosts" => (ConfigValue::List(config.gateway.hosts.clone()), None),
        "gateway.upstream" => (ConfigValue::Str(config.gateway.upstream.clone()), None),
        other => unreachable!("settings::SETTINGS has no dto mapping for {other}"),
    }
}

#[derive(Debug, Serialize)]
pub struct ConfigSectionDto {
    pub name: String,
    pub items: Vec<ConfigItemDto>,
}

fn listen_str(listen: &config::Listen) -> String {
    match listen {
        config::Listen::Off => "off".to_string(),
        config::Listen::Addr(addr) => addr.to_string(),
    }
}

fn ipfs_api_str(api: &config::IpfsApi) -> String {
    match api {
        config::IpfsApi::Url(url) => url.clone(),
        config::IpfsApi::Managed => "managed".to_string(),
    }
}

fn is_config_writable(config: &config::Config) -> bool {
    if config.config_exists {
        return std::fs::OpenOptions::new()
            .append(true)
            .open(&config.config_path)
            .is_ok();
    }
    let Some(existing) = config
        .config_path
        .ancestors()
        .skip(1)
        .filter(|p| !p.as_os_str().is_empty())
        .find(|p| p.exists())
    else {
        return true;
    };
    match std::fs::metadata(existing) {
        Ok(meta) => meta.is_dir() && !meta.permissions().readonly(),
        Err(_) => false,
    }
}

#[derive(Debug, Serialize)]
pub struct ConfigDto {
    pub config_path: Option<String>,
    pub config_exists: bool,
    pub writable: bool,
    pub restart_required: bool,
    pub sections: Vec<ConfigSectionDto>,
}

pub fn config_dto(config: &config::Config, restart_required: bool) -> ConfigDto {
    let sections = crate::settings::SECTION_ORDER
        .iter()
        .map(|&name| {
            let items = crate::settings::SETTINGS
                .iter()
                .filter(|s| s.section == name)
                .map(|s| config_item_dto(config, s))
                .collect();
            ConfigSectionDto {
                name: name.to_string(),
                items,
            }
        })
        .collect();

    ConfigDto {
        config_path: Some(config.config_path.display().to_string()),
        config_exists: config.config_exists,
        writable: is_config_writable(config),
        restart_required,
        sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_config_at(path: PathBuf, exists: bool) -> config::Config {
        let mut cfg = config::build_config_from_str(
            "[nostr]\nsecret_key = \"k\"\nrelays = [\"wss://r\"]\n",
            |_| None,
        )
        .unwrap();
        cfg.config_path = path;
        cfg.config_exists = exists;
        cfg
    }

    #[test]
    fn missing_config_under_missing_directories_is_writable_when_the_nearest_one_is() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = test_config_at(dir.path().join("a").join("b").join("swing.toml"), false);
        assert!(is_config_writable(&cfg));
    }

    #[test]
    fn missing_config_under_a_file_is_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "").unwrap();
        let cfg = test_config_at(file.join("swing.toml"), false);
        assert!(!is_config_writable(&cfg));
    }

    #[test]
    fn missing_config_in_a_writable_directory_is_writable() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = test_config_at(dir.path().join("swing.toml"), false);
        assert!(is_config_writable(&cfg));
    }

    #[test]
    fn existing_writable_config_is_writable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(&path, "").unwrap();
        let cfg = test_config_at(path, true);
        assert!(is_config_writable(&cfg));
    }

    #[cfg(unix)]
    #[test]
    fn existing_readonly_config_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.toml");
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let cfg = test_config_at(path.clone(), true);
        if std::fs::OpenOptions::new().append(true).open(&path).is_ok() {
            // Root, or a filesystem that ignores 0o400, can still open this.
            return;
        }
        assert!(!is_config_writable(&cfg));
    }
}
