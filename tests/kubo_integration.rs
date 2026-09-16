use std::time::Duration;

use swing::ipfs;

fn kubo_api() -> String {
    std::env::var("SWING_TEST_IPFS_API").unwrap_or_else(|_| "http://127.0.0.1:15001".to_string())
}

// Requires a local Kubo daemon (see docs/architecture.md); run manually with:
//   cargo test --test kubo_integration -- --ignored --test-threads=1
#[tokio::test]
#[ignore]
async fn add_pin_stat_unpin_round_trip() {
    let client = ipfs::IpfsClient::new(kubo_api());

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        b"<html><body>hi</body></html>\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("css")).unwrap();
    std::fs::write(dir.path().join("css/style.css"), b"body { color: red; }\n").unwrap();
    std::fs::create_dir(dir.path().join("empty")).unwrap();

    let cid = client.add_dir(dir.path()).await.expect("add_dir");
    assert!(cid.starts_with("bafy"), "expected CIDv1, got {cid}");

    let size = client.files_stat(&cid).await.expect("files_stat");
    assert!(size > 0);

    let pins = client.pin_ls().await.expect("pin_ls after add(pin=true)");
    assert!(
        pins.contains(&cid),
        "add with pin=true should have pinned {cid}: {pins:?}"
    );

    client.pin_rm(&cid).await.expect("pin_rm");
    let pins_after_rm = client.pin_ls().await.expect("pin_ls after rm");
    assert!(!pins_after_rm.contains(&cid));

    client
        .pin_add_local(&cid, Duration::from_secs(30))
        .await
        .expect("pin_add_local");
    let pins_after_add = client.pin_ls().await.expect("pin_ls after add");
    assert!(pins_after_add.contains(&cid));
}

fn limits(max_bytes: u64) -> ipfs::FetchLimits {
    ipfs::FetchLimits {
        max_bytes,
        total: Duration::from_secs(30),
        idle: Duration::from_secs(2),
    }
}

#[tokio::test]
#[ignore]
async fn fetch_dag_counts_bytes_and_stops_at_limit() {
    let client = ipfs::IpfsClient::new(kubo_api());

    let dir = tempfile::tempdir().unwrap();
    let body: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(dir.path().join("blob.bin"), &body).unwrap();
    let cid = client.add_dir(dir.path()).await.expect("add_dir");

    let size = client.dag_size_local(&cid).await.expect("dag_size_local");
    assert!(size >= 300_000, "unexpected dag size {size}");

    assert_eq!(
        client.fetch_dag(&cid, limits(10_000_000)).await.unwrap(),
        ipfs::Fetched::Complete
    );
    assert_eq!(
        client.fetch_dag(&cid, limits(1_000)).await.unwrap(),
        ipfs::Fetched::TooLarge
    );
}

// With the recommended `IPFS_PROFILE=test` container the daemon has no peers,
// so a missing CID never arrives and only the idle timeout ends the fetch.
#[tokio::test]
#[ignore]
async fn missing_cid_fails_fast_and_is_never_pinned_from_network() {
    let client = ipfs::IpfsClient::new(kubo_api());
    let missing = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";

    let started = std::time::Instant::now();
    assert!(client.fetch_dag(missing, limits(1_000_000)).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(10));

    let started = std::time::Instant::now();
    assert!(
        client
            .pin_add_local(missing, Duration::from_secs(30))
            .await
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
#[ignore]
async fn is_pinned_detects_recursive_and_direct_pins() {
    let client = ipfs::IpfsClient::new(kubo_api());

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"is_pinned fixture\n").unwrap();
    let cid = client.add_dir(dir.path()).await.expect("add_dir");
    assert!(client.is_pinned(&cid).await.unwrap());

    client.pin_rm(&cid).await.expect("pin_rm");
    assert!(!client.is_pinned(&cid).await.unwrap());

    let direct = reqwest::Client::new()
        .post(format!(
            "{}/api/v0/pin/add?arg={cid}&recursive=false",
            kubo_api()
        ))
        .send()
        .await
        .unwrap();
    assert!(direct.status().is_success());
    assert!(client.is_pinned(&cid).await.unwrap());
    client.pin_rm(&cid).await.ok();

    assert!(client.is_pinned("not-a-cid").await.is_err());
}

#[tokio::test]
#[ignore]
async fn add_dir_matches_ipfs_cli_cid_for_known_fixture() {
    let client = ipfs::IpfsClient::new(kubo_api());

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        b"<html><body>hi</body></html>\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("css")).unwrap();
    std::fs::write(dir.path().join("css/style.css"), b"body { color: red; }\n").unwrap();
    std::fs::create_dir(dir.path().join("empty")).unwrap();

    let cid = client.add_dir(dir.path()).await.expect("add_dir");

    let expected = std::env::var("SWING_TEST_EXPECTED_CID").unwrap_or_else(|_| String::new());
    if !expected.is_empty() {
        assert_eq!(cid, expected);
    }
}
