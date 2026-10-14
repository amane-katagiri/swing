use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::task::JoinSet;

use super::super::test_support::*;
use super::*;
use crate::nip05::VerificationResult;
use crate::signer::Signer;
use crate::state::{Verification, VersionRecord};
use crate::test_support::{
    CID_A, CID_B, deletion_event as deletion, make_site_event, refusing_relay, relay_holding,
};

fn site_event(keys: &Keys, d: &str, cid: &str, created_at: u64) -> Event {
    make_site_event(keys, 35980, d, cid, created_at)
}

fn record_of(event: &Event) -> VersionRecord {
    let cid = nostr::parse_site_event(event, 35980).unwrap().cid;
    VersionRecord {
        stored_at: 1,
        event: Some(event.clone()),
        ..crate::test_support::version(&cid, 1, event.created_at.as_secs())
    }
}

fn key_of(event: &Event) -> SiteKey {
    state::site_key(&event.pubkey.to_hex(), &event.tags.identifier().unwrap())
}

async fn hold(fx: &Fixture, event: &Event) {
    fx.agent
        .state
        .lock()
        .await
        .apply_store(&key_of(event), record_of(event));
}

async fn ids_on(url: &str) -> Vec<EventId> {
    let client = Client::default();
    client.add_relay(url).await.unwrap();
    client.connect().await;
    let events = client
        .fetch_events(Filter::new().kind(Kind::Custom(35980)))
        .timeout(Duration::from_secs(10))
        .await
        .unwrap();
    client.shutdown().await;
    let mut ids: Vec<EventId> = events.into_iter().map(|e| e.id).collect();
    ids.sort();
    ids
}

async fn resend_through(fx: &Fixture, relays: &[String]) {
    let client = RelayClient::connect(Signer::Local(Keys::generate()), relays)
        .await
        .unwrap();
    resend_held_sites(&client, &fx.agent, &[fx.pubkey]).await;
    client.shutdown().await;
}

fn coordinate(fx: &Fixture, d: &str) -> String {
    format!("35980:{}:{d}", fx.pubkey.to_hex())
}

#[tokio::test]
async fn held_sites_are_sent_again_only_to_relays_that_lack_them() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let held = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let older = site_event(&fx.keys, "a.example", CID_B, now - 200);
    let unfollowed = site_event(&Keys::generate(), "s.example", CID_A, now - 100);
    hold(&fx, &held).await;
    hold(&fx, &unfollowed).await;

    let (_same, same) = relay_holding(std::slice::from_ref(&held)).await;
    let (_older, has_older) = relay_holding(std::slice::from_ref(&older)).await;
    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, &[same.clone(), has_older.clone(), empty.clone()]).await;

    assert_eq!(ids_on(&same).await, vec![held.id]);
    assert_eq!(ids_on(&has_older).await, vec![held.id]);
    assert_eq!(ids_on(&empty).await, vec![held.id]);
}

#[tokio::test]
async fn a_site_superseded_on_any_relay_is_not_sent_again_even_after_relays_drop_the_newer_one() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let held = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let newer = site_event(&fx.keys, "a.example", CID_B, now - 50);
    hold(&fx, &held).await;

    let (_newer, has_newer) = relay_holding(std::slice::from_ref(&newer)).await;
    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, &[has_newer.clone(), empty.clone()]).await;
    assert_eq!(ids_on(&has_newer).await, vec![newer.id]);
    assert!(ids_on(&empty).await.is_empty());
    assert_eq!(
        fx.agent.state.lock().await.withdrawn[&key_of(&held)],
        Withdrawn {
            events: BTreeSet::new(),
            through: Some(now - 51),
        }
    );

    resend_through(&fx, std::slice::from_ref(&empty)).await;
    assert!(ids_on(&empty).await.is_empty());
}

#[tokio::test]
async fn a_site_superseded_in_the_same_second_is_withdrawn_by_its_id() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let x = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let y = site_event(&fx.keys, "a.example", CID_B, now - 100);
    let (newer, held) = if x.id < y.id { (x, y) } else { (y, x) };
    hold(&fx, &held).await;

    let (_newer, has_newer) = relay_holding(std::slice::from_ref(&newer)).await;
    resend_through(&fx, std::slice::from_ref(&has_newer)).await;
    let state = fx.agent.state.lock().await;
    let withdrawn = &state.withdrawn[&key_of(&held)];
    assert_eq!(withdrawn.events, BTreeSet::from([held.id.to_hex()]));
    assert_eq!(withdrawn.through, None);
    assert!(!withdrawn.covers(&newer.id.to_hex(), now - 100));
}

#[tokio::test]
async fn sites_of_authors_unfollowed_since_the_poll_are_not_sent() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let held = site_event(&fx.keys, "a.example", CID_A, now - 100);
    hold(&fx, &held).await;
    fx.agent.replace_targets(HashSet::new());

    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, std::slice::from_ref(&empty)).await;
    assert!(ids_on(&empty).await.is_empty());
}

fn outgoing(event: &Event, to: &[RelayUrl]) -> (Held, Vec<RelayUrl>) {
    let held = Held {
        key: key_of(event),
        pubkey: event.pubkey,
        d: event.tags.identifier().unwrap(),
        cid: nostr::parse_site_event(event, 35980).unwrap().cid,
        version_at: event.created_at.as_secs(),
        event: event.clone(),
        withdrawn: None,
    };
    (held, to.to_vec())
}

async fn send_to_empty(fx: &Fixture, events: &[&Event]) -> Vec<EventId> {
    let (_empty, empty) = relay_holding(&[]).await;
    let client = RelayClient::connect(
        Signer::Local(Keys::generate()),
        std::slice::from_ref(&empty),
    )
    .await
    .unwrap();
    let to = vec![RelayUrl::parse(&empty).unwrap()];
    let sends = events.iter().map(|event| outgoing(event, &to)).collect();
    send_held(&client, &fx.agent, sends).await;
    client.shutdown().await;
    ids_on(&empty).await
}

#[tokio::test]
async fn sites_of_authors_unfollowed_during_the_fetches_are_not_sent() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let followed = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let unfollowed = site_event(&Keys::generate(), "b.example", CID_A, now - 100);
    hold(&fx, &followed).await;
    hold(&fx, &unfollowed).await;
    fx.agent.replace_targets(HashSet::from([fx.pubkey]));

    assert_eq!(
        send_to_empty(&fx, &[&followed, &unfollowed]).await,
        vec![followed.id]
    );
}

#[tokio::test]
async fn sites_replaced_or_withdrawn_during_the_fetches_are_not_sent() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let kept = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let replaced = site_event(&fx.keys, "b.example", CID_A, now - 100);
    let resigned = site_event(&fx.keys, "c.example", CID_A, now - 100);
    let withdrawn = site_event(&fx.keys, "d.example", CID_A, now - 100);
    for event in [&kept, &replaced, &resigned, &withdrawn] {
        hold(&fx, event).await;
    }
    hold(&fx, &site_event(&fx.keys, "b.example", CID_B, now - 50)).await;
    {
        let mut state = fx.agent.state.lock().await;
        let later = site_event(&fx.keys, "c.example", CID_A, now - 50);
        assert!(state.fill_latest_event(&key_of(&resigned), CID_A, &later));
        state.record_withdrawal(
            &key_of(&withdrawn),
            Withdrawn {
                events: BTreeSet::from([withdrawn.id.to_hex()]),
                through: None,
            },
        );
    }

    assert_eq!(
        send_to_empty(&fx, &[&kept, &replaced, &resigned, &withdrawn]).await,
        vec![kept.id]
    );
}

#[tokio::test]
async fn sites_no_longer_verified_by_nip05_during_the_fetches_are_not_sent() {
    let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let verified = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let flipped = site_event(&fx.keys, "b.example", CID_A, now - 100);
    for event in [&verified, &flipped] {
        hold(&fx, event).await;
    }
    {
        let mut state = fx.agent.state.lock().await;
        for (event, status) in [
            (&verified, nip05::STATE_VERIFIED),
            (&flipped, nip05::STATE_ERROR),
        ] {
            state.set_verification(
                &key_of(event),
                Verification {
                    status: status.into(),
                    detail: None,
                    checked_at: 1,
                },
            );
        }
    }

    assert_eq!(
        send_to_empty(&fx, &[&verified, &flipped]).await,
        vec![verified.id]
    );
}

#[tokio::test]
async fn a_later_signing_of_the_held_cid_is_kept_and_sent_on_the_next_check() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let held = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let resigned = site_event(&fx.keys, "a.example", CID_A, now - 50);
    hold(&fx, &held).await;

    let (_resigned, has_resigned) = relay_holding(std::slice::from_ref(&resigned)).await;
    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, &[has_resigned.clone(), empty.clone()]).await;
    assert!(ids_on(&empty).await.is_empty());
    {
        let state = fx.agent.state.lock().await;
        assert!(state.withdrawn.is_empty());
        let latest = state.latest_version(&key_of(&held)).unwrap();
        assert_eq!(latest.event, Some(resigned.clone()));
        assert_eq!(latest.created_at, now - 100);
    }

    resend_through(&fx, &[has_resigned.clone(), empty.clone()]).await;
    assert_eq!(ids_on(&empty).await, vec![resigned.id]);
    assert_eq!(ids_on(&has_resigned).await, vec![resigned.id]);
}

#[tokio::test]
async fn a_withdrawal_is_not_recorded_once_a_newer_version_is_stored_during_the_check() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let held = site_event(&fx.keys, "a.example", CID_A, now - 100);
    hold(&fx, &held).await;
    let candidates = resend_candidates(
        held_site_events(&*fx.agent.state.lock().await, &HashSet::from([fx.pubkey])),
        35980,
    );
    assert_eq!(candidates.len(), 1);

    hold(&fx, &site_event(&fx.keys, "a.example", CID_B, now - 50)).await;
    let withdrawn = candidates
        .into_iter()
        .map(|h| {
            let w = Withdrawn {
                events: BTreeSet::from([h.event.id.to_hex()]),
                through: None,
            };
            (h, w)
        })
        .collect();
    apply_if_still_held(
        &fx.agent,
        withdrawn,
        "recording withdrawn site events",
        "recording the withdrawal of the held site event",
        |state, h, w| state.record_withdrawal(&h.key, w),
    )
    .await;

    assert!(
        !fx.agent
            .state
            .lock()
            .await
            .withdrawn
            .contains_key(&key_of(&held))
    );
}

#[tokio::test]
async fn when_nip05_is_required_only_sites_verified_within_the_cache_ttl_are_sent() {
    let fx = Fixture::new(nip05_policy(CheckMode::Require), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let pubkey_hex = fx.pubkey.to_hex();
    let cases = [
        (
            "fresh.example",
            Some((nip05::STATE_VERIFIED, now - 100)),
            None,
            true,
        ),
        (
            "expired-mismatch.example",
            Some((nip05::STATE_VERIFIED, 1)),
            Some(VerificationResult::Mismatch),
            false,
        ),
        (
            "expired-verified.example",
            Some((nip05::STATE_VERIFIED, 1)),
            Some(VerificationResult::Verified),
            true,
        ),
        (
            "fresh-error.example",
            Some((nip05::STATE_ERROR, now - 100)),
            None,
            false,
        ),
        (
            "expired-error.example",
            Some((nip05::STATE_ERROR, now - 1000)),
            Some(VerificationResult::Verified),
            true,
        ),
        (
            "unchecked.example",
            None,
            Some(VerificationResult::Verified),
            true,
        ),
    ];
    let mut expected = Vec::new();
    for (d, recorded, answer, sent) in &cases {
        let event = site_event(&fx.keys, d, CID_A, now - 100);
        hold(&fx, &event).await;
        if let Some((status, checked_at)) = recorded {
            fx.agent.state.lock().await.set_verification(
                &key_of(&event),
                Verification {
                    status: status.to_string(),
                    detail: None,
                    checked_at: *checked_at,
                },
            );
        }
        if let Some(answer) = answer {
            fx.agent.nip05.set(d, &pubkey_hex, answer.clone());
        }
        if *sent {
            expected.push(event.id);
        }
    }
    expected.sort();

    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, std::slice::from_ref(&empty)).await;
    assert_eq!(ids_on(&empty).await, expected);
    assert_eq!(fx.agent.nip05.calls(), 4);
    let state = fx.agent.state.lock().await;
    let recorded = &state.verifications[&fx.key("expired-mismatch.example")];
    assert_eq!(recorded.status, "mismatch");
    assert!(recorded.checked_at >= now);
    assert!(state.verifications[&fx.key("expired-verified.example")].checked_at >= now);
}

#[tokio::test]
async fn a_withdrawn_site_is_not_sent_again_even_after_relays_drop_the_deletion() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let by_id = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let by_address = site_event(&fx.keys, "b.example", CID_A, now - 100);
    let kept = site_event(&fx.keys, "c.example", CID_A, now - 100);
    for event in [&by_id, &by_address, &kept] {
        hold(&fx, event).await;
    }
    let withdrawal = deletion(
        &fx.keys,
        &[
            ["e", &by_id.id.to_hex()],
            ["a", &coordinate(&fx, "b.example")],
        ],
        now - 10,
    );

    let (_deleting, deleting) = relay_holding(std::slice::from_ref(&withdrawal)).await;
    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, &[deleting.clone(), empty.clone()]).await;
    assert_eq!(ids_on(&empty).await, vec![kept.id]);
    assert_eq!(ids_on(&deleting).await, vec![kept.id]);
    {
        let state = fx.agent.state.lock().await;
        assert_eq!(
            state.withdrawn[&key_of(&by_id)].events,
            BTreeSet::from([by_id.id.to_hex()])
        );
        assert_eq!(
            state.withdrawn[&key_of(&by_address)].through,
            Some(now - 10)
        );
        assert!(!state.withdrawn.contains_key(&key_of(&kept)));
    }
    assert!(fx.state_path.exists());

    let (_forgetful, forgetful) = relay_holding(&[]).await;
    resend_through(&fx, std::slice::from_ref(&forgetful)).await;
    assert_eq!(ids_on(&forgetful).await, vec![kept.id]);
}

#[tokio::test]
async fn a_newer_version_after_an_address_withdrawal_is_sent_again() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let newer = site_event(&fx.keys, "a.example", CID_B, now - 50);
    hold(&fx, &newer).await;
    fx.agent.state.lock().await.record_withdrawal(
        &key_of(&newer),
        Withdrawn {
            events: BTreeSet::from(["00".repeat(32)]),
            through: Some(now - 100),
        },
    );

    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, std::slice::from_ref(&empty)).await;
    assert_eq!(ids_on(&empty).await, vec![newer.id]);
}

#[tokio::test]
async fn a_relay_that_did_not_answer_the_deletion_check_is_not_sent_to() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let held = site_event(&fx.keys, "a.example", CID_A, now - 100);
    hold(&fx, &held).await;

    let (_answering, answering) = relay_holding(&[]).await;
    let (_refusing, refusing) = refusing_relay(Some(Kind::EventDeletion)).await;
    resend_through(&fx, &[answering.clone(), refusing.clone()]).await;

    assert_eq!(ids_on(&answering).await, vec![held.id]);
    assert!(ids_on(&refusing).await.is_empty());
}

#[tokio::test]
async fn when_deletions_fill_the_limit_only_sites_newer_than_the_cutoff_are_sent() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let newer = site_event(&fx.keys, "a.example", CID_A, now - 100);
    let older = site_event(&fx.keys, "b.example", CID_A, now - 1000);
    hold(&fx, &newer).await;
    hold(&fx, &older).await;
    let earlier: Vec<Event> = (0..6)
        .map(|i| {
            deletion(
                &fx.keys,
                &[["a", &coordinate(&fx, "a.example")]],
                now - 500 - i,
            )
        })
        .collect();

    let (_full, full) = relay_holding(&earlier).await;
    let (_empty, empty) = relay_holding(&[]).await;
    resend_through(&fx, &[full.clone(), empty.clone()]).await;

    assert_eq!(ids_on(&full).await, vec![newer.id]);
    let mut both = vec![newer.id, older.id];
    both.sort();
    assert_eq!(ids_on(&empty).await, both);
    assert!(fx.agent.state.lock().await.withdrawn.is_empty());
}

#[tokio::test]
async fn sites_held_back_by_the_cutoff_are_decided_once_the_deleted_ones_are_recorded() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let kept = site_event(&fx.keys, "k.example", CID_A, now - 1000);
    let gone: Vec<Event> = (0..2)
        .map(|i| site_event(&fx.keys, &format!("g{i}.example"), CID_A, now - 2000))
        .collect();
    hold(&fx, &kept).await;
    for event in &gone {
        hold(&fx, event).await;
    }
    let deletions: Vec<Event> = (0..8)
        .map(|i| {
            let d = format!("g{}.example", i % 2);
            deletion(&fx.keys, &[["a", &coordinate(&fx, &d)]], now - 100 - i)
        })
        .collect();

    let (_relay, url) = relay_holding(&deletions).await;
    resend_through(&fx, std::slice::from_ref(&url)).await;
    assert!(ids_on(&url).await.is_empty());
    {
        let state = fx.agent.state.lock().await;
        for event in &gone {
            assert!(state.withdrawn[&key_of(event)].through.is_some());
        }
        assert!(!state.withdrawn.contains_key(&key_of(&kept)));
    }

    resend_through(&fx, std::slice::from_ref(&url)).await;
    assert_eq!(ids_on(&url).await, vec![kept.id]);
}

#[tokio::test]
async fn relays_in_backoff_are_skipped_until_it_expires() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let now = Timestamp::now().as_secs();
    let held = site_event(&fx.keys, "a.example", CID_A, now - 100);
    hold(&fx, &held).await;
    let (_empty, empty) = relay_holding(&[]).await;
    let url = RelayUrl::parse(&empty).unwrap();
    fx.agent
        .resend
        .lock()
        .unwrap()
        .backoff
        .failed(&url, held.id, now_secs());

    resend_through(&fx, std::slice::from_ref(&empty)).await;
    assert!(ids_on(&empty).await.is_empty());

    fx.agent
        .resend
        .lock()
        .unwrap()
        .backoff
        .entries
        .get_mut(&(url.clone(), held.id))
        .unwrap()
        .until = 0;
    resend_through(&fx, std::slice::from_ref(&empty)).await;
    assert_eq!(ids_on(&empty).await, vec![held.id]);
    assert!(fx.agent.resend.lock().unwrap().backoff.entries.is_empty());
}

#[test]
fn held_events_that_do_not_match_their_record_or_are_withdrawn_are_not_candidates() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let good = site_event(&fx.keys, "good.example", CID_A, 100);
    let other_cid = site_event(&fx.keys, "cid.example", CID_A, 100);
    let other_time = site_event(&fx.keys, "time.example", CID_A, 100);
    let withdrawn = site_event(&fx.keys, "gone.example", CID_A, 100);
    let mut state = State::default();
    state.apply_store(&key_of(&good), record_of(&good));
    state.apply_store(
        &key_of(&other_cid),
        VersionRecord {
            cid: CID_B.into(),
            ..record_of(&other_cid)
        },
    );
    state.apply_store(
        &key_of(&other_time),
        VersionRecord {
            created_at: 101,
            ..record_of(&other_time)
        },
    );
    state.apply_store(&key_of(&withdrawn), record_of(&withdrawn));
    state.record_withdrawal(
        &key_of(&withdrawn),
        Withdrawn {
            events: BTreeSet::new(),
            through: Some(100),
        },
    );

    let candidates =
        resend_candidates(held_site_events(&state, &HashSet::from([fx.pubkey])), 35980);
    let ids: Vec<EventId> = candidates.iter().map(|h| h.event.id).collect();
    assert_eq!(ids, vec![good.id]);
}

#[tokio::test]
async fn the_check_runs_on_the_first_poll_and_then_once_per_interval() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let mut tasks = JoinSet::new();
    let start = tokio::time::Instant::now();
    let ran = Arc::new(AtomicUsize::new(0));
    let check = |ran: &Arc<AtomicUsize>| {
        let ran = Arc::clone(ran);
        Some(async move {
            ran.fetch_add(1, Ordering::SeqCst);
        })
    };

    assert!(fx.agent.begin_report_round(start, check(&ran), &mut tasks));
    while tasks.join_next().await.is_some() {}
    assert!(fx.agent.begin_report_round(
        start + RESEND_CHECK_INTERVAL - Duration::from_secs(1),
        check(&ran),
        &mut tasks
    ));
    while tasks.join_next().await.is_some() {}
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert!(
        fx.agent
            .begin_report_round(start + RESEND_CHECK_INTERVAL, check(&ran), &mut tasks)
    );
    while tasks.join_next().await.is_some() {}
    assert_eq!(ran.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_check_runs_after_the_report_round_in_the_same_task() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    let mut tasks = JoinSet::new();
    let start = tokio::time::Instant::now();
    let agent = Arc::clone(&fx.agent);
    let reports_before_check = Arc::new(std::sync::Mutex::new(None));
    let seen = Arc::clone(&reports_before_check);

    assert!(fx.agent.begin_report_round(
        start,
        Some(async move {
            *seen.lock().unwrap() = Some(agent.reporter.s.lock().unwrap().sent.len());
        }),
        &mut tasks
    ));
    assert_eq!(tasks.len(), 1);
    while tasks.join_next().await.is_some() {}
    assert_eq!(*reports_before_check.lock().unwrap(), Some(1));
}

#[tokio::test]
async fn a_check_skipped_because_the_round_is_running_is_retried_on_the_next_poll() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    let mut tasks = JoinSet::new();
    let start = tokio::time::Instant::now();
    let blocker = fx.agent.reports.lock().await;

    assert!(fx.agent.begin_report_round(start, NO_RESEND, &mut tasks));
    assert!(
        !fx.agent
            .begin_report_round(start, Some(async {}), &mut tasks)
    );
    assert_eq!(tasks.len(), 1);

    drop(blocker);
    while tasks.join_next().await.is_some() {}
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    assert!(fx.agent.begin_report_round(
        start + Duration::from_secs(1),
        Some(async move { flag.store(true, Ordering::SeqCst) }),
        &mut tasks
    ));
    while tasks.join_next().await.is_some() {}
    assert!(ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_poll_without_a_decided_follow_set_runs_the_report_round_only() {
    let fx = Fixture::new(default_policy(), FakeKubo::default());
    fx.seed(D, "bafy-a", 1, 100).await;
    let mut tasks = JoinSet::new();
    let start = tokio::time::Instant::now();

    assert!(fx.agent.begin_report_round(start, NO_RESEND, &mut tasks));
    while tasks.join_next().await.is_some() {}
    assert_eq!(fx.take_reports().len(), 1);
    assert!(fx.agent.resend.lock().unwrap().take_due(start));
}

#[test]
fn backoff_doubles_from_the_check_interval_up_to_a_day_and_resets_on_acceptance() {
    let relay = RelayUrl::parse("wss://r.example").unwrap();
    let other = RelayUrl::parse("wss://o.example").unwrap();
    let id = EventId::from_byte_array([1; 32]);
    let mut backoff = Backoff::default();
    assert!(backoff.ready(&relay, id, 0));

    let mut waits = Vec::new();
    for _ in 0..7 {
        backoff.failed(&relay, id, 1000);
        waits.push(backoff.entries[&(relay.clone(), id)].until - 1000);
    }
    assert_eq!(
        waits,
        vec![3600, 7200, 14_400, 28_800, 57_600, 86_400, 86_400]
    );
    assert!(!backoff.ready(&relay, id, 1000 + 86_399));
    assert!(backoff.ready(&relay, id, 1000 + 86_400));
    assert!(backoff.ready(&other, id, 1000));

    backoff.accepted(&relay, id);
    assert!(backoff.ready(&relay, id, 1000));
    backoff.failed(&relay, id, 1000);
    assert_eq!(backoff.entries[&(relay.clone(), id)].until, 1000 + 3600);

    backoff.retain(&HashSet::new());
    assert!(backoff.entries.is_empty());
}
