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

    relay.client.shutdown().await;
}
