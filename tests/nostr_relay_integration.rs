use nostr_sdk::prelude::*;
use swing::nostr;

fn relay_url() -> String {
    std::env::var("SWING_TEST_RELAY").unwrap_or_else(|_| "ws://127.0.0.1:18080".to_string())
}

// Requires a local Nostr relay (see docs/architecture.md); run manually with:
//   cargo test --test nostr_relay_integration -- --ignored --test-threads=1
#[tokio::test]
#[ignore]
async fn publish_and_fetch_site_event_round_trip() {
    let keys = Keys::generate();
    let secret = keys.secret_key().to_secret_hex();
    let relay = nostr::RelayClient::connect(&secret, &[relay_url()])
        .await
        .expect("connect");

    let event = nostr::build_site_event_builder(
        35980,
        "roundtrip.example",
        "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
        Some("https://roundtrip.example/"),
        Some(4242),
        Some("Roundtrip site"),
        Some("Add a roundtrip page"),
    )
    .finalize(&relay.keys)
    .unwrap();

    let out = relay.publish_to_relays(&event).await.expect("publish");
    assert!(!out.success.is_empty(), "relay did not ack the event");

    let fetched = relay
        .fetch_site_events(35980, &[keys.public_key()])
        .await
        .expect("fetch_site_events");
    assert!(
        !fetched.is_empty(),
        "did not fetch the just-published event back"
    );

    let parsed = nostr::parse_site_event(&fetched[0], 35980).unwrap();
    assert_eq!(parsed.d, "roundtrip.example");
    assert_eq!(parsed.size, Some(4242));
    assert_eq!(parsed.title.as_deref(), Some("Roundtrip site"));
    assert_eq!(parsed.message.as_deref(), Some("Add a roundtrip page"));

    relay.client.shutdown().await;
}

#[tokio::test]
#[ignore]
async fn replica_reports_are_found_by_site_and_replaced_by_withdrawals() {
    use nostr::ReportRelay;
    use std::collections::BTreeSet;

    let author = Keys::generate();
    let reporter = nostr::RelayClient::connect(
        &Keys::generate().secret_key().to_secret_hex(),
        &[relay_url()],
    )
    .await
    .expect("connect reporter");
    let other = nostr::RelayClient::connect(
        &Keys::generate().secret_key().to_secret_hex(),
        &[relay_url()],
    )
    .await
    .expect("connect other");
    let cid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string();
    let now = Timestamp::now().as_secs();
    let build = |d: &str, cids: BTreeSet<String>, created_at: u64| {
        nostr::build_replica_report_builder(
            35981,
            35980,
            &author.public_key(),
            d,
            &cids,
            Timestamp::from_secs(now + 3600),
        )
        .custom_created_at(Timestamp::from_secs(created_at))
    };

    assert!(
        reporter
            .send_report(build(
                "replica.example",
                BTreeSet::from([cid.clone()]),
                now - 10
            ))
            .await
            .unwrap()
    );
    assert!(
        reporter
            .send_report(build(
                "unrelated.example",
                BTreeSet::from([cid.clone()]),
                now - 10
            ))
            .await
            .unwrap()
    );
    assert!(
        other
            .send_report(build(
                "replica.example",
                BTreeSet::from([cid.clone()]),
                now - 10
            ))
            .await
            .unwrap()
    );
    assert!(
        other
            .send_report(build("replica.example", BTreeSet::new(), now))
            .await
            .unwrap()
    );

    let own = reporter.fetch_own_reports(35981).await.unwrap();
    assert_eq!(own.len(), 2);

    let coordinate = nostr::site_coordinate(35980, &author.public_key(), "replica.example");
    let events = reporter
        .fetch_replica_reports(35981, &[coordinate])
        .await
        .unwrap();
    let mut reports: Vec<nostr::ReplicaReport> = nostr::newest_by_address(events)
        .iter()
        .map(|e| nostr::parse_replica_report(e, 35981, 35980).unwrap())
        .collect();
    reports.sort_by_key(|r| r.cids.is_empty());
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].reporter, reporter.keys.public_key());
    assert_eq!(reports[0].d, "replica.example");
    assert_eq!(reports[0].cids, BTreeSet::from([cid.clone()]));
    assert_eq!(reports[1].reporter, other.keys.public_key());
    assert!(reports[1].cids.is_empty());

    let follow_set = EventBuilder::new(Kind::Custom(30000), "")
        .tag(Tag::identifier("swing"))
        .tag(Tag::public_key(author.public_key()))
        .finalize(&reporter.keys)
        .unwrap();
    reporter.publish_to_relays(&follow_set).await.unwrap();
    let follow_sets = reporter
        .fetch_follow_sets(
            "swing",
            &[reporter.keys.public_key(), other.keys.public_key()],
        )
        .await
        .unwrap();
    assert_eq!(follow_sets.len(), 1);
    assert_eq!(follow_sets[&reporter.keys.public_key()].id, follow_set.id);

    reporter.client.shutdown().await;
    other.client.shutdown().await;
}

async fn publish(relay: &nostr::RelayClient, builder: EventBuilder) {
    let event = builder.finalize(&relay.keys).unwrap();
    let out = relay.publish_to_relays(&event).await.unwrap();
    assert!(!out.success.is_empty(), "relay did not ack the event");
}

#[tokio::test]
#[ignore]
async fn follow_set_authors_are_found_by_referenced_account() {
    let target = Keys::generate().public_key();
    let connect = || async {
        nostr::RelayClient::connect(
            &Keys::generate().secret_key().to_secret_hex(),
            &[relay_url()],
        )
        .await
        .expect("connect")
    };
    let (follower, former, other_set) = (connect().await, connect().await, connect().await);
    let now = Timestamp::now().as_secs();
    let follow_set = |d: &str, pks: &[PublicKey], created_at: u64| {
        EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier(d))
            .tags(pks.iter().map(|pk| Tag::public_key(*pk)))
            .custom_created_at(Timestamp::from_secs(created_at))
    };
    publish(&follower, follow_set("swing", &[target], now)).await;
    publish(&former, follow_set("swing", &[target], now - 10)).await;
    publish(&former, follow_set("swing", &[], now)).await;
    publish(&other_set, follow_set("other", &[target], now)).await;

    let authors = follower
        .fetch_follow_set_authors_referencing("swing", &[target])
        .await
        .unwrap();
    assert_eq!(
        authors,
        std::collections::HashSet::from([follower.keys.public_key()])
    );

    for relay in [follower, former, other_set] {
        relay.client.shutdown().await;
    }
}
