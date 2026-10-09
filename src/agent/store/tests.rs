use super::super::test_support::*;
use super::*;
use nostr_sdk::prelude::Keys;
use std::collections::HashSet;

use crate::nip05::VerificationResult;

#[test]
fn rejected_cids_keep_only_the_newest_per_account() {
    let mut attempts = Attempts::default();
    let other = state::site_key("bb", "x.example");
    attempts.reject(&other, "keep".into());
    let alternating = state::site_key("aa", "alt.example");
    attempts.reject(&alternating, "big-1".into());
    attempts.reject(&alternating, "big-2".into());
    attempts.reject(&alternating, "big-1".into());
    assert!(attempts.is_rejected(&alternating, "big-1"));
    assert!(attempts.is_rejected(&alternating, "big-2"));
    for i in 0..REJECTED_PER_ACCOUNT {
        attempts.reject(
            &state::site_key("aa", &format!("{i}.example")),
            format!("c{i}"),
        );
    }
    assert!(!attempts.is_rejected(&alternating, "big-1"));
    assert!(!attempts.is_rejected(&alternating, "big-2"));
    assert!(attempts.is_rejected(&state::site_key("aa", "0.example"), "c0"));
    let last = REJECTED_PER_ACCOUNT - 1;
    assert!(attempts.is_rejected(
        &state::site_key("aa", &format!("{last}.example")),
        &format!("c{last}")
    ));
    assert!(attempts.is_rejected(&other, "keep"));
    let total: usize = attempts.sites.values().map(|s| s.rejected.len()).sum();
    assert_eq!(total, REJECTED_PER_ACCOUNT + 1);
}

#[test]
fn every_site_gets_one_attempt_per_interval_until_it_stores() {
    let mut attempts = Attempts::default();
    let a = state::site_key("aa", "a.example");
    assert_eq!(attempts.try_attempt(&a, false, 1000, 600, 10), Ok(()));
    assert_eq!(
        attempts.try_attempt(&a, false, 1599, 600, 10),
        Err("fetch_attempt_interval")
    );
    assert_eq!(attempts.try_attempt(&a, false, 1600, 600, 10), Ok(()));

    let stored = state::site_key("aa", "stored.example");
    assert_eq!(attempts.try_attempt(&stored, true, 1000, 600, 10), Ok(()));
    assert_eq!(
        attempts.try_attempt(&stored, true, 1001, 600, 10),
        Err("fetch_attempt_interval")
    );
    attempts.clear(&stored);
    assert_eq!(attempts.try_attempt(&stored, true, 1002, 600, 10), Ok(()));
    attempts.reject(&stored, "big".into());
    assert_eq!(
        attempts.try_attempt(&stored, true, 1003, 600, 10),
        Err("fetch_attempt_interval")
    );
    attempts.clear(&stored);
    assert_eq!(attempts.try_attempt(&stored, true, 1004, 600, 10), Ok(()));

    assert_eq!(attempts.try_attempt(&a, false, 5000, 0, 10), Ok(()));
    assert_eq!(attempts.try_attempt(&a, false, 5000, 0, 10), Ok(()));
}

#[test]
fn new_d_tags_of_one_account_are_throttled_together() {
    let mut attempts = Attempts::default();
    for i in 0..3 {
        let key = state::site_key("aa", &format!("{i}.example"));
        assert_eq!(attempts.try_attempt(&key, false, 1000, 600, 3), Ok(()));
    }
    let fourth = state::site_key("aa", "3.example");
    assert_eq!(
        attempts.try_attempt(&fourth, false, 1000, 600, 3),
        Err("fetch_attempts_per_account")
    );
    let other = state::site_key("bb", "0.example");
    assert_eq!(attempts.try_attempt(&other, false, 1000, 600, 3), Ok(()));
    let stored = state::site_key("aa", "stored.example");
    assert_eq!(attempts.try_attempt(&stored, true, 1000, 600, 3), Ok(()));
    assert_eq!(attempts.try_attempt(&fourth, false, 1600, 600, 3), Ok(()));
    assert_eq!(state::account_entries(&attempts.sites, "aa").count(), 1);
}

#[tokio::test]
async fn stores_a_new_site_under_its_versioned_path() {
    let fx = Fixture::new(default_policy(), sized(&[("bafy-new", 20)]));

    fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

    assert_eq!(fx.kubo().paths(), vec![fx.path(D, 200)]);
    assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
    assert_eq!(fx.site_bytes(D).await, 20);
    assert!(fx.state_path.exists());
}

#[tokio::test]
async fn fetch_failure_leaves_state_and_old_versions_untouched() {
    let kubo = FakeKubo::with(|s| {
        s.fail_fetch.insert("bafy-new".into());
    });
    let fx = Fixture::new(default_policy(), kubo);
    fx.seed(D, "bafy-old", 10, 100).await;

    fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

    assert_eq!(fx.cids(D).await, vec!["bafy-old"]);
    assert_eq!(fx.kubo().paths(), vec![fx.path(D, 100)]);
    assert!(!fx.state_path.exists());
}

#[tokio::test]
async fn a_cid_that_is_a_file_is_rejected_before_fetching() {
    let kubo = FakeKubo::with(|s| {
        s.files.insert("bafy-file".into());
    });
    let fx = Fixture::new(default_policy(), kubo);

    fx.apply(fx.event(D, "bafy-file", Some(20), 200)).await;
    fx.apply(fx.event(D, "bafy-file", Some(20), 200)).await;

    assert!(fx.kubo().fetched.is_empty());
    assert!(fx.kubo().paths().is_empty());
    assert!(fx.cids(D).await.is_empty());
    assert!(!fx.state_path.exists());
}

#[tokio::test]
async fn event_from_non_target_pubkey_is_ignored() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.agent.replace_targets(HashSet::new());

    fx.apply(fx.event(D, "bafy-intruder", Some(20), 200)).await;

    assert!(fx.agent.state.lock().await.sites.is_empty());
    assert!(fx.kubo().fetched.is_empty());
}

#[tokio::test]
async fn submit_drops_events_from_pubkeys_outside_the_follow_set_before_queueing() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let mut flood = fx.event(D, "bafy-intruder", None, 200);
    flood.pubkey = Keys::generate().public_key();

    let mut tasks = JoinSet::new();
    for i in 0..50 {
        let mut ev = flood.clone();
        ev.d = format!("intruder-{i}.example");
        fx.agent.submit(ev, &mut tasks);
    }

    assert_eq!(tasks.len(), 0);
    assert!(fx.agent.queue.lock().unwrap().is_empty());
    assert!(fx.kubo().fetched.is_empty());
}

#[tokio::test]
async fn store_failure_records_nothing() {
    let kubo = FakeKubo::with(|s| {
        s.fail_put.insert("bafy-new".into());
    });
    let fx = Fixture::new(default_policy(), kubo);

    fx.apply(fx.event(D, "bafy-new", None, 200)).await;

    assert!(fx.cids(D).await.is_empty());
    assert!(fx.kubo().mfs.is_empty());
}

#[tokio::test]
async fn incomplete_content_is_removed_and_not_recorded() {
    let kubo = FakeKubo::with(|s| {
        s.fail_stat.insert("bafy-new".into());
    });
    let fx = Fixture::new(default_policy(), kubo);

    fx.apply(fx.event(D, "bafy-new", None, 200)).await;

    assert!(fx.cids(D).await.is_empty());
    assert!(fx.kubo().mfs.is_empty());
}

#[tokio::test]
async fn oversized_content_is_aborted_during_fetch_even_with_a_small_size_tag() {
    let mut policy = default_policy();
    policy.max_per_site = 50;
    let fx = Fixture::new(policy, sized(&[("bafy-liar", 1_000)]));

    fx.apply(fx.event(D, "bafy-liar", Some(20), 200)).await;

    assert_eq!(fx.kubo().fetched, vec!["bafy-liar".to_string()]);
    assert!(fx.kubo().put_calls.is_empty());
    assert!(fx.cids(D).await.is_empty());
}

#[tokio::test]
async fn actual_size_is_recorded_and_rechecked_instead_of_the_size_tag() {
    let mut policy = default_policy();
    policy.max_total_storage = 100;
    let fx = Fixture::new(policy, sized(&[("bafy-liar", 80), ("bafy-honest", 30)]));
    fx.seed("other.example", "bafy-other", 40, 100).await;

    fx.apply(fx.event(D, "bafy-liar", Some(1), 200)).await;
    assert!(!fx.kubo().stores("bafy-liar"));
    assert_eq!(fx.site_bytes(D).await, 0);

    fx.apply(fx.event(D, "bafy-honest", Some(1), 300)).await;
    assert!(fx.kubo().stores("bafy-honest"));
    assert_eq!(fx.site_bytes(D).await, 30);
}

#[tokio::test]
async fn a_cid_rejected_after_fetch_is_not_fetched_again_until_it_changes() {
    let mut policy = default_policy();
    policy.max_per_site = 50;
    let kubo = sized(&[("bafy-big", 1_000), ("bafy-small", 10)]);
    kubo.s.lock().unwrap().files.insert("bafy-file".into());
    let fx = Fixture::new(policy, kubo);

    fx.apply(fx.event(D, "bafy-big", None, 200)).await;
    fx.apply(fx.event(D, "bafy-big", None, 200)).await;
    fx.apply(fx.event("f.example", "bafy-file", None, 200))
        .await;
    fx.apply(fx.event("f.example", "bafy-file", None, 200))
        .await;
    assert_eq!(fx.kubo().fetched, vec!["bafy-big"]);

    fx.apply(fx.event(D, "bafy-small", None, 300)).await;
    assert!(fx.kubo().stores("bafy-small"));
}

#[tokio::test]
async fn alternating_oversized_cids_are_fetched_once_each_and_throttled() {
    let mut policy = default_policy();
    policy.max_per_site = 50;
    let kubo = sized(&[("bafy-big-1", 1_000), ("bafy-big-2", 1_000)]);
    let fx = Fixture::new(policy, kubo);

    for (cid, at) in [
        ("bafy-big-1", 200),
        ("bafy-big-2", 201),
        ("bafy-big-1", 202),
    ] {
        fx.apply(fx.event(D, cid, None, at)).await;
    }
    assert_eq!(fx.kubo().fetched, vec!["bafy-big-1", "bafy-big-2"]);

    let mut policy = default_policy();
    policy.min_update_interval = 3600;
    let fx = Fixture::new(policy, sized(&[("bafy-big-1", 1_000)]));
    fx.kubo().fail_fetch.insert("bafy-new".into());
    fx.apply(fx.event(D, "bafy-new", None, 200)).await;
    fx.apply(fx.event(D, "bafy-new", None, 201)).await;
    assert_eq!(fx.kubo().fetched, vec!["bafy-new"]);
}

#[tokio::test]
async fn failed_updates_of_a_stored_site_are_throttled_and_skip_nip05() {
    let mut policy = nip05_policy(CheckMode::Warn);
    policy.min_update_interval = 3600;
    policy.nip05_cache_ttl = 0;
    let fx = Fixture::new(policy, FakeKubo::default());
    fx.seed(D, "bafy-old", 10, 100).await;
    fx.agent
        .nip05
        .set(D, &fx.pubkey.to_hex(), VerificationResult::Verified);
    fx.kubo().fail_fetch.insert("bafy-hang".into());

    fx.apply(fx.event(D, "bafy-hang", None, 200)).await;
    fx.apply(fx.event(D, "bafy-hang", None, 201)).await;
    assert_eq!(fx.kubo().fetched, vec!["bafy-hang"]);
    assert_eq!(fx.agent.nip05.calls(), 1);
    assert_eq!(fx.cids(D).await, vec!["bafy-old"]);
}

#[tokio::test]
async fn a_failed_fetch_is_retried() {
    let kubo = FakeKubo::with(|s| {
        s.fail_fetch.insert("bafy-new".into());
    });
    let fx = Fixture::new(default_policy(), kubo);

    fx.apply(fx.event(D, "bafy-new", None, 200)).await;
    fx.kubo().fail_fetch.clear();
    fx.apply(fx.event(D, "bafy-new", None, 200)).await;

    assert_eq!(fx.kubo().fetched, vec!["bafy-new", "bafy-new"]);
    assert!(fx.kubo().stores("bafy-new"));
}

#[tokio::test]
async fn the_sweep_leaves_a_version_that_is_being_stored() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let path = fx.path(D, 200);
    fx.kubo().mfs.insert(path.clone(), "bafy-new".into());

    {
        let _storing = Storing::new(&fx.agent.storing, path.clone());
        fx.agent.sweep().await;
        assert_eq!(fx.kubo().paths(), vec![path.clone()]);
    }
    fx.agent.sweep().await;
    assert!(fx.kubo().paths().is_empty());
}

#[tokio::test]
async fn the_sweep_keeps_what_was_recorded_while_it_listed() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let path = fx.path(D, 200);
    let stale = fx
        .agent
        .layout
        .agent_version(&Keys::generate().public_key().to_hex(), D, 100);
    fx.kubo().mfs.insert(path.clone(), "bafy-new".into());
    fx.kubo().mfs.insert(stale.clone(), "bafy-stale".into());

    let snapshot = fx.agent.state.lock().await.clone();
    let garbage = crate::health::find_garbage(&fx.agent.ipfs, &fx.agent.layout, &snapshot).await;
    fx.seed(D, "bafy-new", 1, 200).await;
    fx.agent.remove_garbage(&garbage).await;

    assert_eq!(fx.kubo().paths(), vec![path]);
    assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
}

#[tokio::test]
async fn unfollow_leaves_an_account_with_a_version_being_stored_to_the_sweep() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-old", 1, 100).await;
    let path = fx.path(D, 200);
    fx.kubo().mfs.insert(path.clone(), "bafy-new".into());

    {
        let _storing = Storing::new(&fx.agent.storing, path.clone());
        fx.agent.replace_targets(HashSet::new());
        fx.agent.remove_unfollowed().await;
        assert!(fx.cids(D).await.is_empty());
        assert_eq!(fx.kubo().paths(), vec![fx.path(D, 100), path.clone()]);
    }
    fx.agent.sweep().await;
    assert!(fx.kubo().paths().is_empty());
}

#[tokio::test]
async fn declared_size_over_limit_is_skipped_before_fetching() {
    let mut policy = default_policy();
    policy.max_update_size = 50;
    let fx = Fixture::new(policy, FakeKubo::default());

    fx.apply(fx.event(D, "bafy-big", Some(1_000), 200)).await;

    assert!(fx.kubo().fetched.is_empty());
}

#[tokio::test]
async fn new_version_evicts_the_oldest_per_keep_versions() {
    let mut policy = default_policy();
    policy.keep_versions = 1;
    let fx = Fixture::new(policy, sized(&[("bafy-new", 20)]));
    fx.seed(D, "bafy-old", 10, 100).await;

    fx.apply(fx.event(D, "bafy-new", None, 200)).await;

    assert_eq!(fx.cids(D).await, vec!["bafy-new"]);
    assert_eq!(fx.kubo().paths(), vec![fx.path(D, 200)]);
    assert_eq!(fx.site_bytes(D).await, 20);
}

#[tokio::test]
async fn max_per_account_limits_the_sum_of_an_accounts_sites() {
    let mut policy = default_policy();
    policy.max_per_account = 100;
    let fx = Fixture::new(policy, sized(&[("bafy-b", 60), ("bafy-c", 40)]));
    fx.seed("a.example", "bafy-a", 60, 100).await;

    fx.apply(fx.event("b.example", "bafy-b", None, 200)).await;
    assert!(!fx.kubo().stores("bafy-b"));

    fx.apply(fx.event("c.example", "bafy-c", None, 200)).await;
    assert!(fx.kubo().stores("bafy-c"));
    assert_eq!(fx.site_bytes("c.example").await, 40);
}

#[tokio::test]
async fn no_space_left_rejects_without_touching_the_network() {
    let mut policy = default_policy();
    policy.max_per_account = 100;
    let fx = Fixture::new(policy, sized(&[("bafy-b", 60)]));
    fx.seed("a.example", "bafy-a", 100, 100).await;

    fx.apply(fx.event("b.example", "bafy-b", None, 200)).await;

    assert!(fx.kubo().dir_checks.is_empty());
    assert!(fx.kubo().fetched.is_empty());
    assert!(
        fx.agent
            .attempts
            .lock()
            .unwrap()
            .is_rejected(&fx.key("b.example"), "bafy-b")
    );
}

#[tokio::test]
async fn the_fetch_stops_at_the_space_left_for_the_account_and_the_node() {
    let mut policy = default_policy();
    policy.max_per_account = 100;
    let fx = Fixture::new(policy, sized(&[("bafy-b", 60)]));
    fx.seed("a.example", "bafy-a", 60, 100).await;

    fx.apply(fx.event("b.example", "bafy-b", None, 200)).await;
    assert_eq!(fx.kubo().fetched, vec!["bafy-b"]);
    assert!(fx.kubo().put_calls.is_empty());
    assert!(
        fx.agent
            .attempts
            .lock()
            .unwrap()
            .is_rejected(&fx.key("b.example"), "bafy-b")
    );

    let mut policy = default_policy();
    policy.max_total_storage = 100;
    let fx = Fixture::new(policy, sized(&[("bafy-b", 60)]));
    let other = Keys::generate().public_key().to_hex();
    fx.agent.state.lock().await.apply_store(
        &state::site_key(&other, "a.example"),
        VersionRecord {
            cid: "bafy-a".into(),
            size: 60,
            created_at: 100,
            stored_at: 100,
        },
    );

    fx.apply(fx.event("b.example", "bafy-b", None, 200)).await;
    assert_eq!(fx.kubo().fetched, vec!["bafy-b"]);
    assert!(fx.kubo().put_calls.is_empty());
}

#[test]
fn attempts_of_unfollowed_accounts_are_dropped() {
    let mut attempts = Attempts::default();
    let kept = state::site_key("aa", "a.example");
    let gone = state::site_key("bb", "b.example");
    attempts.reject(&kept, "x".into());
    attempts.reject(&gone, "y".into());
    assert_eq!(attempts.try_attempt(&gone, false, 1000, 600, 10), Ok(()));

    attempts.retain_accounts(|account| account == "aa");

    assert!(attempts.is_rejected(&kept, "x"));
    assert!(!attempts.is_rejected(&gone, "y"));
    assert_eq!(state::account_entries(&attempts.sites, "bb").count(), 0);
}

#[tokio::test]
async fn unfollow_forgets_the_rejected_cids_of_the_account() {
    let mut policy = default_policy();
    policy.max_per_site = 50;
    let fx = Fixture::new(policy, sized(&[("bafy-big", 1_000)]));
    fx.apply(fx.event(D, "bafy-big", None, 200)).await;
    assert!(
        fx.agent
            .attempts
            .lock()
            .unwrap()
            .is_rejected(&fx.key(D), "bafy-big")
    );

    fx.agent.replace_targets(HashSet::new());
    fx.agent.remove_unfollowed().await;

    assert!(fx.agent.attempts.lock().unwrap().sites.is_empty());
}

#[tokio::test]
async fn sites_sharing_a_cid_are_stored_and_removed_independently() {
    let mut policy = default_policy();
    policy.keep_versions = 1;
    let fx = Fixture::new(policy, sized(&[("bafy-shared", 10), ("bafy-a2", 10)]));

    fx.apply(fx.event("a.example", "bafy-shared", None, 200))
        .await;
    fx.apply(fx.event("b.example", "bafy-shared", None, 200))
        .await;
    fx.apply(fx.event("a.example", "bafy-a2", None, 300)).await;

    assert_eq!(
        fx.kubo().paths(),
        vec![fx.path("a.example", 300), fx.path("b.example", 200)]
    );
    assert!(fx.kubo().stores("bafy-shared"));
}

#[tokio::test]
async fn unfollow_during_fetch_does_not_store() {
    let fx = Fixture::new(default_policy(), sized(&[("bafy-new", 10)]));
    let gate = fx.agent.ipfs.gate.write().await;

    let agent = Arc::clone(&fx.agent);
    let ev = fx.event(D, "bafy-new", None, 200);
    let task = tokio::spawn(async move { agent.apply_site_event(&ev).await });
    fx.agent.ipfs.entered_fetch.notified().await;
    fx.agent.replace_targets(HashSet::new());
    fx.agent.remove_unfollowed().await;
    drop(gate);
    task.await.unwrap();

    assert!(fx.kubo().mfs.is_empty());
    assert!(fx.agent.state.lock().await.sites.is_empty());
}

#[tokio::test]
async fn submit_coalesces_events_for_a_busy_site_to_the_newest() {
    let fx = Fixture::new(
        default_policy(),
        sized(&[
            ("bafy-1", 10),
            ("bafy-2", 10),
            ("bafy-3", 10),
            ("bafy-other", 10),
        ]),
    );
    let gate = fx.agent.ipfs.gate.write().await;

    let mut tasks = JoinSet::new();
    fx.agent
        .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
    fx.agent
        .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
    fx.agent
        .submit(fx.event(D, "bafy-3", None, 300), &mut tasks);
    fx.agent
        .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
    fx.agent.submit(
        fx.event("other.example", "bafy-other", None, 100),
        &mut tasks,
    );
    assert_eq!(tasks.len(), 2);
    drop(gate);
    while let Some(joined) = tasks.join_next().await {
        joined.unwrap();
    }

    let mut fetched = fx.kubo().fetched.clone();
    fetched.sort();
    assert_eq!(fetched, vec!["bafy-1", "bafy-3", "bafy-other"]);
    assert!(fx.kubo().stores("bafy-3"));
    assert!(fx.agent.queue.lock().unwrap().is_empty());
}

#[tokio::test]
async fn submit_drops_a_future_event_without_displacing_a_pending_one() {
    let fx = Fixture::new(default_policy(), sized(&[("bafy-1", 10), ("bafy-2", 10)]));
    let gate = fx.agent.ipfs.gate.write().await;

    let mut tasks = JoinSet::new();
    fx.agent
        .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
    let forged_future = now_secs() + crate::nostr::MAX_FUTURE_SKEW + 1;
    fx.agent
        .submit(fx.event(D, "bafy-forged", None, forged_future), &mut tasks);
    fx.agent
        .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
    assert_eq!(tasks.len(), 1);
    drop(gate);
    while let Some(joined) = tasks.join_next().await {
        joined.unwrap();
    }

    assert_eq!(fx.kubo().fetched, vec!["bafy-1", "bafy-2"]);
    assert!(fx.kubo().stores("bafy-2"));
    assert!(fx.agent.queue.lock().unwrap().is_empty());
}

#[tokio::test]
async fn submit_after_completion_runs_again() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let mut tasks = JoinSet::new();
    fx.agent
        .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
    while tasks.join_next().await.is_some() {}
    fx.agent
        .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
    while tasks.join_next().await.is_some() {}
    assert_eq!(fx.kubo().fetched, vec!["bafy-1", "bafy-2"]);
}

#[tokio::test]
async fn nip05_warn_mode_stores_despite_mismatch_and_records_result() {
    let fx = Fixture::new(nip05_policy(CheckMode::Warn), sized(&[("bafy-new", 20)]));
    fx.agent
        .nip05
        .set(D, &fx.pubkey.to_hex(), VerificationResult::Mismatch);

    fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

    assert!(fx.kubo().stores("bafy-new"));
    assert_eq!(fx.verification(D).await.as_deref(), Some("mismatch"));
}

#[tokio::test]
async fn nip05_require_mode_skips_fetch_when_not_verified() {
    let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
    fx.agent
        .nip05
        .set(D, &fx.pubkey.to_hex(), VerificationResult::NotApplicable);

    fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

    assert!(fx.kubo().fetched.is_empty());
    assert!(fx.cids(D).await.is_empty());
    assert_eq!(fx.verification(D).await.as_deref(), Some("not_applicable"));
}

#[tokio::test]
async fn nip05_require_mode_stores_when_verified() {
    let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
    fx.agent
        .nip05
        .set(D, &fx.pubkey.to_hex(), VerificationResult::Verified);

    fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

    assert!(fx.kubo().stores("bafy-new"));
    assert_eq!(fx.verification(D).await.as_deref(), Some("verified"));
}

#[tokio::test]
async fn nip05_off_mode_never_calls_verifier() {
    let fx = Fixture::new(nip05_policy(CheckMode::Off), FakeKubo::default());

    fx.apply(fx.event(D, "bafy-new", Some(20), 200)).await;

    assert!(fx.kubo().stores("bafy-new"));
    assert_eq!(fx.agent.nip05.calls(), 0);
    assert_eq!(fx.verification(D).await, None);
}

#[tokio::test]
async fn nip05_result_is_cached_until_ttl_expires() {
    let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
    fx.agent
        .nip05
        .set(D, &fx.pubkey.to_hex(), VerificationResult::Verified);

    fx.apply(fx.event(D, "bafy-1", None, 200)).await;
    fx.apply(fx.event(D, "bafy-2", None, 300)).await;
    assert_eq!(fx.agent.nip05.calls(), 1);
    assert!(fx.kubo().stores("bafy-2"));

    fx.agent
        .state
        .lock()
        .await
        .verifications
        .get_mut(&fx.key(D))
        .unwrap()
        .checked_at -= 86_400;
    fx.apply(fx.event(D, "bafy-3", None, 400)).await;
    assert_eq!(fx.agent.nip05.calls(), 2);
}

#[tokio::test]
async fn nip05_errors_are_cached_for_a_shorter_time() {
    let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
    fx.agent.nip05.set(
        D,
        &fx.pubkey.to_hex(),
        VerificationResult::Error("timeout".into(), nip05::ErrorCategory::Timeout),
    );

    fx.apply(fx.event(D, "bafy-1", None, 200)).await;
    fx.apply(fx.event(D, "bafy-1", None, 200)).await;
    assert_eq!(fx.agent.nip05.calls(), 1);

    fx.agent
        .state
        .lock()
        .await
        .verifications
        .get_mut(&fx.key(D))
        .unwrap()
        .checked_at -= NIP05_ERROR_CACHE_TTL;
    fx.apply(fx.event(D, "bafy-1", None, 200)).await;
    assert_eq!(fx.agent.nip05.calls(), 2);
    assert!(fx.kubo().fetched.is_empty());
}

#[tokio::test]
async fn skipped_events_do_not_trigger_nip05() {
    let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
    fx.seed(D, "bafy-1", 10, 100).await;

    fx.apply(fx.event(D, "bafy-1", None, 100)).await;

    assert_eq!(fx.agent.nip05.calls(), 0);
}

#[tokio::test]
async fn max_sites_per_account_skips_new_sites_before_fetching() {
    let mut policy = default_policy();
    policy.max_sites_per_account = 2;
    let fx = Fixture::new(policy, FakeKubo::default());
    fx.seed("a.example", "bafy-a", 1, 100).await;
    fx.seed("b.example", "bafy-b", 1, 100).await;

    fx.apply(fx.event("c.example", "bafy-c", None, 200)).await;
    assert!(fx.kubo().fetched.is_empty());

    fx.apply(fx.event("a.example", "bafy-a2", None, 200)).await;
    assert_eq!(fx.kubo().fetched, vec!["bafy-a2"]);
}

#[tokio::test]
async fn submit_caps_in_progress_sites_per_account() {
    let mut policy = default_policy();
    policy.max_sites_per_account = 2;
    let fx = Fixture::new(policy, FakeKubo::default());
    let gate = fx.agent.ipfs.gate.write().await;

    let mut tasks = JoinSet::new();
    for d in ["a.example", "b.example", "c.example"] {
        fx.agent.submit(fx.event(d, "bafy", None, 100), &mut tasks);
    }
    fx.agent
        .submit(fx.event("a.example", "bafy-a2", None, 200), &mut tasks);
    assert_eq!(tasks.len(), 2);
    drop(gate);
    while let Some(joined) = tasks.join_next().await {
        joined.unwrap();
    }
    let mut fetched = fx.kubo().fetched.clone();
    fetched.sort();
    assert_eq!(fetched, vec!["bafy", "bafy", "bafy-a2"]);
}

#[tokio::test]
async fn verifications_of_unstored_sites_are_pruned_per_account() {
    let mut policy = nip05_policy(CheckMode::Require);
    policy.max_sites_per_account = 2;
    let fx = Fixture::new(policy, FakeKubo::default());
    for d in ["a.example", "b.example", "c.example"] {
        fx.agent
            .nip05
            .set(d, &fx.pubkey.to_hex(), VerificationResult::Mismatch);
        fx.apply(fx.event(d, "bafy", None, 200)).await;
    }
    assert_eq!(fx.agent.state.lock().await.verifications.len(), 2);
}
