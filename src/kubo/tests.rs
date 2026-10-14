use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

#[cfg(unix)]
use crate::proc::{process_alive, process_start_marker};

use super::access::{api_access_path, read_api_access, read_peer_id};
use super::binary::run;
use super::config::{
    GATEWAY_CONTENT_SECURITY_POLICY, PATH_GATEWAY_BLOCKED_HOSTS, default_swarm_addrs,
    gateway_http_headers_json, gateway_multiaddr, public_gateways_json,
};
use super::daemon::is_repo_lock_error;
use super::orphan::{PidRecord, read_pid_file};
use super::*;
use crate::test_support::{RawServer, http_response, serve_raw};

mod access;
mod binary;
mod config;
mod daemon;
mod orphan;

fn test_kubo_bin() -> PathBuf {
    let bin = std::env::var_os("SWING_TEST_KUBO_BIN")
        .expect("SWING_TEST_KUBO_BIN must point at a Kubo binary to run this ignored test");
    PathBuf::from(bin)
}

#[cfg(unix)]
fn fake_kubo(dir: &Path, script: &str, peer_id: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-ipfs");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join("config"),
        format!(r#"{{"Identity":{{"PeerID":"{peer_id}"}}}}"#),
    )
    .unwrap();
    path
}

#[cfg(unix)]
fn writes_api_file_then_sleeps(port: u16) -> String {
    format!("echo /ip4/127.0.0.1/tcp/{port} > \"$IPFS_PATH/api\"\nexec sleep 30")
}

async fn serve_id(peer_id: &str) -> RawServer {
    serve_raw(http_response("200 OK", &format!(r#"{{"ID":"{peer_id}"}}"#))).await
}

fn access(port: u16) -> ApiAccess {
    ApiAccess {
        port,
        secret: ApiSecret::parse("abcd").unwrap(),
    }
}

#[test]
fn pick_free_port_returns_bindable_port() {
    let port = pick_free_port().unwrap();
    assert!(port > 0);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}

async fn configure_test_repo(
    bin: &Path,
    repo: &Path,
    public_gateway_hosts: Vec<String>,
) -> KuboSettings {
    run(bin, Some(repo), &["config", "profile", "apply", "test"])
        .await
        .unwrap();
    let settings = KuboSettings {
        storage_max: 123_456_789,
        provide_strategy: "pinned+mfs".to_string(),
        api: ApiAccess::generate(pick_free_port().unwrap()),
        gateway: SocketAddr::from(([127, 0, 0, 1], pick_free_port().unwrap())),
        swarm_port: None,
        public_gateway_hosts,
    };
    apply_config(repo, &settings).unwrap();
    settings
}
