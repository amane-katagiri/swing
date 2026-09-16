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
        .pin_add(&cid, Duration::from_secs(30))
        .await
        .expect("pin_add");
    let pins_after_add = client.pin_ls().await.expect("pin_ls after add");
    assert!(pins_after_add.contains(&cid));
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
