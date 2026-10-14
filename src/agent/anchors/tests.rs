use std::collections::BTreeSet;

use super::super::test_support::*;
use super::*;
use crate::anchor::{AD_REFRESH, NOT_PUBLIC_AFTER, PUBLIC_AFTER, Reason};
use crate::ipfs::Reachability;
use crate::nostr::{AdReachability, parse_anchor_ad};
use crate::test_support::{CID_A, PEER_A, PEER_B, PEER_OWN, replica_report_event};

fn fixture(advertise: bool, max_peers: usize) -> Fixture {
    let mut config = test_config(default_policy());
    config.anchor.advertise = advertise;
    config.anchor.max_peers = max_peers;
    Fixture::with_config(config, FakeKubo::default())
}

async fn settle(fx: &Fixture, observed: Observed, reachability: Reachability) {
    let held = match observed {
        Observed::Public => PUBLIC_AFTER,
        Observed::NotPublic => NOT_PUBLIC_AFTER,
    };
    let mut book = fx.agent.anchors.book.lock().await;
    book.hysteresis = Hysteresis::default();
    book.hysteresis.observe(observed, now_secs() - held);
    fx.kubo().reachability = Some(reachability);
}

fn sent_ads(fx: &Fixture) -> Vec<nostr::AnchorAd> {
    std::mem::take(&mut fx.relay().anchor_sent)
        .iter()
        .map(|e| {
            assert_eq!(e.content, "");
            assert_eq!(
                e.tags
                    .iter()
                    .find(|t| t.kind() == "alt")
                    .and_then(|t| t.content()),
                Some(format!("SWING anchor: {PEER_OWN}").as_str())
            );
            parse_anchor_ad(e).unwrap()
        })
        .collect()
}

fn own_ad(fx: &Fixture, reach: AdReachability, created_at: u64) -> Event {
    nostr::build_anchor_ad_builder(PEER_OWN, reach, Timestamp::from_secs(created_at + AD_TTL))
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(&fx.agent.reporter.keys)
        .unwrap()
}

#[tokio::test]
async fn a_node_that_stays_public_advertises_once_and_then_withdraws() {
    let fx = fixture(true, 4);
    fx.kubo().reachability = Some(Reachability::Public);
    fx.agent.check_anchor_ad().await;
    assert!(sent_ads(&fx).is_empty());
    assert!(!fx.agent.anchors.status.snapshot().advertising);

    settle(&fx, Observed::Public, Reachability::Public).await;
    fx.agent.check_anchor_ad().await;
    let ads = sent_ads(&fx);
    assert_eq!(ads.len(), 1);
    let public = &ads[0];
    assert_eq!(public.peer_id, PEER_OWN);
    assert_eq!(public.reachability, AdReachability::Public);
    assert_eq!(public.expiration, public.created_at + AD_TTL);
    assert!(fx.agent.anchors.status.snapshot().advertising);

    fx.agent.check_anchor_ad().await;
    assert!(sent_ads(&fx).is_empty());

    settle(&fx, Observed::NotPublic, Reachability::Private).await;
    fx.agent.check_anchor_ad().await;
    let ads = sent_ads(&fx);
    assert_eq!(ads.len(), 1);
    assert_eq!(ads[0].reachability, AdReachability::Private);
    assert!(ads[0].created_at > public.created_at);
    assert_eq!(ads[0].expiration, ads[0].created_at + AD_TTL);
    assert!(!fx.agent.anchors.status.snapshot().advertising);

    fx.agent.check_anchor_ad().await;
    assert!(sent_ads(&fx).is_empty());
}

#[tokio::test]
async fn a_restarted_agent_reads_its_ad_back_before_deciding() {
    let fx = fixture(true, 4);
    let now = now_secs();
    let ad = own_ad(&fx, AdReachability::Public, now - 100);
    fx.relay().anchor_ads.push(ad);
    settle(&fx, Observed::Public, Reachability::Public).await;
    fx.agent.check_anchor_ad().await;
    assert!(sent_ads(&fx).is_empty());
    assert!(fx.agent.anchors.status.snapshot().advertising);

    let fx = fixture(true, 4);
    fx.relay()
        .anchor_ads
        .push(own_ad(&fx, AdReachability::Public, now - AD_REFRESH));
    settle(&fx, Observed::Public, Reachability::Public).await;
    fx.agent.check_anchor_ad().await;
    assert_eq!(sent_ads(&fx)[0].reachability, AdReachability::Public);

    let fx = fixture(true, 4);
    fx.relay()
        .anchor_ads
        .push(own_ad(&fx, AdReachability::Public, now - 100));
    settle(&fx, Observed::NotPublic, Reachability::Private).await;
    fx.agent.check_anchor_ad().await;
    assert_eq!(sent_ads(&fx)[0].reachability, AdReachability::Private);
}

#[tokio::test]
async fn an_undecided_node_neither_advertises_nor_withdraws() {
    let fx = fixture(true, 4);
    fx.relay()
        .anchor_ads
        .push(own_ad(&fx, AdReachability::Public, now_secs() - 100));
    fx.kubo().reachability = Some(Reachability::Private);
    fx.agent.check_anchor_ad().await;
    assert!(sent_ads(&fx).is_empty());
    assert_eq!(fx.relay().anchor_fetches, 0);
}

#[tokio::test]
async fn turning_advertising_off_withdraws_right_away() {
    let fx = fixture(false, 4);
    fx.relay()
        .anchor_ads
        .push(own_ad(&fx, AdReachability::Public, now_secs() - 100));
    fx.kubo().reachability = Some(Reachability::Public);
    fx.agent.check_anchor_ad().await;
    assert_eq!(sent_ads(&fx)[0].reachability, AdReachability::Private);

    let fx = fixture(false, 4);
    settle(&fx, Observed::Public, Reachability::Public).await;
    fx.agent.check_anchor_ad().await;
    assert!(sent_ads(&fx).is_empty());
}

#[tokio::test]
async fn with_advertising_off_kubo_is_left_alone_once_nothing_is_live() {
    let fx = fixture(false, 4);
    fx.relay()
        .anchor_ads
        .push(own_ad(&fx, AdReachability::Public, now_secs() - 100));
    fx.agent.check_anchor_ad().await;
    assert_eq!(sent_ads(&fx).len(), 1);
    let calls = fx.kubo().node_calls;
    let fetches = fx.relay().anchor_fetches;
    fx.agent.check_anchor_ad().await;
    fx.agent.check_anchor_ad().await;
    assert_eq!(fx.kubo().node_calls, calls);
    assert_eq!(fx.relay().anchor_fetches, fetches);
    assert!(sent_ads(&fx).is_empty());
}

#[tokio::test]
async fn the_peer_id_is_read_once_and_again_after_kubo_fails() {
    let fx = fixture(true, 4);
    settle(&fx, Observed::Public, Reachability::Public).await;
    fx.agent.check_anchor_ad().await;
    fx.agent.check_anchor_ad().await;
    fx.agent.check_anchor_ad().await;
    assert_eq!(fx.kubo().node_calls, 3 + 1);

    fx.kubo().fail_node = true;
    fx.agent.check_anchor_ad().await;
    fx.kubo().fail_node = false;
    let calls = fx.kubo().node_calls;
    fx.agent.check_anchor_ad().await;
    assert_eq!(fx.kubo().node_calls, calls + 2);
}

#[tokio::test]
async fn a_failing_signer_is_retried_later_not_every_check() {
    let fx = fixture(true, 4);
    fx.relay().fail_sign = true;
    settle(&fx, Observed::Public, Reachability::Public).await;
    fx.agent.check_anchor_ad().await;
    fx.agent.check_anchor_ad().await;
    assert_eq!(fx.relay().send_attempts, 1);
    assert!(!fx.agent.anchors.status.snapshot().advertising);

    fx.relay().fail_sign = false;
    fx.agent.anchors.book.lock().await.retry_at = 0;
    fx.agent.check_anchor_ad().await;
    assert_eq!(sent_ads(&fx).len(), 1);
}

fn peer_ad(keys: &Keys, peer: &str, reach: AdReachability) -> Event {
    let now = now_secs();
    nostr::build_anchor_ad_builder(peer, reach, Timestamp::from_secs(now + AD_TTL))
        .custom_created_at(Timestamp::from_secs(now - 10))
        .finalize(keys)
        .unwrap()
}

#[tokio::test]
async fn followed_anchors_are_chosen_and_peered_and_stale_ones_dropped() {
    let fx = fixture(true, 4);
    let mutual = Keys::generate();
    let coverer = Keys::generate();
    let stranger = Keys::generate();
    let targets = vec![fx.pubkey, mutual.public_key(), coverer.public_key()];
    fx.agent.set_followed_sites(vec![
        (fx.pubkey, D.to_string()),
        (fx.pubkey, "b.example".to_string()),
    ]);
    let now = now_secs();
    {
        let mut relay = fx.relay();
        relay.anchor_ads = vec![
            peer_ad(&mutual, PEER_A, AdReachability::Public),
            peer_ad(&coverer, PEER_B, AdReachability::Public),
            peer_ad(&coverer, PEER_OWN, AdReachability::Public),
            peer_ad(
                &stranger,
                "12D3KooW9xQmjvcbHoKYPPCKBUGiSpUeemz6xjRdiWVcQk12JouQ",
                AdReachability::Public,
            ),
        ];
        relay.followers.insert(mutual.public_key());
        relay.stored.push(replica_report_event(
            &coverer,
            &fx.pubkey,
            D,
            &[CID_A],
            now - 10,
            now + 1000,
        ));
        relay.stored.push(replica_report_event(
            &coverer,
            &fx.pubkey,
            "b.example",
            &[],
            now - 10,
            now + 1000,
        ));
    }
    fx.kubo().peering = BTreeSet::from(["user-configured".to_string(), PEER_B.to_string()]);
    fx.agent
        .anchors
        .status
        .set_managed(BTreeSet::from(["stale".to_string()]));
    fx.kubo().peering.insert("stale".to_string());
    fx.kubo().connected = BTreeSet::from([PEER_B.to_string()]);

    fx.agent.refresh_anchors(&targets).await;

    let snapshot = fx.agent.anchors.status.snapshot();
    let picked: Vec<(&str, Reason, usize)> = snapshot
        .selected
        .iter()
        .map(|s| (s.peer_id.as_str(), s.reason, s.sites))
        .collect();
    assert_eq!(
        picked,
        [(PEER_A, Reason::Mutual, 0), (PEER_B, Reason::Coverage, 1)]
    );
    assert_eq!(
        fx.kubo().peering,
        BTreeSet::from([
            "user-configured".to_string(),
            PEER_A.to_string(),
            PEER_B.to_string()
        ])
    );
    assert_eq!(
        fx.agent.anchors.status.managed(),
        BTreeSet::from([PEER_A.to_string()])
    );
    assert_eq!(
        fx.agent.anchors.status.snapshot().connected,
        Some(BTreeSet::from([PEER_B.to_string()]))
    );
}

#[tokio::test]
async fn connections_to_the_chosen_anchors_are_checked() {
    let fx = fixture(false, 4);
    fx.agent.check_anchor_connections().await;
    assert_eq!(fx.kubo().node_calls, 0);
    assert_eq!(fx.agent.anchors.status.snapshot().connected, None);

    let advertiser = Keys::generate().public_key();
    fx.agent.anchors.status.set_selected(
        [PEER_A, PEER_B]
            .map(|peer_id| Selected {
                peer_id: peer_id.to_string(),
                advertiser,
                reason: Reason::Coverage,
                sites: 1,
            })
            .to_vec(),
    );
    fx.kubo().connected = BTreeSet::from([PEER_A.to_string(), PEER_OWN.to_string()]);
    fx.agent.check_anchor_connections().await;
    assert_eq!(
        fx.agent.anchors.status.snapshot().connected,
        Some(BTreeSet::from([PEER_A.to_string()]))
    );

    fx.kubo().fail_node = true;
    fx.agent.check_anchor_connections().await;
    assert_eq!(fx.agent.anchors.status.snapshot().connected, None);
}

#[tokio::test]
async fn an_anchor_refresh_still_running_is_not_started_twice() {
    let fx = fixture(true, 4);
    let mut tasks = tokio::task::JoinSet::new();
    fx.relay().anchor_gate = Some(std::sync::Arc::new(tokio::sync::Notify::new()));
    fx.agent.replace_targets(HashSet::from([fx.pubkey]));
    assert!(fx.agent.start_anchor_refresh(&mut tasks));
    assert!(!fx.agent.start_anchor_refresh(&mut tasks));
    let gate = fx.relay().anchor_gate.take().unwrap();
    gate.notify_one();
    tasks.join_next().await.unwrap().unwrap();
    assert!(fx.agent.start_anchor_refresh(&mut tasks));
}

#[tokio::test]
async fn zero_max_peers_drops_our_peerings_without_asking_relays() {
    let fx = fixture(true, 0);
    fx.agent
        .anchors
        .status
        .set_managed(BTreeSet::from([PEER_A.to_string()]));
    fx.kubo().peering = BTreeSet::from([PEER_A.to_string(), PEER_B.to_string()]);
    fx.agent.refresh_anchors(&[fx.pubkey]).await;
    assert_eq!(fx.relay().anchor_fetches, 0);
    assert_eq!(fx.kubo().peering, BTreeSet::from([PEER_B.to_string()]));
    assert!(fx.agent.anchors.status.managed().is_empty());
}

#[tokio::test]
async fn a_failed_ad_fetch_keeps_the_current_anchors() {
    let fx = fixture(true, 4);
    fx.agent
        .anchors
        .status
        .set_managed(BTreeSet::from([PEER_A.to_string()]));
    fx.kubo().peering = BTreeSet::from([PEER_A.to_string()]);
    fx.relay().fail_anchor_fetch = true;
    fx.agent.refresh_anchors(&[fx.pubkey]).await;
    assert_eq!(fx.kubo().peering, BTreeSet::from([PEER_A.to_string()]));
    assert_eq!(
        fx.agent.anchors.status.managed(),
        BTreeSet::from([PEER_A.to_string()])
    );
}
