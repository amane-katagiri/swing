use super::*;

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
        format!("/ip4/127.0.0.1/tcp/{}", stale.port()),
    )
    .unwrap();
    write_api_access(dir.path(), &access(own.port())).unwrap();
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
