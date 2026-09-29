use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use nostr_sdk::prelude::*;

use super::{Channel, MAX_SIGNER_EVENT_BYTES, NO_ANSWER_IN_TIME, RemoteSigner, RemoteSignerFile};
use crate::config::NostrConfig;

pub const PAIRING_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const PUBLIC_KEY_TIMEOUT: Duration = Duration::from_secs(60);
pub const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const APP_NAME: &str = "SWING";
pub const MAX_PAIRING_RELAYS: usize = 5;
const SECRET_BYTES: usize = 16;

pub fn requested_perms(kinds: &[u16]) -> String {
    std::iter::once("get_public_key".to_string())
        .chain(kinds.iter().map(|k| format!("sign_event:{k}")))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn parse_pairing_relays(relays: &[String]) -> Result<Vec<RelayUrl>> {
    let relays: Vec<&str> = relays
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    if relays.is_empty() {
        bail!("relays must include at least one entry");
    }
    if relays.len() > MAX_PAIRING_RELAYS {
        bail!("relays must include at most {MAX_PAIRING_RELAYS} entries");
    }
    relays
        .into_iter()
        .map(|r| RelayUrl::parse(r).map_err(|e| anyhow!("invalid relay {r}: {e}")))
        .collect()
}

pub fn nostrconnect_uri(
    app: &PublicKey,
    relays: &[RelayUrl],
    secret: &str,
    perms: &str,
) -> Result<String> {
    let mut url = reqwest::Url::parse(&format!("nostrconnect://{}", app.to_hex()))
        .context("building the nostrconnect URI")?;
    {
        let mut query = url.query_pairs_mut();
        for relay in relays {
            query.append_pair("relay", relay.as_str_without_trailing_slash());
        }
        query.append_pair("secret", secret);
        query.append_pair("perms", perms);
        query.append_pair("name", APP_NAME);
        // Older signers, including rust-nostr's own parser, only read the name from here.
        query.append_pair("metadata", &NostrConnectMetadata::new(APP_NAME).as_json());
    }
    Ok(url.to_string())
}

pub fn qr_svg(text: &str) -> Result<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).context("encoding the QR code")?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(256, 256)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .quiet_zone(true)
        .build())
}

// Light modules are drawn as blocks, so the code reads correctly on the usual dark terminal background.
pub fn qr_text(text: &str) -> Result<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::new(text.as_bytes()).context("encoding the QR code")?;
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingState {
    Waiting,
    Checking { user: PublicKey },
    Ready(Box<PairedSigner>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedSigner {
    pub file: RemoteSignerFile,
    pub user: PublicKey,
    pub probe_signed: bool,
    pub probe_error: Option<String>,
}

pub struct PairingRequest {
    pub relays: Vec<RelayUrl>,
    pub perms: String,
    pub probe_kind: u16,
    pub pairing_timeout: Duration,
    pub relay_timeout: Duration,
    pub probe_timeout: Duration,
}

impl PairingRequest {
    pub fn for_config(nostr: &NostrConfig, relays: Vec<RelayUrl>) -> Self {
        Self {
            relays,
            perms: requested_perms(&[
                nostr.replica_event_kind,
                nostr.site_event_kind,
                crate::nostr::FOLLOW_SET_KIND,
            ]),
            probe_kind: nostr.replica_event_kind,
            pairing_timeout: PAIRING_TIMEOUT,
            relay_timeout: RELAY_CONNECT_TIMEOUT,
            probe_timeout: PROBE_TIMEOUT,
        }
    }
}

pub struct Pairing {
    uri: String,
    state: Arc<Mutex<PairingState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Pairing {
    pub fn start(request: PairingRequest) -> Result<Self> {
        if request.relays.is_empty() {
            bail!("at least one relay is needed to reach the signer app");
        }
        let app_keys = Keys::generate();
        let secret = crate::auth::random_hex(SECRET_BYTES);
        let uri = nostrconnect_uri(
            &app_keys.public_key(),
            &request.relays,
            &secret,
            &request.perms,
        )?;
        let state = Arc::new(Mutex::new(PairingState::Waiting));
        let task = tokio::spawn(run_pairing(app_keys, secret, request, Arc::clone(&state)));
        Ok(Self { uri, state, task })
    }

    #[cfg(test)]
    pub fn finished(state: PairingState) -> Self {
        Self {
            uri: String::new(),
            state: Arc::new(Mutex::new(state)),
            task: tokio::spawn(async {}),
        }
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn state(&self) -> PairingState {
        self.state.lock().expect("pairing state lock").clone()
    }
}

impl Drop for Pairing {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_pairing(
    app_keys: Keys,
    secret: String,
    request: PairingRequest,
    state: Arc<Mutex<PairingState>>,
) {
    let outcome = pair(app_keys, secret, &request, &state).await;
    let next = match outcome {
        Ok(paired) => PairingState::Ready(Box::new(paired)),
        Err(e) => PairingState::Failed(format!("{e:#}")),
    };
    *state.lock().expect("pairing state lock") = next;
}

async fn pair(
    app_keys: Keys,
    secret: String,
    request: &PairingRequest,
    state: &Mutex<PairingState>,
) -> Result<PairedSigner> {
    let relays: Vec<String> = request.relays.iter().map(|r| r.to_string()).collect();
    let client = crate::nostr::bounded_client(MAX_SIGNER_EVENT_BYTES);
    let notifications = client.notifications();
    let connected = async {
        super::listen(&client, &relays, app_keys.public_key()).await?;
        let signer = tokio::time::timeout(
            request.pairing_timeout,
            await_connect(&app_keys, &secret, notifications),
        )
        .await
        .map_err(|_| anyhow!(NO_ANSWER_IN_TIME))??;
        let channel = Channel::listening(
            client.clone(),
            app_keys.clone(),
            signer,
            relays.clone(),
            PUBLIC_KEY_TIMEOUT,
        );
        let user = channel
            .request(NostrConnectRequest::GetPublicKey)
            .await?
            .to_get_public_key()
            .context("the signer app answered with something other than a public key")?;
        Ok((user, signer))
    };
    let unreachable = async {
        tokio::time::sleep(request.relay_timeout).await;
        let relays = client.relays().await;
        if relays
            .values()
            .any(|r| r.status() == RelayStatus::Connected)
        {
            std::future::pending().await
        } else {
            let relays: Vec<&str> = request.relays.iter().map(|r| r.as_str()).collect();
            Err(anyhow!(
                "could not connect to the relay ({})",
                relays.join(", ")
            ))
        }
    };
    let connected = tokio::select! {
        result = connected => result,
        result = unreachable => result,
    };
    client.shutdown().await;
    let (user, signer) = connected?;
    *state.lock().expect("pairing state lock") = PairingState::Checking { user };

    let file = RemoteSignerFile {
        app_secret_key: app_keys.secret_key().to_secret_hex().into(),
        signer_pubkey: signer.to_hex(),
        relays,
        user_pubkey: user.to_hex(),
    };
    let probe_error = probe(&file, request).await.err().map(|e| format!("{e:#}"));
    Ok(PairedSigner {
        file,
        user,
        probe_signed: probe_error.is_none(),
        probe_error,
    })
}

// nostr-connect also accepts a bare "ack", which anyone who sees the app key in a relay subscription could send first.
async fn await_connect(
    app_keys: &Keys,
    secret: &str,
    mut notifications: impl futures_util::Stream<Item = ClientNotification> + Unpin,
) -> Result<PublicKey> {
    while let Some(notification) = notifications.next().await {
        let ClientNotification::Event { event, .. } = notification else {
            continue;
        };
        if is_connect_with_secret(app_keys, secret, &event) {
            return Ok(event.pubkey);
        }
    }
    bail!("the connection to the relay closed")
}

fn is_connect_with_secret(app_keys: &Keys, secret: &str, event: &Event) -> bool {
    if event.kind != Kind::NostrConnect {
        return false;
    }
    let Ok(text) = nip44::decrypt(app_keys.secret_key(), &event.pubkey, &event.content) else {
        return false;
    };
    let Ok(message) = NostrConnectMessage::from_json(text) else {
        return false;
    };
    if !message.is_response() {
        return false;
    }
    match message.to_response(NostrConnectMethod::Connect) {
        Ok(NostrConnectResponse {
            result: Some(ResponseResult::ConnectSecret(echoed)),
            error: None,
        }) => echoed == secret,
        Ok(NostrConnectResponse {
            result: Some(ResponseResult::Ack),
            ..
        }) => {
            tracing::warn!(
                signer = %event.pubkey,
                "ignoring a pairing answer that does not echo the secret"
            );
            false
        }
        _ => false,
    }
}

// A fresh connection, so this also exercises how the agent reconnects after a restart.
async fn probe(file: &RemoteSignerFile, request: &PairingRequest) -> Result<()> {
    let remote = RemoteSigner::from_file(file, request.probe_timeout)?;
    let builder = EventBuilder::new(Kind::Custom(request.probe_kind), "")
        .tag(Tag::custom("alt", ["SWING signer check".to_string()]));
    let result = remote.sign(builder).await.map(|_| ());
    remote.shutdown().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::Signer;
    use crate::signer::tests::{answer_as, config_in};

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
        let uri =
            nostrconnect_uri(&app, &relays, "s3cret", "get_public_key,sign_event:35981").unwrap();
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
        assert!(
            matches!(reparsed, NostrConnectUri::Client { public_key, .. } if public_key == app)
        );
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

    async fn wait_until_paired(pairing: &Pairing) -> PairedSigner {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            match pairing.state() {
                PairingState::Ready(paired) => return *paired,
                PairingState::Failed(e) => panic!("pairing failed: {e}"),
                _ if tokio::time::Instant::now() > deadline => panic!("pairing timed out"),
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
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
                NostrConnectEventBuilder::new(event.pubkey, answer)
                    .finalize(&signer)
                    .unwrap()
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match pairing.state() {
                PairingState::Failed(e) => {
                    assert!(e.contains("could not connect to the relay"), "{e}");
                    assert!(e.contains("ws://127.0.0.1:1"), "{e}");
                    return;
                }
                PairingState::Waiting if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                other => panic!("unexpected state: {other:?}"),
            }
        }
    }
}
