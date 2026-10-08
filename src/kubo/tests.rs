use std::net::SocketAddr;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
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

fn test_kubo_bin() -> Option<PathBuf> {
    match std::env::var("SWING_TEST_KUBO_BIN") {
        Ok(bin) => Some(PathBuf::from(bin)),
        Err(_) => {
            eprintln!("skipping: SWING_TEST_KUBO_BIN not set");
            None
        }
    }
}

#[test]
fn converts_ip4_and_ip6_multiaddrs() {
    assert_eq!(
        multiaddr_to_http_url("/ip4/127.0.0.1/tcp/5001").unwrap(),
        "http://127.0.0.1:5001"
    );
    assert_eq!(
        multiaddr_to_http_url("/ip6/::1/tcp/5001\n").unwrap(),
        "http://[::1]:5001"
    );
    assert!(multiaddr_to_http_url("/unix/tmp/api.sock").is_err());
    assert!(multiaddr_to_http_url("/ip4/127.0.0.1/udp/5001").is_err());
}

#[tokio::test]
async fn missing_api_access_says_kubo_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("api"), "/ip4/127.0.0.1/tcp/41234\n").unwrap();
    let Err(err) = managed_client(dir.path(), dir.path()).await else {
        panic!("a missing API access file must not yield a client");
    };
    assert!(err.to_string().contains("Kubo is not running"), "{err}");
}

#[test]
fn public_gateways_json_matches_shell_script_shape() {
    let hosts = vec!["a.example.com".to_string(), "b.example.com".to_string()];
    let value = public_gateways_json(&hosts);
    assert_eq!(
        value,
        json!({
            "a.example.com": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false},
            "b.example.com": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false},
            "127.0.0.1": {"Paths": [], "UseSubdomains": false, "NoDNSLink": true},
            "::1": {"Paths": [], "UseSubdomains": false, "NoDNSLink": true},
            "*.localhost": {"Paths": [], "UseSubdomains": false, "NoDNSLink": true},
        })
    );
}

#[test]
fn gateway_config_matches_shell_script() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docker/kubo-init.d/001-swing-config.sh"),
    )
    .unwrap();
    assert!(script.contains(GATEWAY_CONTENT_SECURITY_POLICY));
    let blocked: Vec<String> = PATH_GATEWAY_BLOCKED_HOSTS
        .iter()
        .map(|h| {
            if h.contains('*') {
                format!("\"{h}\"")
            } else {
                h.to_string()
            }
        })
        .collect();
    let line = format!("for host in {}; do", blocked.join(" "));
    assert!(script.contains(&line), "{line}");
}

#[test]
fn public_gateways_json_only_blocks_path_hosts_when_no_hosts() {
    let value = public_gateways_json(&[]);
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys.len(), PATH_GATEWAY_BLOCKED_HOSTS.len());
    for host in PATH_GATEWAY_BLOCKED_HOSTS {
        assert_eq!(value[host]["Paths"], json!([]));
    }
}

#[test]
fn default_swarm_addrs_substitutes_port() {
    let addrs = default_swarm_addrs(4321);
    assert_eq!(
        addrs,
        vec![
            "/ip4/0.0.0.0/tcp/4321",
            "/ip6/::/tcp/4321",
            "/ip4/0.0.0.0/udp/4321/webrtc-direct",
            "/ip4/0.0.0.0/udp/4321/quic-v1",
            "/ip4/0.0.0.0/udp/4321/quic-v1/webtransport",
            "/ip6/::/udp/4321/webrtc-direct",
            "/ip6/::/udp/4321/quic-v1",
            "/ip6/::/udp/4321/quic-v1/webtransport",
        ]
    );
}

#[test]
fn gateway_multiaddr_formats_v4_and_v6() {
    assert_eq!(
        gateway_multiaddr("127.0.0.1:8080".parse().unwrap()),
        "/ip4/127.0.0.1/tcp/8080"
    );
    assert_eq!(
        gateway_multiaddr("[::1]:8080".parse().unwrap()),
        "/ip6/::1/tcp/8080"
    );
}

#[test]
fn storage_max_is_decimal_bytes() {
    assert_eq!((100u64 * (1u64 << 30)).to_string(), "107374182400");
}

#[test]
fn repo_lock_detection_ignores_unrelated_lock_words() {
    assert!(is_repo_lock_error("Error: someone else has the lock"));
    assert!(is_repo_lock_error(
        "cannot acquire lock: Lock FcntlFlock of /x/repo.lock failed"
    ));
    assert!(!is_repo_lock_error("fetched block bafy from peer"));
    assert!(!is_repo_lock_error("blockstore: 12 blocks"));
}

#[test]
fn read_peer_id_reads_identity_from_repo_config() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_peer_id(dir.path()).is_err());
    std::fs::write(dir.path().join("config"), r#"{"Identity":{}}"#).unwrap();
    assert!(read_peer_id(dir.path()).is_err());
    std::fs::write(
        dir.path().join("config"),
        r#"{"Identity":{"PeerID":"12D3KooWmine"}}"#,
    )
    .unwrap();
    assert_eq!(read_peer_id(dir.path()).unwrap(), "12D3KooWmine");
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

struct IdServer {
    port: u16,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
}

impl IdServer {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

async fn serve(status: &'static str, body: String) -> IdServer {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let seen = Arc::clone(&seen);
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase());
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    IdServer { port, requests }
}

async fn serve_id(peer_id: &str) -> IdServer {
    serve("200 OK", format!(r#"{{"ID":"{peer_id}"}}"#)).await
}

fn access(port: u16) -> ApiAccess {
    ApiAccess {
        port,
        secret: ApiSecret::parse("abcd").unwrap(),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn daemon_wait_healthy_fails_fast_when_the_child_exits() {
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_kubo(dir.path(), "exit 3", "mine");
    let server = serve_id("mine").await;
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(server.port))
        .await
        .unwrap();
    let _ = daemon.wait().await;
    let err = daemon
        .wait_healthy(dir.path(), Duration::from_secs(30))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("exited"), "{err}");
    assert!(server.requests().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn daemon_wait_healthy_requires_the_repo_peer_id() {
    let dir = tempfile::tempdir().unwrap();

    let foreign = serve_id("someone-else").await;
    let bin = fake_kubo(
        dir.path(),
        &writes_api_file_then_sleeps(foreign.port),
        "mine",
    );
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(foreign.port))
        .await
        .unwrap();
    let err = daemon
        .wait_healthy(dir.path(), Duration::from_secs(1))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("someone-else") && err.contains("mine"),
        "{err}"
    );
    daemon.stop(Duration::ZERO).await.unwrap();

    let own = serve_id("mine").await;
    let bin = fake_kubo(dir.path(), &writes_api_file_then_sleeps(own.port), "mine");
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(own.port))
        .await
        .unwrap();
    daemon
        .wait_healthy(dir.path(), Duration::from_secs(5))
        .await
        .unwrap();
    daemon.stop(Duration::ZERO).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn daemon_wait_healthy_sends_nothing_until_kubo_writes_the_api_file() {
    let dir = tempfile::tempdir().unwrap();
    let squatter = serve_id("mine").await;
    std::fs::write(
        dir.path().join("api"),
        format!("/ip4/127.0.0.1/tcp/{}", squatter.port),
    )
    .unwrap();
    let bin = fake_kubo(dir.path(), "exec sleep 30", "mine");
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(squatter.port))
        .await
        .unwrap();
    assert!(!dir.path().join("api").exists());
    assert!(
        daemon
            .wait_healthy(dir.path(), Duration::from_secs(2))
            .await
            .is_err()
    );
    daemon.stop(Duration::ZERO).await.unwrap();
    let requests = squatter.requests();
    assert!(
        requests.iter().all(|r| !r.contains("/api/v0/id")),
        "{requests:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn daemon_wait_healthy_refuses_without_a_readable_peer_id() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve_id("mine").await;
    let bin = fake_kubo(
        dir.path(),
        &writes_api_file_then_sleeps(server.port),
        "mine",
    );
    std::fs::write(dir.path().join("config"), r#"{"Identity":{}}"#).unwrap();
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(server.port))
        .await
        .unwrap();
    let err = daemon
        .wait_healthy(dir.path(), Duration::from_secs(5))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("PeerID"), "{err}");
    daemon.stop(Duration::ZERO).await.unwrap();
    assert!(
        server.requests().iter().all(|r| !r.contains("/api/v0/id")),
        "no probe may carry the secret when the peer ID cannot be checked"
    );
}

#[tokio::test]
async fn ensure_own_daemon_rejects_an_api_with_another_peer_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config"),
        r#"{"Identity":{"PeerID":"mine"}}"#,
    )
    .unwrap();
    let foreign = crate::ipfs::IpfsClient::new(serve_id("someone-else").await.url());
    let err = ensure_own_daemon(&foreign, dir.path())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("someone-else") && err.contains("not the Kubo"),
        "{err}"
    );
    let own = crate::ipfs::IpfsClient::new(serve_id("mine").await.url());
    ensure_own_daemon(&own, dir.path()).await.unwrap();
}

#[tokio::test]
async fn managed_client_uses_the_port_stored_with_the_secret() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config"),
        r#"{"Identity":{"PeerID":"mine"}}"#,
    )
    .unwrap();
    let stale = serve_id("someone-else").await;
    let own = serve_id("mine").await;
    std::fs::write(
        dir.path().join("api"),
        format!("/ip4/127.0.0.1/tcp/{}", stale.port),
    )
    .unwrap();
    write_api_access(dir.path(), &access(own.port)).unwrap();
    let client = managed_client(dir.path(), dir.path()).await.unwrap();
    assert_eq!(client.api_url(), own.url());
    assert!(stale.requests().is_empty());
    assert!(
        own.requests()[0].contains("authorization: bearer abcd"),
        "{:?}",
        own.requests()
    );
}

#[test]
fn api_access_round_trips_through_a_private_file_and_stays_out_of_debug() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    assert!(read_api_access(&state_dir).unwrap().is_none());
    let first = ApiAccess::generate(4001);
    write_api_access(&state_dir, &first).unwrap();
    assert_eq!(read_api_access(&state_dir).unwrap(), Some(first.clone()));
    let second = ApiAccess::generate(4002);
    assert_ne!(first.secret, second.secret);
    write_api_access(&state_dir, &second).unwrap();
    assert_eq!(read_api_access(&state_dir).unwrap(), Some(second.clone()));
    assert!(!format!("{second:?}").contains(&second.secret.0));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(api_access_path(&state_dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::write(
        api_access_path(&state_dir),
        r#"{"port":4001,"secret":"not hex"}"#,
    )
    .unwrap();
    assert!(read_api_access(&state_dir).is_err());
    remove_api_access(&state_dir).unwrap();
    assert!(read_api_access(&state_dir).unwrap().is_none());
    remove_api_access(&state_dir).unwrap();
}

fn settings(api: ApiAccess, swarm_port: Option<u16>) -> KuboSettings {
    KuboSettings {
        storage_max: 123_456_789,
        provide_strategy: "pinned+mfs".to_string(),
        api,
        gateway: SocketAddr::from(([127, 0, 0, 1], 8081)),
        swarm_port,
        public_gateway_hosts: vec!["example.com".to_string()],
    }
}

#[test]
fn apply_config_writes_its_keys_and_keeps_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config"),
        r#"{"Identity":{"PeerID":"mine"},"API":{"HTTPHeaders":{}},"Addresses":{"Gateway":["/ip4/127.0.0.1/tcp/8080"],"Swarm":["/ip4/0.0.0.0/tcp/4001"]},"Datastore":{"StorageMax":"10GB","GCPeriod":"1h"}}"#,
    )
    .unwrap();
    let api = access(41234);
    apply_config(dir.path(), &settings(api.clone(), None)).unwrap();
    let text = std::fs::read_to_string(dir.path().join("config")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(config["Identity"]["PeerID"], "mine");
    assert_eq!(config["API"]["HTTPHeaders"], json!({}));
    assert_eq!(
        config["API"]["Authorizations"],
        json!({"swing": {"AuthSecret": "bearer:abcd", "AllowedPaths": ["/api/v0"]}})
    );
    assert_eq!(
        config["Addresses"],
        json!({
            "API": ["/ip4/127.0.0.1/tcp/41234"],
            "Gateway": ["/ip4/127.0.0.1/tcp/8081"],
            "Swarm": ["/ip4/0.0.0.0/tcp/4001"],
        })
    );
    assert_eq!(
        config["Datastore"],
        json!({"StorageMax": "123456789", "GCPeriod": "1h"})
    );
    assert_eq!(config["Provide"], json!({"Strategy": "pinned+mfs"}));
    assert_eq!(
        config["Gateway"],
        json!({
            "NoFetch": true,
            "NoDNSLink": true,
            "PublicGateways": public_gateways_json(&["example.com".to_string()]),
            "HTTPHeaders": gateway_http_headers_json(),
        })
    );
    assert_eq!(api.secret.authorization(), "Bearer abcd");

    apply_config(dir.path(), &settings(api.clone(), Some(4321))).unwrap();
    let text = std::fs::read_to_string(dir.path().join("config")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        config["Addresses"]["Swarm"],
        json!(default_swarm_addrs(4321))
    );

    std::fs::write(dir.path().join("config"), r#"{"API":null}"#).unwrap();
    apply_config(dir.path(), &settings(api, None)).unwrap();
    let text = std::fs::read_to_string(dir.path().join("config")).unwrap();
    assert!(
        text.contains("bearer:abcd") && text.contains("/tcp/41234"),
        "{text}"
    );

    std::fs::write(dir.path().join("config"), r#"{"Gateway":[]}"#).unwrap();
    let err = apply_config(dir.path(), &settings(access(41234), None)).unwrap_err();
    assert!(err.to_string().contains("Gateway"), "{err}");
}

#[cfg(unix)]
#[test]
fn apply_config_writes_through_a_symlinked_repo_config() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real-config");
    std::fs::write(&real, r#"{"Identity":{"PeerID":"mine"}}"#).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    std::os::unix::fs::symlink(&real, repo.join("config")).unwrap();
    apply_config(&repo, &settings(access(41234), None)).unwrap();
    assert!(
        std::fs::symlink_metadata(repo.join("config"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::read_to_string(&real)
            .unwrap()
            .contains("bearer:abcd")
    );
}

#[tokio::test]
async fn kubo_http_client_sends_the_secret_as_a_bearer_token() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let body = r#"{"ID":"mine"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(resp.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase()
    });
    let secret = ApiSecret::parse("abcd").unwrap();
    let client = crate::ipfs::IpfsClient::with_secret(format!("http://{addr}"), Some(&secret));
    assert_eq!(client.peer_id().await.unwrap(), "mine");
    let request = server.await.unwrap();
    assert!(request.contains("authorization: bearer abcd"), "{request}");
}

#[test]
fn kubo_commands_do_not_inherit_secret_settings() {
    let command = super::binary::kubo_command(Path::new("ipfs"));
    let removed: Vec<_> = command
        .as_std()
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect();
    assert_eq!(removed, ["SWING_NOSTR_SECRET_KEY"]);
}

#[test]
fn find_on_path_skips_relative_entries() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("ipfs-test-exe");
    std::fs::write(&exe, b"").unwrap();
    let relative = std::env::join_paths([PathBuf::from("."), PathBuf::from("")]).unwrap();
    assert_eq!(
        super::binary::find_on_path(&relative, "ipfs-test-exe"),
        None
    );
    let absolute = std::env::join_paths([PathBuf::from("."), dir.path().to_path_buf()]).unwrap();
    assert_eq!(
        super::binary::find_on_path(&absolute, "ipfs-test-exe"),
        Some(exe)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_repo_creates_a_private_repo_dir() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_kubo(dir.path(), "exit 0", "mine");
    let repo = dir.path().join("custom/repo");
    assert!(ensure_repo(&bin, &repo).await.unwrap());
    let mode = std::fs::metadata(&repo).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn locate_binary_errors_on_missing_explicit_path() {
    let missing = PathBuf::from("/nonexistent/path/to/ipfs-binary-that-does-not-exist");
    let err = locate_binary(Some(&missing)).unwrap_err().to_string();
    assert!(err.contains("Kubo binary not found"), "{err}");
}

#[test]
fn windows_installer_pins_the_same_kubo_version() {
    let pinned = include_str!("../../packaging/windows/kubo.sha512");
    let fields: Vec<&str> = pinned.split_whitespace().collect();
    assert_eq!(fields.len(), 2, "{pinned}");
    assert_eq!(fields[0].len(), 128, "{pinned}");
    assert_eq!(fields[1], format!("kubo_v{KUBO_VERSION}_windows-amd64.zip"));
}

#[test]
fn install_sh_pins_the_same_kubo_version() {
    let script = include_str!("../../packaging/linux/install.sh");
    let value = |name: &str| {
        script
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("{name} is not set in install.sh"))
    };
    assert_eq!(value("KUBO_VERSION"), KUBO_VERSION);
    for name in ["KUBO_SHA512_AMD64", "KUBO_SHA512_ARM64"] {
        let hash = value(name);
        assert!(
            hash.len() == 128 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "{name}={hash}"
        );
    }
}

#[test]
fn pick_free_port_returns_bindable_port() {
    let port = pick_free_port().unwrap();
    assert!(port > 0);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[test]
fn pid_file_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_pid_file(dir.path()).unwrap(), None);
    let pid = std::process::id();
    write_pid_file(dir.path(), pid, 4242).unwrap();
    let record = read_pid_file(dir.path()).unwrap().unwrap();
    assert_eq!(record.pid, pid);
    assert_eq!(record.api_port, 4242);
    assert!(!record.started_at.is_empty());
    #[cfg(target_os = "linux")]
    assert_eq!(Some(record.boot_id), crate::proc::boot_id());
    remove_pid_file(dir.path()).unwrap();
    assert_eq!(read_pid_file(dir.path()).unwrap(), None);
    remove_pid_file(dir.path()).unwrap();
}

#[test]
fn read_pid_file_rejects_unparseable_content() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("kubo.pid"), "not a pid file").unwrap();
    assert!(read_pid_file(dir.path()).is_err());
}

#[tokio::test]
async fn recover_orphan_with_no_pid_file_is_ok() {
    let dir = tempfile::tempdir().unwrap();
    recover_orphan(dir.path(), dir.path()).await.unwrap();
}

#[tokio::test]
async fn recover_orphan_skips_an_unparseable_pid_file() {
    for content in ["not a pid file", ""] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kubo.pid"), content).unwrap();
        recover_orphan(dir.path(), dir.path()).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("kubo.pid")).unwrap(),
            content
        );
    }
}

#[tokio::test]
async fn recover_orphan_removes_stale_pid_file() {
    let dir = tempfile::tempdir().unwrap();
    let record = PidRecord::new(999_999_999, pick_free_port().unwrap(), "0".to_string()).unwrap();
    std::fs::write(
        dir.path().join("kubo.pid"),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();
    recover_orphan(dir.path(), dir.path()).await.unwrap();
    assert_eq!(read_pid_file(dir.path()).unwrap(), None);
}

#[cfg(unix)]
async fn orphan_shutdown_request(access_port_matches: bool) -> String {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id();
    std::thread::spawn(move || child.wait());
    let server = serve("500 Internal Server Error", String::new()).await;
    let record = PidRecord::new(pid, server.port, process_start_marker(pid).unwrap()).unwrap();
    std::fs::write(
        dir.path().join("kubo.pid"),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();
    let port = if access_port_matches {
        server.port
    } else {
        pick_free_port().unwrap()
    };
    write_api_access(dir.path(), &access(port)).unwrap();

    recover_orphan(dir.path(), dir.path()).await.unwrap();

    assert!(!process_alive(pid));
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests[0].clone()
}

#[cfg(unix)]
#[tokio::test]
async fn recover_orphan_sends_the_secret_only_to_the_port_it_was_issued_for() {
    let request = orphan_shutdown_request(true).await;
    assert!(request.contains("authorization: bearer abcd"), "{request}");
    let request = orphan_shutdown_request(false).await;
    assert!(!request.contains("authorization"), "{request}");
}

#[cfg(unix)]
#[tokio::test]
async fn recover_orphan_does_not_kill_on_start_time_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("5")
        .spawn()
        .unwrap();
    let pid = child.id();
    let real_started_at =
        process_start_marker(pid).expect("spawned process should have a start time");
    let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    api.set_nonblocking(true).unwrap();
    let record = PidRecord::new(
        pid,
        api.local_addr().unwrap().port(),
        format!("{real_started_at}-not-the-real-one"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("kubo.pid"),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();

    recover_orphan(dir.path(), dir.path()).await.unwrap();

    assert_eq!(read_pid_file(dir.path()).unwrap(), None);
    assert!(
        process_alive(pid),
        "a start-time mismatch must not kill the process"
    );
    assert_eq!(
        api.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "a start-time mismatch must not send a shutdown to the recorded API port"
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn recover_orphan_does_not_touch_a_process_recorded_before_a_reboot() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("5")
        .spawn()
        .unwrap();
    let pid = child.id();
    let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    api.set_nonblocking(true).unwrap();
    let mut record = PidRecord::new(
        pid,
        api.local_addr().unwrap().port(),
        process_start_marker(pid).unwrap(),
    )
    .unwrap();
    record.boot_id = "00000000-0000-0000-0000-000000000000".to_string();
    std::fs::write(
        dir.path().join("kubo.pid"),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();

    recover_orphan(dir.path(), dir.path()).await.unwrap();

    assert_eq!(read_pid_file(dir.path()).unwrap(), None);
    assert!(process_alive(pid));
    assert_eq!(
        api.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn recover_orphan_skips_a_pid_file_without_a_boot_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("5")
        .spawn()
        .unwrap();
    let pid = child.id();
    let content = json!({
        "pid": pid,
        "api_port": pick_free_port().unwrap(),
        "started_at": process_start_marker(pid).unwrap(),
    })
    .to_string();
    std::fs::write(dir.path().join("kubo.pid"), &content).unwrap();

    recover_orphan(dir.path(), dir.path()).await.unwrap();

    assert!(process_alive(pid));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("kubo.pid")).unwrap(),
        content
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
#[ignore]
async fn full_lifecycle_against_real_kubo() {
    let Some(bin) = test_kubo_bin() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("kubo-repo");

    let created = ensure_repo(&bin, &repo).await.unwrap();
    assert!(created);

    run(&bin, Some(&repo), &["config", "profile", "apply", "test"])
        .await
        .unwrap();

    let api = ApiAccess::generate(pick_free_port().unwrap());
    let api_port = api.port;
    let gateway_port = pick_free_port().unwrap();
    let settings = KuboSettings {
        storage_max: 123_456_789,
        provide_strategy: "pinned+mfs".to_string(),
        api: api.clone(),
        gateway: SocketAddr::from(([127, 0, 0, 1], gateway_port)),
        swarm_port: None,
        public_gateway_hosts: vec!["example.com".to_string()],
    };
    apply_config(&repo, &settings).unwrap();

    std::fs::write(repo.join("api"), "/ip4/127.0.0.1/tcp/1\n").unwrap();
    let mut daemon = Daemon::spawn(&bin, &repo, &api).await.unwrap();

    let health = daemon.wait_healthy(&repo, Duration::from_secs(60)).await;
    if health.is_err() {
        let _ = daemon.wait().await;
    }
    health.unwrap();

    let anonymous = crate::ipfs::IpfsClient::new(api.url());
    assert!(anonymous.peer_id().await.is_err());
    assert_eq!(
        api.client().peer_id().await.unwrap(),
        read_peer_id(&repo).unwrap()
    );

    let config_text = std::fs::read_to_string(repo.join("config")).unwrap();
    let config: serde_json::Value = serde_json::from_str(&config_text).unwrap();
    assert_eq!(config["Datastore"]["StorageMax"], "123456789");
    assert_eq!(config["Provide"]["Strategy"], "pinned+mfs");
    assert_eq!(config["Gateway"]["NoFetch"], true);
    assert_eq!(config["Gateway"]["NoDNSLink"], true);
    assert_eq!(
        config["API"]["Authorizations"]["swing"]["AllowedPaths"],
        json!(["/api/v0"])
    );
    assert_eq!(
        config["Gateway"]["PublicGateways"]["example.com"]["Paths"],
        json!([])
    );
    assert_eq!(
        config["Gateway"]["HTTPHeaders"],
        gateway_http_headers_json()
    );
    assert_eq!(
        config["Addresses"]["API"],
        json!([format!("/ip4/127.0.0.1/tcp/{api_port}")])
    );
    assert_eq!(
        config["Addresses"]["Gateway"],
        json!([format!("/ip4/127.0.0.1/tcp/{gateway_port}")])
    );

    assert_eq!(
        std::fs::read_to_string(repo.join("api")).unwrap().trim(),
        format!("/ip4/127.0.0.1/tcp/{api_port}")
    );

    daemon.stop(Duration::from_secs(10)).await.unwrap();
}

#[cfg(unix)]
async fn spawn_orphan(bin: &Path, dir: &Path) -> (PathBuf, String, u32) {
    let repo = dir.join("kubo-repo");
    let state_dir = dir.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    ensure_repo(bin, &repo).await.unwrap();
    run(bin, Some(&repo), &["config", "profile", "apply", "test"])
        .await
        .unwrap();

    let api = ApiAccess::generate(pick_free_port().unwrap());
    let settings = KuboSettings {
        storage_max: 123_456_789,
        provide_strategy: "pinned+mfs".to_string(),
        api: api.clone(),
        gateway: SocketAddr::from(([127, 0, 0, 1], pick_free_port().unwrap())),
        swarm_port: None,
        public_gateway_hosts: vec![],
    };
    apply_config(&repo, &settings).unwrap();

    let api_url = api.url();
    let mut daemon = Daemon::spawn(bin, &repo, &api).await.unwrap();
    let pid = daemon.pid().unwrap();
    write_pid_file(&state_dir, pid, api.port).unwrap();

    daemon
        .wait_healthy(&repo, Duration::from_secs(60))
        .await
        .unwrap();
    write_api_access(&state_dir, &api).unwrap();

    // Simulate an orphan: the child must survive past this test's own process exit.
    std::mem::forget(daemon);
    // mem::forget skips tokio's orphan reaper, so stand in for it here or the killed child zombies.
    std::thread::spawn(move || unsafe {
        let mut status = 0;
        libc::waitpid(pid as libc::pid_t, &mut status, 0);
    });

    (state_dir, api_url, pid)
}

#[cfg(unix)]
#[tokio::test]
#[ignore]
async fn recover_orphan_shuts_down_a_leftover_daemon_via_its_api() {
    let Some(bin) = test_kubo_bin() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (state_dir, api_url, pid) = spawn_orphan(&bin, dir.path()).await;

    recover_orphan(&state_dir, dir.path()).await.unwrap();

    assert_eq!(read_pid_file(&state_dir).unwrap(), None);
    assert!(
        !process_alive(pid),
        "orphaned Kubo should have exited after the API shutdown"
    );
    let client = reqwest::Client::new();
    let result = client
        .post(format!("{api_url}/api/v0/id"))
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    assert!(
        result.is_err(),
        "Kubo API should no longer answer after recovery"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore]
async fn recover_orphan_falls_back_to_signals_when_api_is_unreachable() {
    let Some(bin) = test_kubo_bin() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (state_dir, _api_url, pid) = spawn_orphan(&bin, dir.path()).await;

    // Overwrite the recorded api_port with a bogus, unlisted one so the graceful-shutdown
    // attempt fails to connect, forcing the start-time check and signal escalation path.
    let mut record = read_pid_file(&state_dir).unwrap().unwrap();
    record.api_port = pick_free_port().unwrap();
    std::fs::write(
        state_dir.join("kubo.pid"),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();

    recover_orphan(&state_dir, dir.path()).await.unwrap();

    assert_eq!(read_pid_file(&state_dir).unwrap(), None);
    assert!(
        !process_alive(pid),
        "orphaned Kubo should have been killed via signals"
    );
}
