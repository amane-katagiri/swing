use super::super::test_support::*;
use super::*;
use crate::state::VersionRecord;
use std::collections::HashSet;
use tokio::task::JoinSet;

fn cids(list: &[&str]) -> Vec<String> {
    list.iter().map(|c| c.to_string()).collect()
}

#[tokio::test]
async fn stored_versions_are_reported_after_each_store() {
    let mut policy = default_policy();
    policy.keep_versions = 1;
    let fx = Fixture::new(policy, FakeKubo::default());
    let mut tasks = JoinSet::new();

    fx.agent
        .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
    while tasks.join_next().await.is_some() {}
    assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-1"]))]);

    fx.agent
        .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
    while tasks.join_next().await.is_some() {}
    assert!(fx.take_reports().is_empty());

    fx.agent
        .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
    while tasks.join_next().await.is_some() {}
    assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-2"]))]);
}

#[tokio::test]
async fn reports_are_refreshed_before_they_expire() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.seed(D, "bafy-b", 1, 200).await;

    fx.agent.sync_reports().await;
    assert_eq!(
        fx.take_reports(),
        vec![(fx.key(D), cids(&["bafy-a", "bafy-b"]))]
    );
    fx.agent.sync_reports().await;
    assert!(fx.take_reports().is_empty());

    let first = fx.sent_created_at(&fx.key(D)).await;
    fx.agent
        .reports
        .lock()
        .await
        .sent
        .get_mut(&fx.key(D))
        .unwrap()
        .created_at -= REPORT_TTL / 2;
    fx.agent.sync_reports().await;
    assert_eq!(
        fx.take_reports(),
        vec![(fx.key(D), cids(&["bafy-a", "bafy-b"]))]
    );
    assert!(fx.sent_created_at(&fx.key(D)).await >= first);
}

#[tokio::test]
async fn a_changed_report_is_newer_than_the_previous_one_within_a_second() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.agent.sync_reports().await;
    let first = fx.sent_created_at(&fx.key(D)).await;

    fx.seed(D, "bafy-b", 1, 200).await;
    fx.agent.sync_reports().await;

    assert!(fx.sent_created_at(&fx.key(D)).await > first);
    assert_eq!(fx.take_reports().len(), 2);
}

#[test]
fn reports_in_one_round_get_distinct_times_newer_than_their_previous_ones() {
    assert_eq!(
        report_times(&[None, None, None], 100, true),
        vec![100, 99, 98]
    );
    assert_eq!(
        report_times(&[None, Some(100), Some(99), None], 100, true),
        vec![100, 101, 99, 97]
    );
    assert_eq!(
        report_times(&[Some(500), Some(500)], 100, true),
        vec![500, 501]
    );
    assert_eq!(report_times(&[None, None], 0, true), vec![0, 1]);
}

#[test]
fn reports_sent_before_own_reports_are_read_are_not_backdated() {
    assert_eq!(
        report_times(&[None, None, None], 100, false),
        vec![100, 100, 100]
    );
    assert_eq!(
        report_times(&[None, Some(90), Some(150), None], 100, false),
        vec![100, 100, 150, 100]
    );
}

#[tokio::test]
async fn reports_sent_while_own_reports_are_unread_are_dated_now() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let ds = ["a.example", "b.example", "c.example"];
    for (i, d) in ds.iter().enumerate() {
        fx.seed(d, &format!("bafy-{i}"), 1, 100).await;
    }
    fx.relay().partial_fetch = true;
    let now = now_secs();
    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports().len(), ds.len());
    for d in ds {
        assert!(fx.sent_created_at(&fx.key(d)).await >= now);
    }
}

#[tokio::test]
async fn reports_sent_together_do_not_share_a_second() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let ds = ["a.example", "b.example", "c.example"];
    for (i, d) in ds.iter().enumerate() {
        fx.seed(d, &format!("bafy-{i}"), 1, 100).await;
    }
    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports().len(), ds.len());
    let mut times = HashSet::new();
    for d in ds {
        assert!(times.insert(fx.sent_created_at(&fx.key(d)).await));
    }
}

#[tokio::test]
async fn unfollowed_sites_are_withdrawn_once() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.agent.sync_reports().await;
    fx.take_reports();

    fx.agent.replace_targets(HashSet::new());
    fx.agent.remove_unfollowed().await;
    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports(), vec![(fx.key(D), vec![])]);

    fx.agent.sync_reports().await;
    assert!(fx.take_reports().is_empty());
}

const CID_A: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
const CID_B: &str = "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR";
const CID_MIRRORED: &str = "bafybeigzhpfl7lkz4hnmxg6c4mutrtiqnxpsueszwpz3dy2vu7x4ji27qq";
const CID_PUBLISHED: &str = "bafybeifqjzpkean3aqgk4u7wsp3khcr6ac3c3jqdtsrer7wnlg37zbbisq";
const CID_AB: &str = "bafybeih3ryqpylsmh4siyygdtplff46bgrzjro4xpofu2widxbifkyqgam";

#[tokio::test]
async fn reports_left_on_relays_are_withdrawn_or_kept_on_startup() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, CID_A, 1, 100).await;
    let author = fx.pubkey;
    let now = now_secs();
    let old = |d: &str, cids: &[&str], created_at: u64| {
        nostr::build_replica_report_builder(
            35981,
            35980,
            &author,
            d,
            &cids.iter().map(|c| c.to_string()).collect(),
            Timestamp::from_secs(created_at + REPORT_TTL),
        )
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(&fx.agent.reporter.keys)
        .unwrap()
    };
    let foreign = nostr::build_replica_report_builder(
        35981,
        35980,
        &author,
        "foreign.example",
        &BTreeSet::from([CID_A.to_string()]),
        Timestamp::from_secs(now + REPORT_TTL),
    )
    .finalize(&Keys::generate())
    .unwrap();
    fx.relay().stored = vec![
        old(D, &[CID_A], now - 10),
        old("gone.example", &[CID_A], now - 20),
        old("gone.example", &[CID_B], now - 10),
        old("withdrawn.example", &[], now - 10),
        foreign,
    ];

    fx.agent.sync_reports().await;

    assert_eq!(fx.take_reports(), vec![(fx.key("gone.example"), vec![])]);
    assert!(fx.sent_created_at(&fx.key("gone.example")).await >= now);
}

#[tokio::test]
async fn stale_reports_are_withdrawn_once_the_relays_answer() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    let stale = nostr::build_replica_report_builder(
        35981,
        35980,
        &fx.pubkey,
        "gone.example",
        &BTreeSet::from([CID_B.to_string()]),
        Timestamp::from_secs(now_secs() + REPORT_TTL),
    )
    .custom_created_at(Timestamp::from_secs(now_secs() - 10))
    .finalize(&fx.agent.reporter.keys)
    .unwrap();
    {
        let mut relay = fx.relay();
        relay.stored = vec![stale];
        relay.fail_fetch = true;
    }

    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-a"]))]);

    fx.relay().fail_fetch = false;
    fx.agent.sync_reports().await;
    assert!(fx.take_reports().is_empty());
    assert_eq!(fx.relay().own_fetches, 1);

    fx.agent.sync_reports_in_round().await;
    assert_eq!(fx.take_reports(), vec![(fx.key("gone.example"), vec![])]);
    assert_eq!(fx.relay().own_fetches, 2);
}

#[tokio::test]
async fn own_reports_are_fetched_again_after_a_walk_cut_short() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    let stale = nostr::build_replica_report_builder(
        35981,
        35980,
        &fx.pubkey,
        "gone.example",
        &BTreeSet::from([CID_B.to_string()]),
        Timestamp::from_secs(now_secs() + REPORT_TTL),
    )
    .custom_created_at(Timestamp::from_secs(now_secs() - 10))
    .finalize(&fx.agent.reporter.keys)
    .unwrap();
    fx.relay().partial_fetch = true;

    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-a"]))]);

    {
        let mut relay = fx.relay();
        relay.stored.push(stale);
        relay.partial_fetch = false;
    }
    fx.agent.sync_reports().await;
    assert!(fx.take_reports().is_empty());
    assert_eq!(fx.relay().own_fetches, 1);

    fx.agent.sync_reports_in_round().await;
    assert_eq!(fx.take_reports(), vec![(fx.key("gone.example"), vec![])]);
    assert_eq!(fx.relay().own_fetches, 2);

    fx.agent.sync_reports_in_round().await;
    assert_eq!(fx.relay().own_fetches, 2);
}

#[tokio::test]
async fn a_failed_load_is_retried_once_per_report_round() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.relay().fail_fetch = true;

    fx.agent.sync_reports().await;
    assert_eq!(fx.relay().own_fetches, 1);
    fx.agent.sync_reports().await;
    assert_eq!(fx.relay().own_fetches, 1);

    fx.agent.sync_reports_in_round().await;
    assert_eq!(fx.relay().own_fetches, 2);
    fx.agent.sync_reports().await;
    assert_eq!(fx.relay().own_fetches, 2);

    fx.relay().fail_fetch = false;
    fx.agent.sync_reports_in_round().await;
    assert_eq!(fx.relay().own_fetches, 3);
    fx.agent.sync_reports_in_round().await;
    fx.agent.sync_reports().await;
    assert_eq!(fx.relay().own_fetches, 3);
}

#[tokio::test]
async fn rejected_reports_are_retried() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    fx.relay().reject = true;
    fx.agent.sync_reports().await;
    assert!(fx.take_reports().is_empty());

    fx.relay().reject = false;
    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-a"]))]);
}

#[tokio::test]
async fn a_signing_failure_stops_the_round_and_the_rest_are_retried() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed("a.example", "bafy-a", 1, 100).await;
    fx.seed("b.example", "bafy-b", 1, 100).await;
    fx.relay().fail_sign = true;
    fx.agent.sync_reports().await;
    assert_eq!(fx.relay().send_attempts, 1);
    assert!(fx.take_reports().is_empty());

    fx.relay().fail_sign = false;
    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports().len(), 2);
}

#[tokio::test]
async fn published_versions_are_reported_together_with_mirrored_ones() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.agent.state.lock().await.apply_store(
        &fx.own_key(D),
        VersionRecord {
            cid: CID_MIRRORED.into(),
            size: 1,
            created_at: 100,
            stored_at: 100,
        },
    );
    {
        let mut kubo = fx.kubo();
        kubo.mfs
            .insert(fx.publish_path(D, "100"), CID_MIRRORED.into());
        kubo.mfs
            .insert(fx.publish_path(D, "200"), CID_PUBLISHED.into());
        kubo.mfs
            .insert(fx.publish_path(D, "250"), "bafy-planted".into());
        kubo.mfs
            .insert(fx.publish_path(D, "notes.txt"), "bafy-ignored".into());
        kubo.mfs
            .insert(fx.publish_path("a/b.example", "300"), CID_AB.into());
        let other = fx.agent.layout.publish_version(&fx.pubkey.to_hex(), D, 1);
        kubo.mfs.insert(other, "bafy-not-mine".into());
    }

    fx.agent.sync_reports().await;

    let mut expected = vec![
        (fx.own_key(D), cids(&[CID_PUBLISHED, CID_MIRRORED])),
        (fx.own_key("a/b.example"), cids(&[CID_AB])),
    ];
    expected.sort();
    assert_eq!(fx.take_reports(), expected);

    fx.kubo().mfs.retain(|p, _| !p.contains("a%2Fb.example"));
    fx.agent.sync_reports().await;
    assert_eq!(fx.take_reports(), vec![(fx.own_key("a/b.example"), vec![])]);
}

#[tokio::test]
async fn syncing_records_the_newest_published_version_and_never_lowers_it() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.agent.sync_reports().await;
    assert_eq!(fx.agent.activity.latest_published_at(), Some(0));

    {
        let mut kubo = fx.kubo();
        kubo.mfs.insert(fx.publish_path(D, "100"), CID_A.into());
        kubo.mfs
            .insert(fx.publish_path("a/b.example", "300"), CID_AB.into());
        kubo.mfs.insert(fx.publish_path(D, "999999"), String::new());
        kubo.mfs
            .insert(fx.publish_path(D, "888888"), "bafy-planted".into());
        kubo.mfs
            .insert(fx.publish_path(D, "notes.txt"), "bafy-n".into());
    }
    fx.agent.sync_reports().await;
    assert_eq!(fx.agent.activity.latest_published_at(), Some(300));

    fx.agent.activity.record_published(400);
    fx.kubo().mfs.retain(|p, _| !p.contains("a%2Fb.example"));
    fx.agent.sync_reports().await;
    assert_eq!(fx.agent.activity.latest_published_at(), Some(400));
}

fn report_about_own(fx: &Fixture, reporter: &Keys, d: &str, created_at: u64) -> Event {
    report_about_own_cid(fx, reporter, d, CID_A, created_at)
}

fn report_about_own_cid(
    fx: &Fixture,
    reporter: &Keys,
    d: &str,
    cid: &str,
    created_at: u64,
) -> Event {
    nostr::build_replica_report_builder(
        35981,
        35980,
        &fx.agent.own,
        d,
        &BTreeSet::from([cid.to_string()]),
        Timestamp::from_secs(created_at + REPORT_TTL),
    )
    .custom_created_at(Timestamp::from_secs(created_at))
    .finalize(reporter)
    .unwrap()
}

async fn publish_own(fx: &Fixture, ds: &[&str]) {
    {
        let mut kubo = fx.kubo();
        for d in ds {
            kubo.mfs.insert(fx.publish_path(d, "100"), CID_A.into());
        }
    }
    fx.agent.sync_reports().await;
    fx.take_reports();
}

#[tokio::test]
async fn only_reports_naming_a_published_cid_up_to_now_move_the_replica_report_time() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    publish_own(&fx, &[D]).await;
    let now = now_secs();
    let other = Keys::generate();
    let future = Keys::generate();
    fx.choose_reporters(&[other.public_key(), future.public_key()])
        .await;
    fx.relay().stored = vec![
        report_about_own_cid(&fx, &other, D, CID_B, now - 10),
        report_about_own(&fx, &other, "unpublished.example", now - 10),
        report_about_own(&fx, &future, D, now + 600),
    ];

    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(0));

    fx.relay()
        .stored
        .push(report_about_own(&fx, &other, D, now - 5));
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 5));
}

#[tokio::test]
async fn reports_by_others_about_own_sites_advance_the_replica_report_time() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    publish_own(&fx, &[D, "b.example"]).await;
    let now = now_secs();
    let other = Keys::generate();
    let second = Keys::generate();
    let own_keys = fx.agent.reporter.keys.clone();
    fx.choose_reporters(&[
        other.public_key(),
        second.public_key(),
        own_keys.public_key(),
    ])
    .await;
    let about_someone_else = nostr::build_replica_report_builder(
        35981,
        35980,
        &fx.pubkey,
        D,
        &BTreeSet::from([CID_A.to_string()]),
        Timestamp::from_secs(now + REPORT_TTL),
    )
    .custom_created_at(Timestamp::from_secs(now - 1))
    .tag(Tag::public_key(fx.agent.own))
    .finalize(&other)
    .unwrap();
    fx.relay().stored = vec![
        report_about_own(&fx, &own_keys, D, now - 1),
        about_someone_else,
        report_about_own(&fx, &other, D, now + nostr::MAX_FUTURE_SKEW + 60),
    ];

    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(0));

    fx.relay()
        .stored
        .push(report_about_own(&fx, &other, D, now - 100));
    fx.relay()
        .stored
        .push(report_about_own(&fx, &second, "b.example", now - 50));
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 50));

    fx.relay()
        .stored
        .push(report_about_own(&fx, &own_keys, "b.example", now));
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 50));
    assert_eq!(fx.relay().about_since, vec![None, None, Some(now - 50)]);
}

#[tokio::test]
async fn only_chosen_reporters_move_the_replica_report_time() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    publish_own(&fx, &[D]).await;
    let now = now_secs();
    let chosen = Keys::generate();
    let stranger = Keys::generate();
    fx.choose_reporters(&[chosen.public_key(), fx.agent.own])
        .await;
    fx.relay().stored = vec![report_about_own(&fx, &stranger, D, now - 5)];

    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(0));
    assert_eq!(fx.relay().about_reporters, vec![vec![chosen.public_key()]]);

    fx.relay()
        .stored
        .push(report_about_own(&fx, &chosen, D, now - 20));
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 20));
}

#[tokio::test]
async fn a_failed_fetch_keeps_the_replica_report_time() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    publish_own(&fx, &[D]).await;
    let now = now_secs();
    let other = Keys::generate();
    fx.choose_reporters(&[other.public_key()]).await;
    fx.relay().fail_fetch = true;
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), None);

    {
        let mut relay = fx.relay();
        relay.stored = vec![report_about_own(&fx, &other, D, now - 10)];
        relay.fail_fetch = false;
    }
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 10));

    {
        let mut relay = fx.relay();
        relay.stored.push(report_about_own(&fx, &other, D, now));
        relay.fail_fetch = true;
    }
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 10));

    fx.relay().fail_fetch = false;
    fx.agent.record_replica_reports().await;
    assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now));
}

#[test]
fn reports_are_not_touched_for_sites_whose_listing_failed() {
    let sent = BTreeMap::from([
        (
            "me:a".to_string(),
            SentReport {
                cids: BTreeSet::from(["x".to_string()]),
                created_at: 0,
            },
        ),
        (
            "me:b".to_string(),
            SentReport {
                cids: BTreeSet::from(["x".to_string()]),
                created_at: 0,
            },
        ),
        (
            "other:c".to_string(),
            SentReport {
                cids: BTreeSet::from(["x".to_string()]),
                created_at: 0,
            },
        ),
    ]);
    let held = Held {
        cids: BTreeMap::from([("me:b".to_string(), BTreeSet::from(["y".to_string()]))]),
        unknown: BTreeSet::from(["me:a".to_string(), "me:b".to_string()]),
        unknown_prefix: None,
    };
    assert_eq!(
        reports_to_send(&held, &sent, 1, 100),
        vec![("other:c".to_string(), BTreeSet::new())]
    );

    let held = Held {
        unknown: BTreeSet::new(),
        unknown_prefix: Some("me:".to_string()),
        ..held
    };
    assert_eq!(
        reports_to_send(&held, &sent, 1, 100),
        vec![("other:c".to_string(), BTreeSet::new())]
    );
}
