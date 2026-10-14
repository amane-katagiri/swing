use super::*;

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
        assert!(read_pid_file(dir.path()).is_err(), "{content:?}");
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
    let server = serve_raw(http_response("500 Internal Server Error", "")).await;
    let record = PidRecord::new(pid, server.port(), process_start_marker(pid).unwrap()).unwrap();
    std::fs::write(
        dir.path().join("kubo.pid"),
        serde_json::to_string(&record).unwrap(),
    )
    .unwrap();
    let port = if access_port_matches {
        server.port()
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

#[cfg(unix)]
async fn spawn_orphan(bin: &Path, dir: &Path) -> (PathBuf, String, u32) {
    let repo = dir.join("kubo-repo");
    let state_dir = dir.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    ensure_repo(bin, &repo).await.unwrap();
    let api = configure_test_repo(bin, &repo, vec![]).await.api;

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
    let bin = test_kubo_bin();
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
    let bin = test_kubo_bin();
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
