use super::*;
use crate::signer::Signer;
use crate::signer::tests::{answer_as, config_in, refusal};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn perms_request_the_public_key_and_each_kind() {
    assert_eq!(
        requested_perms(&[35981, 35980, 30000]),
        "get_public_key,sign_event:35981,sign_event:35980,sign_event:30000"
    );
}

#[test]
fn nostrconnect_uri_carries_relays_secret_perms_and_name() {
    let app = Keys::generate().public_key();
    let relays = vec![RelayUrl::parse("wss://relay.example/").unwrap()];
    let uri = nostrconnect_uri(&app, &relays, "s3cret", "get_public_key,sign_event:35981").unwrap();
    let parsed = reqwest::Url::parse(&uri).unwrap();
    assert_eq!(parsed.scheme(), "nostrconnect");
    assert_eq!(parsed.host_str(), Some(app.to_hex().as_str()));
    let pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
    assert_eq!(
        pairs,
        vec![
            ("relay".to_string(), "wss://relay.example".to_string()),
            ("secret".to_string(), "s3cret".to_string()),
            (
                "perms".to_string(),
                "get_public_key,sign_event:35981".to_string()
            ),
            ("name".to_string(), "SWING".to_string()),
            ("metadata".to_string(), r#"{"name":"SWING"}"#.to_string()),
        ]
    );
    let reparsed = NostrConnectUri::parse(&uri).unwrap();
    assert!(matches!(reparsed, NostrConnectUri::Client { public_key, .. } if public_key == app));
}

#[test]
fn qr_svg_is_an_svg() {
    let svg = qr_svg("nostrconnect://abc").unwrap();
    assert!(svg.contains("<svg"));
}

#[test]
fn qr_text_is_a_block_of_equal_width_lines() {
    let text = qr_text("nostrconnect://abc").unwrap();
    let widths: Vec<usize> = text.lines().map(|l| l.chars().count()).collect();
    assert!(widths.len() > 10);
    assert!(widths.iter().all(|w| *w == widths[0]));
}

#[test]
fn pairing_relays_are_trimmed_and_validated() {
    let relays =
        parse_pairing_relays(&[" wss://relay.example ".to_string(), String::new()]).unwrap();
    assert_eq!(
        relays,
        vec![RelayUrl::parse("wss://relay.example").unwrap()]
    );
    assert!(parse_pairing_relays(&[]).is_err());
    assert!(parse_pairing_relays(&["https://relay.example".to_string()]).is_err());
    let many: Vec<String> = (0..6).map(|i| format!("wss://r{i}.example")).collect();
    assert!(parse_pairing_relays(&many).is_err());
}

fn request_via(relay: RelayUrl) -> PairingRequest {
    PairingRequest {
        relays: vec![relay],
        perms: requested_perms(&[35981]),
        probe_kind: 35981,
        pairing_timeout: Duration::from_secs(10),
        relay_timeout: Duration::from_secs(5),
        probe_timeout: Duration::from_secs(3),
    }
}

async fn wait_past(
    pairing: &Pairing,
    within: Duration,
    pending: impl Fn(&PairingState) -> bool,
) -> PairingState {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let state = pairing.state();
        if !pending(&state) {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "pairing timed out: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_until_paired(pairing: &Pairing) -> PairedSigner {
    let pending =
        |s: &PairingState| matches!(s, PairingState::Waiting | PairingState::Checking { .. });
    match wait_past(pairing, Duration::from_secs(20), pending).await {
        PairingState::Ready(paired) => *paired,
        other => panic!("pairing failed: {other:?}"),
    }
}

async fn wait_for_failure(pairing: &Pairing) -> String {
    let pending = |s: &PairingState| matches!(s, PairingState::Waiting);
    match wait_past(pairing, Duration::from_secs(10), pending).await {
        PairingState::Failed(e) => e,
        other => panic!("unexpected state: {other:?}"),
    }
}

async fn pair_with(user: &Keys, sign: bool) -> (LocalRelay, PairedSigner) {
    let relay = LocalRelay::new();
    relay.run().await.unwrap();
    let pairing = Pairing::start(request_via(relay.url().await)).unwrap();
    crate::test_support::serve_test_signer(pairing.uri(), user, sign);
    let paired = wait_until_paired(&pairing).await;
    (relay, paired)
}

#[tokio::test]
async fn pairs_through_a_qr_code_and_signs_after_reconnecting() {
    let user = Keys::generate();
    let (_relay, paired) = pair_with(&user, true).await;
    assert_eq!(paired.user, user.public_key());
    assert!(
        paired.probe_signed,
        "probe failed: {:?}",
        paired.probe_error
    );

    let dir = tempfile::tempdir().unwrap();
    paired.file.save(dir.path()).unwrap();
    let signer = Signer::require(&config_in(dir.path(), None)).unwrap();
    assert_eq!(signer.public_key(), user.public_key());
    let event = signer
        .sign(EventBuilder::new(Kind::Custom(35980), "hello"))
        .await
        .unwrap();
    assert_eq!(event.pubkey, user.public_key());
    assert_eq!(event.content, "hello");
    event.verify().unwrap();
    assert_eq!(signer.last_failure(), None);
    signer.shutdown().await;
}

#[tokio::test]
async fn a_refused_probe_still_pairs_but_reports_no_automatic_signing() {
    let user = Keys::generate();
    let (_relay, paired) = pair_with(&user, false).await;
    assert_eq!(paired.user, user.public_key());
    assert!(!paired.probe_signed);
    assert!(
        paired.probe_error.as_deref().unwrap().contains("refused"),
        "{:?}",
        paired.probe_error
    );

    let remote = RemoteSigner::from_file(&paired.file, Duration::from_secs(3)).unwrap();
    assert!(
        remote
            .sign(EventBuilder::new(Kind::Custom(35981), ""))
            .await
            .is_err()
    );
    assert!(remote.last_failure().unwrap().message.contains("refused"));
    remote.shutdown().await;
}

fn connect_answer(from: &Keys, app: &PublicKey, result: ResponseResult) -> Event {
    let message = NostrConnectMessage::response("x", NostrConnectResponse::with_result(result));
    NostrConnectEventBuilder::new(*app, message)
        .finalize(from)
        .unwrap()
}

#[test]
fn only_a_connect_answer_echoing_the_secret_identifies_the_signer() {
    let app = Keys::generate();
    let signer = Keys::generate();
    let secret = "s3cret";
    let echoed = connect_answer(
        &signer,
        &app.public_key(),
        ResponseResult::ConnectSecret(secret.to_string()),
    );
    assert!(is_connect_with_secret(&app, secret, &echoed));
    let ack = connect_answer(&signer, &app.public_key(), ResponseResult::Ack);
    assert!(!is_connect_with_secret(&app, secret, &ack));
    let wrong = connect_answer(
        &signer,
        &app.public_key(),
        ResponseResult::ConnectSecret("guess".to_string()),
    );
    assert!(!is_connect_with_secret(&app, secret, &wrong));
}

#[tokio::test]
async fn an_ack_racing_the_signer_does_not_take_over_the_pairing() {
    let relay = LocalRelay::new();
    relay.run().await.unwrap();
    let pairing = Pairing::start(request_via(relay.url().await)).unwrap();
    let NostrConnectUri::Client {
        public_key: app, ..
    } = NostrConnectUri::parse(pairing.uri()).unwrap()
    else {
        panic!("not a nostrconnect URI");
    };

    let attacker = Keys::generate();
    let (client, answering) = {
        let signer = attacker.clone();
        answer_as(relay.url().await, &attacker, move |event, message| {
            let answer = NostrConnectMessage::response(
                message.id(),
                NostrConnectResponse::with_result(ResponseResult::GetPublicKey(
                    signer.public_key(),
                )),
            );
            Some(
                NostrConnectEventBuilder::new(event.pubkey, answer)
                    .finalize(&signer)
                    .unwrap(),
            )
        })
        .await
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    client
        .send_event(&connect_answer(&attacker, &app, ResponseResult::Ack))
        .await
        .unwrap();

    let user = Keys::generate();
    crate::test_support::serve_test_signer(pairing.uri(), &user, true);
    let paired = wait_until_paired(&pairing).await;
    assert_eq!(paired.user, user.public_key());
    assert_ne!(paired.file.signer_pubkey, attacker.public_key().to_hex());
    answering.abort();
    client.shutdown().await;
}

#[tokio::test]
async fn an_unreachable_relay_fails_the_pairing_quickly() {
    let pairing = Pairing::start(PairingRequest {
        pairing_timeout: Duration::from_secs(60),
        relay_timeout: Duration::from_millis(500),
        ..request_via(RelayUrl::parse("ws://127.0.0.1:1").unwrap())
    })
    .unwrap();
    let e = wait_for_failure(&pairing).await;
    assert!(e.contains("could not connect to the relay"), "{e}");
    assert!(e.contains("ws://127.0.0.1:1"), "{e}");
}

async fn serve_fake_signer<F>(
    relay: RelayUrl,
    pairing: &Pairing,
    signer: &Keys,
    answer: F,
) -> (Client, tokio::task::JoinHandle<()>)
where
    F: Fn(&Event, NostrConnectMessage) -> Option<Event> + Send + 'static,
{
    let NostrConnectUri::Client {
        public_key: app,
        secret,
        ..
    } = NostrConnectUri::parse(pairing.uri()).unwrap()
    else {
        panic!("not a nostrconnect URI");
    };
    let (client, answering) = answer_as(relay, signer, answer).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    client
        .send_event(&connect_answer(
            signer,
            &app,
            ResponseResult::ConnectSecret(secret),
        ))
        .await
        .unwrap();
    (client, answering)
}

#[tokio::test]
async fn a_lost_public_key_request_is_sent_again() {
    let relay = LocalRelay::new();
    relay.run().await.unwrap();
    let started = tokio::time::Instant::now();
    let pairing = Pairing::start(request_via(relay.url().await)).unwrap();
    let (user, signer_keys) = (Keys::generate(), Keys::generate());
    let asked = Arc::new(AtomicUsize::new(0));
    let (client, answering) = {
        let (user, signer, asked) = (user.public_key(), signer_keys.clone(), Arc::clone(&asked));
        serve_fake_signer(
            relay.url().await,
            &pairing,
            &signer_keys,
            move |event, message| {
                let id = message.id().to_string();
                match message.to_request().unwrap() {
                    NostrConnectRequest::GetPublicKey => {
                        if asked.fetch_add(1, Ordering::SeqCst) == 0 {
                            return None;
                        }
                        let answer = NostrConnectMessage::response(
                            id,
                            NostrConnectResponse::with_result(ResponseResult::GetPublicKey(user)),
                        );
                        Some(
                            NostrConnectEventBuilder::new(event.pubkey, answer)
                                .finalize(&signer)
                                .unwrap(),
                        )
                    }
                    _ => Some(refusal(&signer, event.pubkey, &id, "user rejected")),
                }
            },
        )
        .await
    };
    let paired = wait_until_paired(&pairing).await;
    assert_eq!(paired.user, user.public_key());
    assert_eq!(paired.file.signer_pubkey, signer_keys.public_key().to_hex());
    assert!(asked.load(Ordering::SeqCst) >= 2);
    assert!(started.elapsed() < PUBLIC_KEY_TIMEOUT / 3);
    assert!(!paired.probe_signed);
    assert_eq!(
        paired.probe_error.as_deref(),
        Some("the signer app refused the request: user rejected")
    );
    answering.abort();
    client.shutdown().await;
}

#[tokio::test]
async fn a_refused_public_key_fails_the_pairing_with_the_signers_reason() {
    let relay = LocalRelay::new();
    relay.run().await.unwrap();
    let pairing = Pairing::start(request_via(relay.url().await)).unwrap();
    let signer_keys = Keys::generate();
    let (client, answering) = {
        let signer = signer_keys.clone();
        serve_fake_signer(
            relay.url().await,
            &pairing,
            &signer_keys,
            move |event, message| {
                Some(refusal(
                    &signer,
                    event.pubkey,
                    message.id(),
                    "user rejected",
                ))
            },
        )
        .await
    };
    assert_eq!(
        wait_for_failure(&pairing).await,
        "the signer app refused the request: user rejected"
    );
    answering.abort();
    client.shutdown().await;
}
