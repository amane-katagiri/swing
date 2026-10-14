use super::*;
use crate::test_support::{random_pubkey, remote_signer_file};

#[test]
fn answer_filter_pins_the_author_only_once_the_signer_is_known() {
    let app = Keys::generate().public_key();
    let signer = Keys::generate().public_key();
    assert_eq!(answer_filter(app, None).authors, None);
    assert_eq!(
        answer_filter(app, Some(signer)).authors,
        Some(std::collections::BTreeSet::from([signer]))
    );
}

#[test]
fn remote_signer_file_round_trips_privately_and_hides_the_app_key() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("nested").join("state");
    assert!(RemoteSignerFile::load(&state_dir).unwrap().is_none());
    let app = Keys::generate();
    let file = remote_signer_file(
        &app,
        random_pubkey(),
        "wss://relay.example",
        random_pubkey(),
    );
    file.save(&state_dir).unwrap();
    assert_eq!(
        RemoteSignerFile::load(&state_dir).unwrap(),
        Some(file.clone())
    );
    assert!(!format!("{file:?}").contains(file.app_secret_key.as_str()));
    #[cfg(unix)]
    {
        assert_eq!(crate::test_support::mode(&state_dir), 0o700);
        assert_eq!(
            crate::test_support::mode(&remote_signer_path(&state_dir)),
            0o600
        );
    }
}

pub(super) fn config_in(dir: &Path, secret_key: Option<&str>) -> Config {
    let toml = format!(
        "[nostr]\n{}[agent]\nstate_dir = {:?}\n",
        secret_key.map_or(String::new(), |k| format!("secret_key = {k:?}\n")),
        dir.display().to_string()
    );
    crate::config::build_config_from_str(&toml, |_| None).unwrap()
}

fn saved_file() -> RemoteSignerFile {
    remote_signer_file(
        &Keys::generate(),
        random_pubkey(),
        "ws://127.0.0.1:1",
        random_pubkey(),
    )
}

#[test]
fn missing_key_and_signer_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path(), None);
    assert!(Signer::load(&config).unwrap().is_none());
    let err = Signer::require(&config).unwrap_err();
    assert!(err.to_string().contains("pair a signer app"));
}

#[tokio::test]
async fn loads_the_local_key_or_the_paired_signer_but_not_both() {
    let dir = tempfile::tempdir().unwrap();
    let keys = Keys::generate();
    let local = config_in(dir.path(), Some(&keys.secret_key().to_secret_hex()));
    let signer = Signer::require(&local).unwrap();
    assert!(!signer.is_remote());
    assert_eq!(signer.public_key(), keys.public_key());

    let file = saved_file();
    file.save(dir.path()).unwrap();
    let err = Signer::load(&local).unwrap_err();
    assert!(err.to_string().contains("remove one"));

    let remote = config_in(dir.path(), None);
    let signer = Signer::require(&remote).unwrap();
    assert!(signer.is_remote());
    assert_eq!(signer.public_key(), file.user_public_key().unwrap());
}

#[test]
fn signed_event_must_match_the_request() {
    let user = Keys::generate();
    let unsigned = EventBuilder::new(Kind::Custom(35981), "").finalize_unsigned(user.public_key());
    let id = unsigned.compute_id();
    let good = unsigned.clone().finalize(&user).unwrap();
    check_signed(&good, user.public_key(), id).unwrap();

    let other = Keys::generate();
    let foreign = EventBuilder::new(Kind::Custom(35981), "")
        .finalize(&other)
        .unwrap();
    assert!(check_signed(&foreign, user.public_key(), id).is_err());

    let altered = EventBuilder::new(Kind::Custom(35981), "changed")
        .finalize(&user)
        .unwrap();
    assert!(check_signed(&altered, user.public_key(), id).is_err());
}

#[tokio::test]
async fn answers_from_a_signer_whose_clock_runs_behind_are_received() {
    let relay = LocalRelay::new();
    relay.run().await.unwrap();
    let url = relay.url().await;
    let user = Keys::generate();
    let signer_keys = Keys::generate();
    let app = Keys::generate();

    let (signer_client, answering) = {
        let (user, signer) = (user.clone(), signer_keys.clone());
        answer_as(url.clone(), &signer_keys, move |event, message| {
            let id = message.id().to_string();
            let NostrConnectRequest::SignEvent(unsigned) = message.to_request().unwrap() else {
                panic!("only sign_event is expected");
            };
            let signed = unsigned.finalize(&user).unwrap();
            let answer = NostrConnectMessage::response(
                id,
                NostrConnectResponse::with_result(ResponseResult::SignEvent(Box::new(signed))),
            );
            let behind = Timestamp::from_secs(Timestamp::now().as_secs() - 600);
            let content = nip44::encrypt(
                signer.secret_key(),
                &event.pubkey,
                answer.as_json(),
                nip44::Version::default(),
            )
            .unwrap();
            Some(
                EventBuilder::new(Kind::NostrConnect, content)
                    .tag(Tag::public_key(event.pubkey))
                    .custom_created_at(behind)
                    .finalize(&signer)
                    .unwrap(),
            )
        })
        .await
    };

    let file = remote_signer_file(
        &app,
        signer_keys.public_key(),
        url.as_str(),
        user.public_key(),
    );
    let remote = RemoteSigner::from_file(&file, Duration::from_secs(5)).unwrap();
    let event = remote
        .sign(EventBuilder::new(Kind::Custom(35981), ""))
        .await
        .unwrap();
    assert_eq!(event.pubkey, user.public_key());
    answering.abort();
    remote.shutdown().await;
    signer_client.shutdown().await;
}

pub(super) fn refusal(signer: &Keys, to: PublicKey, id: &str, error: &str) -> Event {
    let message = NostrConnectMessage::Response {
        id: id.to_string(),
        result: Some(String::new()),
        error: Some(error.to_string()),
    };
    NostrConnectEventBuilder::new(to, message)
        .finalize(signer)
        .unwrap()
}

fn answer(result: Option<&str>, error: Option<&str>) -> NostrConnectMessage {
    NostrConnectMessage::Response {
        id: "1".to_string(),
        result: result.map(str::to_string),
        error: error.map(str::to_string),
    }
}

#[test]
fn the_signers_error_is_read_before_the_empty_result() {
    for method in [
        NostrConnectMethod::GetPublicKey,
        NostrConnectMethod::SignEvent,
    ] {
        let err = read_response(method, answer(Some(""), Some("user rejected"))).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the signer app refused the request: user rejected"
        );
    }
    let err = read_response(
        NostrConnectMethod::SignEvent,
        answer(None, Some("no\u{1b}[31m\nway")),
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "the signer app refused the request: no [31m way"
    );
    let long = "x".repeat(MAX_SIGNER_ERROR_CHARS + 10);
    let err = read_response(NostrConnectMethod::SignEvent, answer(Some(""), Some(&long)))
        .unwrap_err()
        .to_string();
    assert!(err.ends_with('\u{2026}'), "{err}");
    assert!(err.chars().count() < MAX_SIGNER_ERROR_CHARS + 50);
}

#[test]
fn an_answer_without_an_error_is_read_as_the_result() {
    let user = Keys::generate().public_key();
    let result = read_response(
        NostrConnectMethod::GetPublicKey,
        answer(Some(&user.to_hex()), Some("")),
    )
    .unwrap();
    assert_eq!(result.to_get_public_key().unwrap(), user);
    let err = read_response(NostrConnectMethod::GetPublicKey, answer(Some(""), None)).unwrap_err();
    assert!(err.to_string().contains("without a result"), "{err}");
    let err = read_response(
        NostrConnectMethod::GetPublicKey,
        answer(Some("error"), None),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "the signer app refused the request");
    let err = read_response(
        NostrConnectMethod::SignEvent,
        answer(Some("auth_url"), Some("https://signer.example/approve")),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("approve the request at https://signer.example/approve"),
        "{err}"
    );
}

#[tokio::test]
async fn a_refused_signature_reports_the_signers_reason() {
    let relay = LocalRelay::new();
    relay.run().await.unwrap();
    let url = relay.url().await;
    let signer_keys = Keys::generate();
    let (signer_client, answering) = {
        let signer = signer_keys.clone();
        answer_as(url.clone(), &signer_keys, move |event, message| {
            Some(refusal(
                &signer,
                event.pubkey,
                message.id(),
                "user rejected",
            ))
        })
        .await
    };
    let file = remote_signer_file(
        &Keys::generate(),
        signer_keys.public_key(),
        url.as_str(),
        random_pubkey(),
    );
    let remote = RemoteSigner::from_file(&file, Duration::from_secs(5)).unwrap();
    let err = remote
        .sign(EventBuilder::new(Kind::Custom(35981), ""))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "the signer app refused the request: user rejected"
    );
    assert_eq!(
        remote.last_failure().unwrap().message,
        "the signer app refused the request: user rejected"
    );
    answering.abort();
    remote.shutdown().await;
    signer_client.shutdown().await;
}

pub(super) async fn answer_as<F>(
    url: RelayUrl,
    keys: &Keys,
    answer: F,
) -> (Client, tokio::task::JoinHandle<()>)
where
    F: Fn(&Event, NostrConnectMessage) -> Option<Event> + Send + 'static,
{
    let client = Client::new();
    client.add_relay(url).await.unwrap();
    client.connect().and_wait(Duration::from_secs(5)).await;
    client
        .subscribe(
            Filter::new()
                .kind(Kind::NostrConnect)
                .pubkey(keys.public_key())
                .limit(0),
        )
        .await
        .unwrap();
    let mut notifications = client.notifications();
    let answering = {
        let (keys, client) = (keys.clone(), client.clone());
        tokio::spawn(async move {
            while let Some(notification) = notifications.next().await {
                let ClientNotification::Event { event, .. } = notification else {
                    continue;
                };
                let text =
                    nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content).unwrap();
                let message = NostrConnectMessage::from_json(text).unwrap();
                if let Some(reply) = answer(&event, message) {
                    client.send_event(&reply).await.unwrap();
                }
            }
        })
    };
    (client, answering)
}
