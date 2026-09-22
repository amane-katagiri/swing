use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swing::ipfs::{self, IpfsClient};
use swing::mfs::MfsLayout;

fn kubo_api() -> String {
    std::env::var("SWING_TEST_IPFS_API").unwrap_or_else(|_| "http://127.0.0.1:15001".to_string())
}

fn unique_root(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("/swing-test-{name}-{nanos}")
}

fn site_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        b"<html><body>hi</body></html>\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("css")).unwrap();
    std::fs::write(dir.path().join("css/style.css"), b"body { color: red; }\n").unwrap();
    std::fs::create_dir(dir.path().join("empty")).unwrap();
    dir
}

fn blob_fixture(seed: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let body: Vec<u8> = (0..300_000u32).map(|i| (i * seed % 251) as u8).collect();
    std::fs::write(dir.path().join("blob.bin"), &body).unwrap();
    dir
}

fn version_fixture(seed: u32, page: &[u8]) -> tempfile::TempDir {
    let dir = blob_fixture(seed);
    std::fs::write(dir.path().join("index.html"), page).unwrap();
    dir
}

fn limits(max_bytes: u64) -> ipfs::FetchLimits {
    ipfs::FetchLimits {
        max_bytes,
        total: Duration::from_secs(30),
        idle: Duration::from_secs(2),
    }
}

async fn rpc(path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}/api/v0/{path}", kubo_api()))
        .send()
        .await
        .unwrap()
}

async fn run_gc() {
    let gc = rpc("repo/gc").await;
    assert!(gc.status().is_success());
    gc.text().await.unwrap();
}

// Requires a local Kubo daemon (see docs/architecture.md); run manually with:
//   cargo test --test kubo_integration -- --ignored --test-threads=1
#[tokio::test]
#[ignore]
async fn add_dir_into_mfs_round_trip() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("add");
    let layout = MfsLayout::new(root.clone());
    let path = layout.publish_version("pk", "a/b %c", 100);
    let dir = site_fixture();

    let cid = client.add_dir(dir.path(), &path).await.expect("add_dir");
    assert!(cid.starts_with("bafy"), "expected CIDv1, got {cid}");
    assert_eq!(client.mfs_stat_cid(&path).await.unwrap(), Some(cid.clone()));
    assert!(client.dag_size_local(&[cid.as_str()]).await.unwrap() > 0);

    let entries = client
        .mfs_list(&layout.publish_site("pk", "a/b %c"))
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "100");
    assert!(entries[0].is_dir);
    assert_eq!(entries[0].cid, cid);

    let again = client.add_dir(dir.path(), &path).await.expect("re-add");
    assert_eq!(again, cid);

    client.mfs_remove(&path).await.expect("mfs_remove");
    assert_eq!(client.mfs_stat_cid(&path).await.unwrap(), None);
    client
        .mfs_remove(&path)
        .await
        .expect("removing a missing path is a no-op");
    assert!(client.mfs_list(&path).await.unwrap().is_empty());
    client.mfs_remove(&root).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn mfs_put_links_the_same_cid_and_replaces_existing_entries() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("put");
    let a = client
        .add_dir(site_fixture().path(), &format!("{root}/src/a"))
        .await
        .unwrap();
    let b = client
        .add_dir(blob_fixture(3).path(), &format!("{root}/src/b"))
        .await
        .unwrap();

    let path = format!("{root}/agent/pk/site/1");
    client.mfs_put(&a, &path).await.expect("mfs_put");
    assert_eq!(client.mfs_stat_cid(&path).await.unwrap(), Some(a.clone()));
    client.mfs_put(&b, &path).await.expect("mfs_put overwrite");
    assert_eq!(client.mfs_stat_cid(&path).await.unwrap(), Some(b));

    let accounts = client.mfs_list(&format!("{root}/agent")).await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].is_dir);
    client.mfs_remove(&root).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn mfs_entries_protect_content_from_gc_until_the_last_one_is_removed() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("gc");
    let first = format!("{root}/one");
    let second = format!("{root}/two");
    let cid = client
        .add_dir(blob_fixture(17).path(), &first)
        .await
        .unwrap();
    client.mfs_put(&cid, &second).await.unwrap();

    client.mfs_remove(&first).await.unwrap();
    run_gc().await;
    assert!(client.dag_size_local(&[cid.as_str()]).await.is_ok());

    client.mfs_remove(&second).await.unwrap();
    run_gc().await;
    assert!(client.dag_size_local(&[cid.as_str()]).await.is_err());
    client.mfs_remove(&root).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn fetch_dag_counts_bytes_and_stops_at_limit() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("fetch");
    let cid = client
        .add_dir(blob_fixture(7).path(), &format!("{root}/x"))
        .await
        .unwrap();

    let size = client
        .dag_size_local(&[cid.as_str()])
        .await
        .expect("dag_size_local");
    assert!(size >= 300_000, "unexpected dag size {size}");

    assert_eq!(
        client.fetch_dag(&cid, limits(10_000_000)).await.unwrap(),
        ipfs::Fetched::Complete
    );
    assert_eq!(
        client.fetch_dag(&cid, limits(1_000)).await.unwrap(),
        ipfs::Fetched::TooLarge
    );
    client.mfs_remove(&root).await.unwrap();
}

// With the recommended `IPFS_PROFILE=test` container the daemon has no peers,
// so a missing CID never arrives and only the idle timeout ends the fetch.
#[tokio::test]
#[ignore]
async fn missing_cid_fails_fast_and_is_never_fetched_by_mfs_put() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("missing");
    let missing = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";

    let started = std::time::Instant::now();
    assert!(client.fetch_dag(missing, limits(1_000_000)).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(10));

    let started = std::time::Instant::now();
    assert!(client.mfs_put(missing, &format!("{root}/x")).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    client.mfs_remove(&root).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn dag_size_local_fails_fast_on_an_incomplete_dag() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("incomplete");
    let path = format!("{root}/x");
    let cid = client
        .add_dir(blob_fixture(13).path(), &path)
        .await
        .unwrap();
    client.mfs_remove(&root).await.unwrap();

    let refs = rpc(&format!("refs?arg={cid}&recursive=true&unique=true"))
        .await
        .text()
        .await
        .unwrap();
    let leaf: serde_json::Value = serde_json::from_str(refs.lines().last().unwrap()).unwrap();
    let removed = rpc(&format!("block/rm?arg={}", leaf["Ref"].as_str().unwrap())).await;
    assert!(removed.status().is_success());

    let started = std::time::Instant::now();
    assert!(client.dag_size_local(&[cid.as_str()]).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));

    // check_site takes a whole site as complete when one dag/stat over all of
    // its versions succeeds, so one incomplete version has to fail the call.
    let complete = client
        .add_dir(site_fixture().path(), &format!("{root}/ok"))
        .await
        .unwrap();
    assert!(
        client
            .dag_size_local(&[complete.as_str(), cid.as_str()])
            .await
            .is_err()
    );
    client.mfs_remove(&root).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn dag_size_local_counts_blocks_shared_by_versions_once() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("shared");
    let old = client
        .add_dir(version_fixture(17, b"one\n").path(), &format!("{root}/1"))
        .await
        .unwrap();
    let new = client
        .add_dir(version_fixture(17, b"two\n").path(), &format!("{root}/2"))
        .await
        .unwrap();

    let old_size = client.dag_size_local(&[old.as_str()]).await.unwrap();
    let new_size = client.dag_size_local(&[new.as_str()]).await.unwrap();
    let both = client
        .dag_size_local(&[old.as_str(), new.as_str()])
        .await
        .unwrap();

    assert!(both > old_size.max(new_size), "{both} <= one version");
    assert!(both < old_size + new_size, "{both} counts the blob twice");
    client.mfs_remove(&root).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn add_dir_matches_ipfs_cli_cid_for_known_fixture() {
    let client = IpfsClient::new(kubo_api());
    let root = unique_root("fixture");

    let cid = client
        .add_dir(site_fixture().path(), &format!("{root}/x"))
        .await
        .expect("add_dir");
    client.mfs_remove(&root).await.unwrap();

    let expected = std::env::var("SWING_TEST_EXPECTED_CID").unwrap_or_else(|_| String::new());
    if !expected.is_empty() {
        assert_eq!(cid, expected);
    }
}
