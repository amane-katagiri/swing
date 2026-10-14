use super::*;

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
