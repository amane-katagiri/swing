use nostr_sdk::prelude::*;
use swing::nostr;
use swing::signer::Signer;

fn relay_url() -> String {
    std::env::var("SWING_TEST_RELAY").unwrap_or_else(|_| "ws://127.0.0.1:18080".to_string())
}

async fn connect() -> nostr::RelayClient {
    nostr::RelayClient::connect(Signer::Local(Keys::generate()), &[relay_url()])
        .await
        .expect("connect")
}

#[tokio::test]
#[ignore]
async fn publish_and_fetch_site_event_round_trip() {
    let relay = connect().await;

    let event = relay
        .sign(nostr::build_site_event_builder(
            35980,
            &nostr::SiteFields {
                d: "roundtrip.example",
                cid: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
                url: Some("https://roundtrip.example/"),
                size: Some(4242),
                title: Some("Roundtrip site"),
                message: Some("Add a roundtrip page"),
            },
        ))
        .await
        .unwrap();

    let out = relay.publish_to_relays(&event).await.expect("publish");
    assert!(!out.success.is_empty(), "relay did not ack the event");

    let fetched = relay
        .fetch_site_events(35980, &[relay.public_key()])
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
    let reporter = connect().await;
    let other = connect().await;
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
    let mut reports: Vec<nostr::ReplicaReport> = nostr::newest_by_address(events, now)
        .iter()
        .map(|e| nostr::parse_replica_report(e, 35981, 35980).unwrap())
        .collect();
    reports.sort_by_key(|r| r.cids.is_empty());
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].reporter, reporter.public_key());
    assert_eq!(reports[0].d, "replica.example");
    assert_eq!(reports[0].cids, BTreeSet::from([cid.clone()]));
    assert_eq!(reports[1].reporter, other.public_key());
    assert!(reports[1].cids.is_empty());

    let follow_set = reporter
        .sign(
            EventBuilder::new(Kind::Custom(30000), "")
                .tag(Tag::identifier("swing"))
                .tag(Tag::public_key(author.public_key())),
        )
        .await
        .unwrap();
    reporter.publish_to_relays(&follow_set).await.unwrap();
    let follow_sets = reporter
        .fetch_follow_sets("swing", &[reporter.public_key(), other.public_key()])
        .await
        .unwrap();
    assert_eq!(follow_sets.len(), 1);
    assert_eq!(follow_sets[&reporter.public_key()].id, follow_set.id);

    reporter.client.shutdown().await;
    other.client.shutdown().await;
}

async fn publish(relay: &nostr::RelayClient, builder: EventBuilder) {
    let event = relay.sign(builder).await.unwrap();
    let out = relay.publish_to_relays(&event).await.unwrap();
    assert!(!out.success.is_empty(), "relay did not ack the event");
}

#[tokio::test]
#[ignore]
async fn follow_set_authors_are_found_by_referenced_account() {
    let target = Keys::generate().public_key();
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
        std::collections::HashSet::from([follower.public_key()])
    );

    for relay in [follower, former, other_set] {
        relay.client.shutdown().await;
    }
}
