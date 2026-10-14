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
                Kind::TextNote.as_u16(),
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
        super::listen(&client, &relays, app_keys.public_key(), None).await?;
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
mod tests;
