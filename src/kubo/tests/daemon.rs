use super::*;

#[test]
fn repo_lock_detection_ignores_unrelated_lock_words() {
    assert!(is_repo_lock_error("Error: someone else has the lock"));
    assert!(is_repo_lock_error(
        "cannot acquire lock: Lock FcntlFlock of /x/repo.lock failed"
    ));
    assert!(!is_repo_lock_error("fetched block bafy from peer"));
    assert!(!is_repo_lock_error("blockstore: 12 blocks"));
}

#[cfg(unix)]
#[tokio::test]
async fn daemon_wait_healthy_fails_fast_when_the_child_exits() {
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_kubo(dir.path(), "exit 3", "mine");
    let server = serve_id("mine").await;
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(server.port()))
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
        &writes_api_file_then_sleeps(foreign.port()),
        "mine",
    );
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(foreign.port()))
        .await
        .unwrap();
    let err = daemon
        .wait_healthy(dir.path(), Duration::from_secs(5))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("someone-else") && err.contains("mine"),
        "{err}"
    );
    daemon.stop(Duration::ZERO).await.unwrap();

    let own = serve_id("mine").await;
    let bin = fake_kubo(dir.path(), &writes_api_file_then_sleeps(own.port()), "mine");
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(own.port()))
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
        format!("/ip4/127.0.0.1/tcp/{}", squatter.port()),
    )
    .unwrap();
    let bin = fake_kubo(dir.path(), "exec sleep 30", "mine");
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(squatter.port()))
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
        &writes_api_file_then_sleeps(server.port()),
        "mine",
    );
    std::fs::write(dir.path().join("config"), r#"{"Identity":{}}"#).unwrap();
    let mut daemon = Daemon::spawn(&bin, dir.path(), &access(server.port()))
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
#[ignore]
async fn full_lifecycle_against_real_kubo() {
    let bin = test_kubo_bin();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("kubo-repo");

    let created = ensure_repo(&bin, &repo).await.unwrap();
    assert!(created);
    let settings = configure_test_repo(&bin, &repo, vec!["example.com".to_string()]).await;
    let api = settings.api;
    let api_port = api.port;

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

    assert_eq!(
        std::fs::read_to_string(repo.join("api")).unwrap().trim(),
        format!("/ip4/127.0.0.1/tcp/{api_port}")
    );

    daemon.stop(Duration::from_secs(10)).await.unwrap();
}
