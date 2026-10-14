use super::*;
use crate::test_support::kubo_replying;

#[test]
fn percent_encode_leaves_safe_chars_and_slash_boundary() {
    assert_eq!(percent_encode_segment("index.html"), "index.html");
    assert_eq!(percent_encode_segment("a b"), "a%20b");
    assert_eq!(percent_encode_relative_path("css/a b.css"), "css/a%20b.css");
}

#[test]
fn bandwidth_reads_the_totals_of_stats_bw() {
    let bw: Bandwidth =
        serde_json::from_str(r#"{"TotalIn":123,"TotalOut":456,"RateIn":1.5,"RateOut":2.25}"#)
            .unwrap();
    assert_eq!(
        bw,
        Bandwidth {
            total_in: 123,
            total_out: 456
        }
    );
}

#[test]
fn autonat_reads_reachability_nat_kind_and_address_lists() {
    let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let cases = [
        (
            "bare reachability",
            r#"{"reachability":"Private"}"#,
            AutoNat {
                reachability: Reachability::Private,
                nat: None,
                reachable: vec![],
                unreachable: vec![],
                unknown: vec![],
            },
        ),
        (
            "address lists and nat kind",
            r#"{"reachability":"Public","nat":"cgnat","reachable":["/ip4/203.0.113.42/tcp/4001","/ip4/203.0.113.42/udp/4001/quic-v1"],"unreachable":["/ip6/2001:db8::1/tcp/4001"],"unknown":["/ip4/203.0.113.42/udp/4001/webrtc-direct"]}"#,
            AutoNat {
                reachability: Reachability::Public,
                nat: Some(NatKind::CarrierGrade),
                reachable: strings(&[
                    "/ip4/203.0.113.42/tcp/4001",
                    "/ip4/203.0.113.42/udp/4001/quic-v1",
                ]),
                unreachable: strings(&["/ip6/2001:db8::1/tcp/4001"]),
                unknown: strings(&["/ip4/203.0.113.42/udp/4001/webrtc-direct"]),
            },
        ),
        (
            "unexpected values are unknown",
            r#"{"reachability":"Sideways","nat":"","reachable":null}"#,
            AutoNat {
                reachability: Reachability::Unknown,
                nat: None,
                reachable: vec![],
                unreachable: vec![],
                unknown: vec![],
            },
        ),
    ];
    for (case, json, want) in cases {
        assert_eq!(parse_autonat(json).unwrap(), want, "{case}");
    }
    assert!(parse_autonat("{}").is_err());
}

#[tokio::test]
async fn autonat_reports_an_offline_kubo_as_offline_and_keeps_other_failures() {
    let parsed = kubo_replying(&[(
            "swarm/addrs/autonat",
            500,
            r#"{"Message":"this command must be run in online mode. Try running 'ipfs daemon' first","Code":0,"Type":"error"}"#,
        )])
            .await
            .autonat()
            .await
            .unwrap();
    assert_eq!(
        parsed,
        AutoNat {
            reachability: Reachability::Offline,
            nat: None,
            reachable: vec![],
            unreachable: vec![],
            unknown: vec![],
        }
    );

    let err = kubo_replying(&[(
        "swarm/addrs/autonat",
        500,
        r#"{"Message":"something else broke","Code":0,"Type":"error"}"#,
    )])
    .await
    .autonat()
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("something else"), "{err:#}");
}

fn fake_ls() -> axum::Router {
    use axum::extract::Query;
    use std::collections::HashMap;

    axum::Router::new().route(
        "/api/v0/ls",
        axum::routing::post(|Query(q): Query<HashMap<String, String>>| async move {
            assert_eq!(q.get("offline").map(String::as_str), Some("true"));
            let links = match q["arg"].as_str() {
                "root" => serde_json::json!([
                    { "Name": "index.html", "Hash": "f1", "Type": 2 },
                    { "Name": "css", "Hash": "css", "Type": 1 },
                    { "Name": "big", "Hash": "big", "Type": 1 },
                ]),
                "css" => serde_json::json!([{ "Name": "a b.css", "Hash": "f2", "Type": 2 }]),
                "big" => serde_json::json!([{ "Name": "x", "Hash": "f3", "Type": 0 }]),
                other => panic!("unexpected ls of {other}"),
            };
            axum::Json(serde_json::json!({ "Objects": [{ "Hash": q["arg"], "Links": links }] }))
        }),
    )
}

#[tokio::test]
async fn hash_site_sends_the_add_query_with_only_hash() {
    let kubo = crate::test_support::KuboApi::serve("bafyx").await;
    let client = kubo.client();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
    let listing = || SiteListing::read(dir.path()).unwrap();
    assert_eq!(client.add_site(listing()).await.unwrap(), "bafyx");
    assert_eq!(client.hash_site(listing()).await.unwrap(), "bafyx");
    assert_eq!(
        kubo.s.lock().unwrap().adds,
        [
            format!("{ADD_QUERY}&only-hash=false"),
            format!("{ADD_QUERY}&only-hash=true"),
        ]
    );
}

#[tokio::test]
async fn list_files_local_walks_directories_and_caps_the_entries() {
    let addr = crate::test_support::serve_router(fake_ls()).await;
    let client = IpfsClient::new(format!("http://{addr}"));
    assert_eq!(
        client.list_files_local("root", 5).await.unwrap(),
        vec!["big/x", "css/a b.css", "index.html"]
    );
    let err = client.list_files_local("root", 4).await.unwrap_err();
    assert!(err.to_string().contains("more than 4 entries"), "{err}");
}

#[tokio::test]
async fn read_body_refuses_responses_over_the_cap() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf).await;
        let len = MAX_RESPONSE_BYTES + 1;
        let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {len}\r\n\r\n");
        sock.write_all(head.as_bytes()).await.unwrap();
        let chunk = vec![b'x'; 1 << 16];
        let mut sent = 0;
        while sent < len {
            let n = chunk.len().min(len - sent);
            if sock.write_all(&chunk[..n]).await.is_err() {
                break;
            }
            sent += n;
        }
    });
    let client = IpfsClient::new(format!("http://{addr}"));
    let err = client
        .call("stats/bw", "", Duration::from_secs(10))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn the_idle_timeout_only_fires_without_progress() {
    let idle = Duration::from_millis(200);
    let last = std::sync::Mutex::new(tokio::time::Instant::now());
    let busy = async {
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            *last.lock().unwrap() = tokio::time::Instant::now();
        }
        Ok(7)
    };
    assert_eq!(
        with_idle_timeout(busy, &last, idle, "add").await.unwrap(),
        7
    );

    let stalled = std::future::pending::<Result<()>>();
    let err = with_idle_timeout(stalled, &last, idle, "add")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no progress"), "{err}");
}

const KUBO_MISSING: &str = r#"{"Message":"files/ls: file does not exist","Code":0,"Type":"error"}"#;
const KUBO_BROKEN: &str = r#"{"Message":"repo is locked","Code":0,"Type":"error"}"#;

#[tokio::test]
async fn mfs_remove_treats_a_200_with_a_message_as_a_failure() {
    let failing = kubo_replying(&[("files/rm", 200, "Error: cannot remove\n")]).await;
    let err = failing.mfs_remove("/swing/x").await.unwrap_err();
    assert!(
        err.to_string()
            .contains("files/rm failed: Error: cannot remove"),
        "{err}"
    );

    let clean = kubo_replying(&[("files/rm", 200, "\n")]).await;
    clean.mfs_remove("/swing/x").await.unwrap();

    let broken = kubo_replying(&[("files/rm", 500, KUBO_BROKEN)]).await;
    assert!(broken.mfs_remove("/swing/x").await.is_err());
}

#[tokio::test]
async fn a_missing_mfs_path_reads_as_empty_and_other_failures_stay_errors() {
    let missing = kubo_replying(&[
        ("files/ls", 500, KUBO_MISSING),
        ("files/stat", 500, KUBO_MISSING),
    ])
    .await;
    assert_eq!(missing.mfs_list("/swing/a").await.unwrap(), vec![]);
    assert_eq!(missing.mfs_stat_cid("/swing/a").await.unwrap(), None);

    let present = kubo_replying(&[
            (
                "files/ls",
                200,
                r#"{"Entries":[{"Name":"d","Type":1,"Size":0,"Hash":"bafyd"},{"Name":"f","Type":0,"Size":3,"Hash":"bafyf"}]}"#,
            ),
            (
                "files/stat",
                200,
                r#"{"Hash":"bafyf","Size":3,"CumulativeSize":11,"Blocks":0,"Type":"file"}"#,
            ),
        ])
        .await;
    assert_eq!(
        present.mfs_list("/swing/a").await.unwrap(),
        vec![
            MfsEntry {
                name: "d".into(),
                is_dir: true,
                cid: "bafyd".into()
            },
            MfsEntry {
                name: "f".into(),
                is_dir: false,
                cid: "bafyf".into()
            },
        ]
    );
    assert_eq!(
        present.mfs_stat_cid("/swing/a/f").await.unwrap().as_deref(),
        Some("bafyf")
    );

    let broken = kubo_replying(&[
        ("files/ls", 500, KUBO_BROKEN),
        ("files/stat", 500, KUBO_BROKEN),
    ])
    .await;
    assert!(broken.mfs_list("/swing/a").await.is_err());
    assert!(broken.mfs_stat_cid("/swing/a").await.is_err());
}

#[tokio::test]
async fn mfs_place_tells_an_existing_entry_from_a_failure() {
    let placed = kubo_replying(&[("files/mkdir", 200, ""), ("files/cp", 200, "")]).await;
    assert!(placed.mfs_place(CID, "/swing/a/b").await.unwrap());

    let exists = kubo_replying(&[
            ("files/mkdir", 200, ""),
            (
                "files/cp",
                500,
                r#"{"Message":"cp: cannot put node in path /swing/a/b: directory already has entry by that name","Code":0,"Type":"error"}"#,
            ),
        ])
        .await;
    assert!(!exists.mfs_place(CID, "/swing/a/b").await.unwrap());

    let broken = kubo_replying(&[("files/mkdir", 200, ""), ("files/cp", 500, KUBO_BROKEN)]).await;
    assert!(broken.mfs_place(CID, "/swing/a/b").await.is_err());

    let no_parent = kubo_replying(&[("files/mkdir", 500, KUBO_BROKEN)]).await;
    assert!(no_parent.mfs_place(CID, "/swing/a/b").await.is_err());
}

const CID: &str = crate::test_support::CID_A;

fn dag_export_router(chunks: usize, chunk_len: usize, gap: Duration) -> axum::Router {
    axum::Router::new().route(
        "/api/v0/dag/export",
        axum::routing::post(move || async move {
            let stream = futures_util::stream::iter(0..chunks).then(move |_| async move {
                tokio::time::sleep(gap).await;
                Ok::<_, std::convert::Infallible>(vec![0u8; chunk_len])
            });
            axum::body::Body::from_stream(stream)
        }),
    )
}

async fn fetch_from(router: axum::Router, limits: FetchLimits) -> Result<Fetched> {
    let addr = crate::test_support::serve_router(router).await;
    IpfsClient::new(format!("http://{addr}"))
        .fetch_dag(CID, limits)
        .await
}

const ROOMY: FetchLimits = FetchLimits {
    max_bytes: 1 << 20,
    total: Duration::from_secs(30),
    idle: Duration::from_secs(10),
};

#[tokio::test]
async fn fetch_dag_reads_to_the_end_within_the_size_cap() {
    let router = dag_export_router(4, 1000, Duration::ZERO);
    let limits = FetchLimits {
        max_bytes: 4000,
        ..ROOMY
    };
    assert_eq!(fetch_from(router, limits).await.unwrap(), Fetched::Complete);
}

#[tokio::test]
async fn fetch_dag_stops_once_the_dag_grows_past_the_cap() {
    let router = dag_export_router(1000, 1000, Duration::ZERO);
    let limits = FetchLimits {
        max_bytes: 3999,
        ..ROOMY
    };
    assert_eq!(fetch_from(router, limits).await.unwrap(), Fetched::TooLarge);
}

#[tokio::test]
async fn fetch_dag_gives_up_on_a_stalled_export() {
    let router = dag_export_router(2, 10, Duration::from_secs(60));
    let limits = FetchLimits {
        idle: Duration::from_millis(200),
        ..ROOMY
    };
    let err = fetch_from(router, limits).await.unwrap_err();
    assert!(format!("{err:#}").contains("stalled"), "{err:#}");
}

#[tokio::test]
async fn fetch_dag_gives_up_on_an_export_that_trickles_past_the_total() {
    let router = dag_export_router(1000, 10, Duration::from_millis(50));
    let limits = FetchLimits {
        total: Duration::from_millis(500),
        idle: Duration::from_secs(5),
        ..ROOMY
    };
    let err = fetch_from(router, limits).await.unwrap_err();
    assert!(format!("{err:#}").contains("timed out"), "{err:#}");
}

#[tokio::test]
async fn fetch_dag_reports_a_refused_export_with_its_message() {
    let client = kubo_replying(&[("dag/export", 500, KUBO_BROKEN)]).await;
    let err = client.fetch_dag(CID, ROOMY).await.unwrap_err();
    let shown = format!("{err:#}");
    assert!(shown.contains("dag/export failed: 500"), "{shown}");
    assert!(shown.contains("repo is locked"), "{shown}");
}

#[tokio::test]
#[ignore]
async fn kubo_client_probe() {
    let Ok(api) = std::env::var("SWING_TEST_KUBO_PROBE_API") else {
        return;
    };
    IpfsClient::new(api)
        .call("id", "", Duration::from_secs(5))
        .await
        .unwrap();
}

#[tokio::test]
async fn kubo_client_ignores_proxy_environment() {
    let kubo = crate::test_support::serve_raw(
        "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
    )
    .await;
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());

    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ipfs::tests::kubo_client_probe",
            "--ignored",
            "--quiet",
        ])
        .env("SWING_TEST_KUBO_PROBE_API", kubo.url())
        .env("HTTP_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("1 passed"), "{stdout}");
    assert_eq!(kubo.requests().len(), 1, "the probe never reached Kubo");
    assert_eq!(
        proxy.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "the Kubo client must not go through the proxy"
    );
}
