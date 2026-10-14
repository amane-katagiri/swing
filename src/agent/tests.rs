use super::test_support::*;
use super::*;
use crate::config::CheckMode;
use crate::ipfs::IpfsClient;
use crate::nip05::VerificationResult;
use crate::nostr::{SignedSite, SiteEvent};
use crate::state;

#[tokio::test]
async fn unfollow_removes_the_account_directory_and_its_verifications() {
    let fx = Fixture::new(nip05_policy(CheckMode::Warn), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.seed("b.example", "bafy-b", 1, 100).await;
    fx.agent.nip05.set(
        "c.example",
        &fx.pubkey.to_hex(),
        VerificationResult::Mismatch,
    );
    fx.apply(fx.event("c.example", "bafy-c", None, 100)).await;
    let other = format!("{}/other", fx.agent.layout.agent_root());
    fx.kubo()
        .mfs
        .insert(format!("{other}/x/1"), "bafy-x".into());

    fx.agent.replace_targets(HashSet::new());
    fx.agent.remove_unfollowed().await;

    assert_eq!(fx.kubo().paths(), vec![format!("{other}/x/1")]);
    let state = fx.agent.state.lock().await;
    assert!(state.sites.is_empty());
    assert!(state.verifications.is_empty());
}

#[tokio::test]
async fn accounts_dropped_while_stopped_are_removed_on_the_first_refresh() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let kept = Keys::generate().public_key();
    fx.seed(D, "bafy-gone", 1, 100).await;
    let kept_key = state::site_key(&kept.to_hex(), D);
    let kept_path = fx.agent.layout.agent_version(&kept.to_hex(), D, 100);
    fx.agent
        .state
        .lock()
        .await
        .apply_store(&kept_key, crate::test_support::version("bafy-kept", 1, 100));
    fx.kubo().mfs.insert(kept_path.clone(), "bafy-kept".into());

    fx.agent.replace_targets(HashSet::from([kept]));
    fx.agent.remove_unfollowed().await;

    let state = fx.agent.state.lock().await;
    assert_eq!(
        state.sites.keys().cloned().collect::<Vec<_>>(),
        vec![kept_key]
    );
    drop(state);
    assert_eq!(fx.kubo().paths(), vec![kept_path]);
    assert!(fx.state_path.exists());
}

#[tokio::test]
async fn report_rounds_run_as_tasks_one_at_a_time() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    let mut tasks = JoinSet::new();
    let now = tokio::time::Instant::now();
    assert!(fx.agent.begin_report_round(now, NO_RESEND, &mut tasks));
    assert!(!fx.agent.begin_report_round(now, NO_RESEND, &mut tasks));
    assert_eq!(tasks.len(), 1);
    while let Some(joined) = tasks.join_next().await {
        joined.unwrap();
    }
    assert_eq!(fx.take_reports().len(), 1);

    assert!(fx.agent.begin_report_round(now, NO_RESEND, &mut tasks));
    assert_eq!(tasks.len(), 1);
    while tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn remove_unfollowed_without_changes_does_not_write_state() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.agent.remove_unfollowed().await;
    assert!(!fx.state_path.exists());
    assert_eq!(fx.cids(D).await, vec!["bafy-a"]);
}

#[tokio::test]
async fn failed_removal_is_cleaned_up_by_the_next_sweep() {
    let mut policy = default_policy();
    policy.keep_versions = 1;
    let fx = Fixture::new(policy, FakeKubo::default());
    fx.seed(D, "bafy-old", 10, 100).await;
    let old_path = fx.path(D, 100);
    fx.kubo().fail_remove.insert(old_path.clone());

    fx.apply(fx.event(D, "bafy-new", None, 200)).await;
    assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
    assert!(fx.kubo().mfs.contains_key(&old_path));

    fx.kubo().fail_remove.clear();
    fx.agent.sweep().await;
    assert_eq!(fx.kubo().paths(), vec![fx.path(D, 200)]);
}

#[tokio::test]
async fn sweep_removes_the_garbage_it_finds_and_nothing_else() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-kept", 1, 100).await;
    let publish = fx.agent.layout.publish_version(&fx.pubkey.to_hex(), D, 1);
    {
        let mut kubo = fx.kubo();
        kubo.mfs.insert(fx.path(D, 50), "bafy-stale".into());
        kubo.mfs.insert(publish.clone(), "bafy-p".into());
        kubo.mfs.insert("/manual/x".into(), "bafy-m".into());
    }

    fx.agent.sweep().await;

    let mut expected = vec![fx.path(D, 100), publish, "/manual/x".to_string()];
    expected.sort();
    assert_eq!(fx.kubo().paths(), expected);
}

#[tokio::test]
async fn sweep_applies_retention_to_idle_sites() {
    let mut policy = default_policy();
    policy.keep_days = 1;
    let fx = Fixture::new(policy, FakeKubo::default());
    let now = now_secs();
    fx.seed(D, "bafy-old", 1, now - 3 * 86_400).await;
    fx.seed(D, "bafy-new", 1, now - 2 * 86_400).await;

    fx.agent.sweep().await;

    assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
    assert_eq!(fx.kubo().paths(), vec![fx.path(D, now - 2 * 86_400)]);
    assert!(fx.state_path.exists());
}

#[tokio::test]
async fn sweep_without_retention_work_does_not_write_state() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.agent.sweep().await;
    assert!(!fx.state_path.exists());
    assert_eq!(fx.kubo().paths(), vec![fx.path(D, 100)]);
}

#[tokio::test]
async fn reconcile_forgets_missing_mismatched_and_incomplete_versions() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-kept", 1, 100).await;
    fx.seed("missing.example", "bafy-missing", 1, 100).await;
    fx.seed("moved.example", "bafy-moved", 1, 100).await;
    fx.seed("broken.example", "bafy-broken", 1, 100).await;
    {
        let mut kubo = fx.kubo();
        kubo.mfs.remove(&fx.path("missing.example", 100));
        kubo.mfs
            .insert(fx.path("moved.example", 100), "bafy-other".into());
        kubo.fail_stat.insert("bafy-broken".into());
    }

    fx.agent.reconcile().await;

    let state = fx.agent.state.lock().await;
    assert_eq!(
        state.sites.keys().cloned().collect::<Vec<_>>(),
        vec![fx.key(D)]
    );
    drop(state);
    assert!(fx.state_path.exists());

    fx.kubo().fail_stat.clear();
    fx.apply(fx.event("broken.example", "bafy-broken", None, 100))
        .await;
    assert_eq!(fx.cids("broken.example").await, vec!["bafy-broken"]);
}

#[tokio::test]
async fn reconcile_without_problems_does_not_write_state() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-kept", 1, 100).await;
    fx.agent.reconcile().await;
    assert!(!fx.state_path.exists());
}

#[tokio::test]
#[ignore]
async fn agent_stores_and_removes_through_real_kubo() {
    let api = std::env::var("SWING_TEST_IPFS_API")
        .unwrap_or_else(|_| "http://127.0.0.1:15001".to_string());
    let kubo = IpfsClient::new(api);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = format!("/swing-test-agent-{nanos}");
    let site = tempfile::tempdir().unwrap();
    std::fs::write(site.path().join("index.html"), format!("{nanos}")).unwrap();
    let cid = kubo
        .add_dir(site.path(), &format!("{root}/origin"))
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mut policy = default_policy();
    policy.keep_versions = 1;
    let mut config = test_config(policy);
    config.ipfs.mfs_root = root.clone();
    let agent = Agent::new(
        config,
        kubo,
        FakeNip05::default(),
        FakeRelay::default(),
        State::default(),
        dir.path().join("state.json"),
        Arc::default(),
    );
    let keys = Keys::generate();
    let pubkey = keys.public_key();
    agent.replace_targets(HashSet::from([pubkey]));
    let event = EventBuilder::new(Kind::Custom(agent.config.nostr.site_event_kind), "")
        .tag(Tag::identifier("a/b.example"))
        .tag(Tag::custom("cid", [cid.clone()]))
        .custom_created_at(Timestamp::from_secs(100))
        .finalize(&keys)
        .unwrap();
    let ev = SignedSite {
        site: SiteEvent {
            pubkey,
            d: "a/b.example".to_string(),
            cid: cid.clone(),
            url: None,
            size: None,
            title: None,
            message: None,
            created_at: 100,
            id: event.id,
        },
        event,
    };

    agent.apply_site_event(&ev).await;
    let path = agent
        .layout
        .agent_version(&pubkey.to_hex(), &ev.site.d, 100);
    assert_eq!(
        agent.ipfs.mfs_stat_cid(&path).await.unwrap(),
        Some(cid.clone())
    );
    let size = agent
        .state
        .lock()
        .await
        .site_bytes(&state::site_key(&pubkey.to_hex(), &ev.site.d));
    assert!(size > 0);

    agent.reconcile().await;
    agent.sweep().await;
    assert!(agent.ipfs.mfs_stat_cid(&path).await.unwrap().is_some());

    agent
        .ipfs
        .mfs_put(
            &cid,
            &agent.layout.agent_version(&pubkey.to_hex(), "junk", 1),
        )
        .await
        .unwrap();
    agent.sweep().await;
    let sites = agent
        .ipfs
        .mfs_list(&agent.layout.agent_account(&pubkey.to_hex()))
        .await
        .unwrap();
    assert_eq!(sites.len(), 1);

    agent.replace_targets(HashSet::new());
    agent.remove_unfollowed().await;
    assert!(
        agent
            .ipfs
            .mfs_list(&agent.layout.agent_root())
            .await
            .unwrap()
            .is_empty()
    );
    agent.ipfs.mfs_remove(&root).await.unwrap();
}
