use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use swing::health;
use swing::ipfs::IpfsClient;
use swing::kubo::{self, ApiAccess, Daemon, KuboSettings};
use swing::mfs::{self, MfsLayout};
use swing::nostr;
use swing::state::{self, State, VersionRecord};

const PK: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const AGENT_VERSION: u64 = 100;
const PUBLISH_VERSION: u64 = 7;

fn tricky_ds() -> Vec<String> {
    let mut ds: Vec<String> = [
        "example.com",
        "with space",
        "bang!",
        "100%",
        "50%25",
        "%2E",
        "a+b",
        "frag#ment",
        "q?x=1&y=2",
        "semi;colon:and,comma",
        "back\\slash",
        "quote\"'`",
        "日本語のサイト",
        ".",
        "..",
        "...",
        "example.com/blog",
        "a/../b",
        "/leading",
        "trailing/",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    ds.push("l".repeat(253));
    ds.push("日".repeat(84));
    for d in &ds {
        nostr::validate_d_tag(d).unwrap_or_else(|e| panic!("{d:?}: {e}"));
    }
    ds
}

fn site_fixture(label: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), label.as_bytes()).unwrap();
    dir
}

fn kubo_bin() -> Option<PathBuf> {
    match std::env::var("SWING_TEST_KUBO_BIN") {
        Ok(bin) => Some(PathBuf::from(bin)),
        Err(_) => {
            eprintln!("skipping: SWING_TEST_KUBO_BIN not set");
            None
        }
    }
}

async fn start_kubo(bin: &Path, repo: &Path) -> Daemon {
    kubo::ensure_repo(bin, repo).await.unwrap();
    let profile = tokio::process::Command::new(bin)
        .args(["config", "profile", "apply", "test"])
        .env("IPFS_PATH", repo)
        .output()
        .await
        .unwrap();
    assert!(profile.status.success(), "{profile:?}");
    let api = ApiAccess::generate(kubo::pick_free_port().unwrap());
    let settings = KuboSettings {
        storage_max: 1_000_000_000,
        provide_strategy: "pinned+mfs".to_string(),
        api: api.clone(),
        gateway: SocketAddr::from(([127, 0, 0, 1], kubo::pick_free_port().unwrap())),
        swarm_port: None,
        public_gateway_hosts: vec![],
    };
    kubo::apply_config(repo, &settings).unwrap();
    let mut daemon = Daemon::spawn(bin, repo, &api).await.unwrap();
    daemon
        .wait_healthy(repo, Duration::from_secs(60))
        .await
        .unwrap();
    daemon
}

async fn listed_sites(ipfs: &IpfsClient, account: &str) -> BTreeMap<String, String> {
    let mut sites = BTreeMap::new();
    for site in ipfs.mfs_list(account).await.unwrap() {
        assert!(site.is_dir, "{}", site.name);
        let d = mfs::site_from_name(&site.name)
            .unwrap_or_else(|| panic!("{:?} does not decode to a d", site.name));
        assert!(sites.insert(d, site.name).is_none());
    }
    sites
}

async fn only_version(ipfs: &IpfsClient, site_path: &str) -> (String, String) {
    let versions = ipfs.mfs_list(site_path).await.unwrap();
    assert_eq!(versions.len(), 1, "{site_path}: {versions:?}");
    (versions[0].name.clone(), versions[0].cid.clone())
}

#[tokio::test]
#[ignore]
async fn tricky_site_names_round_trip_through_real_mfs() {
    let Some(bin) = kubo_bin() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("kubo-repo");
    let daemon = start_kubo(&bin, &repo).await;
    let ipfs = daemon.ipfs().clone();
    let layout = MfsLayout::new("/swing");
    let ds = tricky_ds();

    let mut agent_cids = BTreeMap::new();
    let mut publish_cids = BTreeMap::new();
    let mut state = State::default();
    for (i, d) in ds.iter().enumerate() {
        let staged = ipfs
            .add_dir(
                site_fixture(&format!("agent {i}")).path(),
                &format!("/staging/{i}"),
            )
            .await
            .unwrap();
        ipfs.mfs_put(&staged, &layout.agent_version(PK, d, AGENT_VERSION))
            .await
            .unwrap_or_else(|e| panic!("mfs_put {d:?}: {e:#}"));
        state.sites.insert(
            state::site_key(PK, d),
            vec![VersionRecord {
                cid: staged.clone(),
                size: 0,
                created_at: AGENT_VERSION,
                stored_at: 0,
            }],
        );
        agent_cids.insert(d.clone(), staged);

        let published = ipfs
            .add_dir(
                site_fixture(&format!("publish {i}")).path(),
                &layout.publish_version(PK, d, PUBLISH_VERSION),
            )
            .await
            .unwrap_or_else(|e| panic!("add_dir {d:?}: {e:#}"));
        publish_cids.insert(d.clone(), published);
    }
    ipfs.mfs_remove("/staging").await.unwrap();

    let expected: BTreeSet<&String> = ds.iter().collect();
    let agent_sites = listed_sites(&ipfs, &layout.agent_account(PK)).await;
    assert_eq!(agent_sites.keys().collect::<BTreeSet<_>>(), expected);
    for (d, cid) in &agent_cids {
        let site_path = layout.agent_site(PK, d);
        assert_eq!(
            site_path,
            format!("{}/{}", layout.agent_account(PK), agent_sites[d])
        );
        assert_eq!(
            only_version(&ipfs, &site_path).await,
            (AGENT_VERSION.to_string(), cid.clone())
        );
        assert_eq!(
            ipfs.mfs_stat_cid(&layout.agent_version(PK, d, AGENT_VERSION))
                .await
                .unwrap()
                .as_ref(),
            Some(cid),
            "{d:?}"
        );
    }

    let garbage = health::find_garbage(&ipfs, &layout, &state).await;
    assert!(garbage.paths.is_empty(), "{:?}", garbage.paths);
    assert!(garbage.unlisted.is_empty(), "{:?}", garbage.unlisted);

    let publish_account = layout.publish_account(PK);
    let publish_sites = listed_sites(&ipfs, &publish_account).await;
    assert_eq!(publish_sites.keys().collect::<BTreeSet<_>>(), expected);
    for (d, name) in &publish_sites {
        let (version, cid) = only_version(&ipfs, &format!("{publish_account}/{name}")).await;
        assert_eq!(version, PUBLISH_VERSION.to_string());
        assert_eq!(nostr::canonical_cid(&cid).unwrap(), publish_cids[d]);
    }

    let removed = &ds[..ds.len() / 2];
    for d in removed {
        ipfs.mfs_remove(&layout.agent_site(PK, d))
            .await
            .unwrap_or_else(|e| panic!("mfs_remove {d:?}: {e:#}"));
        assert_eq!(
            ipfs.mfs_stat_cid(&layout.agent_version(PK, d, AGENT_VERSION))
                .await
                .unwrap(),
            None,
            "{d:?}"
        );
    }
    let left = listed_sites(&ipfs, &layout.agent_account(PK)).await;
    assert_eq!(
        left.keys().collect::<BTreeSet<_>>(),
        ds[ds.len() / 2..].iter().collect()
    );
    for d in &ds[ds.len() / 2..] {
        ipfs.mfs_remove(&layout.publish_site(PK, d)).await.unwrap();
    }
    assert_eq!(
        listed_sites(&ipfs, &publish_account)
            .await
            .keys()
            .collect::<BTreeSet<_>>(),
        removed.iter().collect()
    );

    daemon.stop(Duration::from_secs(10)).await.unwrap();
}
