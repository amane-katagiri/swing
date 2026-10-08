use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::json;

use super::access::ApiAccess;

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
    for host in PATH_GATEWAY_BLOCKED_HOSTS {
        map.insert(
            host.to_string(),
            json!({
                "Paths": [],
                "UseSubdomains": false,
                "NoDNSLink": true,
            }),
        );
    }
    serde_json::Value::Object(map)
}

// A page served from a loopback hostname reaches other local ports and the LAN without the browser's Local Network Access prompt.
pub(super) const GATEWAY_CONTENT_SECURITY_POLICY: &str =
    "connect-src 'self' https: wss:; form-action 'self' https:";
// Path-style URLs on these hosts would put every site in one origin that shares cookies with the dashboard.
pub(super) const PATH_GATEWAY_BLOCKED_HOSTS: &[&str] = &["127.0.0.1", "::1", "*.localhost"];

pub(super) fn gateway_http_headers_json() -> serde_json::Value {
    json!({ "Content-Security-Policy": [GATEWAY_CONTENT_SECURITY_POLICY] })
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

// Edits the file directly so the API secret never reaches a command line other users can read.
pub fn apply_config(repo: &Path, s: &KuboSettings) -> Result<()> {
    let path = repo.join("config");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut config: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let root = config
        .as_object_mut()
        .with_context(|| format!("{} is not a JSON object", path.display()))?;

    object_entry(root, "Datastore", &path)?
        .insert("StorageMax".to_string(), json!(s.storage_max.to_string()));
    object_entry(root, "Provide", &path)?.insert("Strategy".to_string(), json!(s.provide_strategy));

    let gateway = object_entry(root, "Gateway", &path)?;
    gateway.insert("NoFetch".to_string(), json!(true));
    gateway.insert("NoDNSLink".to_string(), json!(true));
    gateway.insert(
        "PublicGateways".to_string(),
        public_gateways_json(&s.public_gateway_hosts),
    );
    gateway.insert("HTTPHeaders".to_string(), gateway_http_headers_json());

    let addresses = object_entry(root, "Addresses", &path)?;
    addresses.insert("Gateway".to_string(), json!([gateway_multiaddr(s.gateway)]));
    if let Some(port) = s.swarm_port {
        addresses.insert("Swarm".to_string(), json!(default_swarm_addrs(port)));
    }
    addresses.insert("API".to_string(), json!([s.api.multiaddr()]));

    object_entry(root, "API", &path)?.insert(
        "Authorizations".to_string(),
        s.api.secret.kubo_authorizations(),
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
