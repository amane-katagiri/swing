use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::json;

use super::access::ApiAccess;
use super::binary::run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KuboSettings {
    pub storage_max: u64,
    pub provide_strategy: String,
    pub api: ApiAccess,
    pub gateway: SocketAddr,
    pub swarm_port: Option<u16>,
    pub public_gateway_hosts: Vec<String>,
}

pub fn multiaddr_to_http_url(addr: &str) -> Result<String> {
    let parts: Vec<&str> = addr.trim().trim_start_matches('/').split('/').collect();
    let [proto, host, "tcp", port] = parts.as_slice() else {
        bail!("unsupported multiaddr (expected /ip4|ip6|dns4|dns6|dns/<host>/tcp/<port>): {addr}");
    };
    let port: u16 = port
        .parse()
        .with_context(|| format!("invalid port in multiaddr: {addr}"))?;
    let host = match *proto {
        "ip4" | "dns4" | "dns6" | "dns" => host.to_string(),
        "ip6" => format!("[{host}]"),
        other => bail!("unsupported multiaddr protocol {other}: {addr}"),
    };
    if host.is_empty() || host == "[]" {
        bail!("empty host in multiaddr: {addr}");
    }
    Ok(format!("http://{host}:{port}"))
}

pub(super) fn public_gateways_json(hosts: &[String]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for host in hosts {
        map.insert(
            host.clone(),
            json!({
                "Paths": [],
                "UseSubdomains": false,
                "NoDNSLink": false,
            }),
        );
    }
    serde_json::Value::Object(map)
}

pub(super) fn default_swarm_addrs(port: u16) -> Vec<String> {
    vec![
        format!("/ip4/0.0.0.0/tcp/{port}"),
        format!("/ip6/::/tcp/{port}"),
        format!("/ip4/0.0.0.0/udp/{port}/webrtc-direct"),
        format!("/ip4/0.0.0.0/udp/{port}/quic-v1"),
        format!("/ip4/0.0.0.0/udp/{port}/quic-v1/webtransport"),
        format!("/ip6/::/udp/{port}/webrtc-direct"),
        format!("/ip6/::/udp/{port}/quic-v1"),
        format!("/ip6/::/udp/{port}/quic-v1/webtransport"),
    ]
}

pub(super) fn gateway_multiaddr(addr: SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(ip) => format!("/ip4/{ip}/tcp/{}", addr.port()),
        IpAddr::V6(ip) => format!("/ip6/{ip}/tcp/{}", addr.port()),
    }
}

async fn set_config(bin: &Path, repo: &Path, key: &str, value: &str) -> Result<()> {
    run(bin, Some(repo), &["config", key, value]).await?;
    Ok(())
}

async fn set_config_json(
    bin: &Path,
    repo: &Path,
    key: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let value = serde_json::to_string(value).with_context(|| format!("serializing {key}"))?;
    run(bin, Some(repo), &["config", "--json", key, &value]).await?;
    Ok(())
}

pub async fn apply_config(bin: &Path, repo: &Path, s: &KuboSettings) -> Result<()> {
    set_config(
        bin,
        repo,
        "Datastore.StorageMax",
        &s.storage_max.to_string(),
    )
    .await?;
    set_config(bin, repo, "Provide.Strategy", &s.provide_strategy).await?;

    set_config_json(bin, repo, "Gateway.NoFetch", &json!(true)).await?;
    set_config_json(bin, repo, "Gateway.NoDNSLink", &json!(true)).await?;
    set_config_json(
        bin,
        repo,
        "Gateway.PublicGateways",
        &public_gateways_json(&s.public_gateway_hosts),
    )
    .await?;

    set_config_json(
        bin,
        repo,
        "Addresses.Gateway",
        &json!([gateway_multiaddr(s.gateway)]),
    )
    .await?;

    if let Some(port) = s.swarm_port {
        set_config_json(
            bin,
            repo,
            "Addresses.Swarm",
            &json!(default_swarm_addrs(port)),
        )
        .await?;
    }

    set_api_access(repo, &s.api)
}

// Edits the file directly because `ipfs config` would put the port and secret on a command line other users can read.
pub(super) fn set_api_access(repo: &Path, access: &ApiAccess) -> Result<()> {
    let path = repo.join("config");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut config: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let root = config
        .as_object_mut()
        .with_context(|| format!("{} is not a JSON object", path.display()))?;
    object_entry(root, "Addresses", &path)?.insert("API".to_string(), json!([access.multiaddr()]));
    object_entry(root, "API", &path)?.insert(
        "Authorizations".to_string(),
        access.secret.kubo_authorizations(),
    );
    let text = serde_json::to_string_pretty(&config).context("serializing the Kubo config")?;
    crate::settings::write_atomic(&path, &text)
}

fn object_entry<'a>(
    root: &'a mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    path: &Path,
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>> {
    let value = root.entry(key).or_insert_with(|| json!({}));
    if value.is_null() {
        *value = json!({});
    }
    value
        .as_object_mut()
        .with_context(|| format!("{key} in {} is not a JSON object", path.display()))
}
