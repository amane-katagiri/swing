use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::config::Config;

mod pair;

pub use pair::{
    APP_NAME, MAX_PAIRING_RELAYS, PAIRING_TIMEOUT, PROBE_TIMEOUT, PairedSigner, Pairing,
    PairingRequest, PairingState, RELAY_CONNECT_TIMEOUT, nostrconnect_uri, parse_pairing_relays,
    qr_svg, qr_text, requested_perms,
};

pub const REMOTE_SIGNER_FILE: &str = "remote-signer.json";
pub const SIGN_TIMEOUT: Duration = Duration::from_secs(90);
// A NIP-44 payload tops out near 87 KiB of base64, so the signer's answers need more room than site events.
const MAX_SIGNER_EVENT_BYTES: u32 = 128 * 1024;
const RESEND_INTERVAL: Duration = Duration::from_secs(5);
const MAX_SIGNER_ERROR_CHARS: usize = 500;
const NO_ANSWER_IN_TIME: &str =
    "the signer app did not answer in time; check that it is running and approve the request";

pub fn remote_signer_path(state_dir: &Path) -> PathBuf {
    state_dir.join(REMOTE_SIGNER_FILE)
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSignerFile {
    pub app_secret_key: Zeroizing<String>,
    pub signer_pubkey: String,
    pub relays: Vec<String>,
    pub user_pubkey: String,
}

impl std::fmt::Debug for RemoteSignerFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSignerFile")
            .field("signer_pubkey", &self.signer_pubkey)
            .field("relays", &self.relays)
            .field("user_pubkey", &self.user_pubkey)
            .finish_non_exhaustive()
    }
}

impl RemoteSignerFile {
    pub fn load(state_dir: &Path) -> Result<Option<Self>> {
        let path = remote_signer_path(state_dir);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => {
                #[cfg(unix)]
                crate::auth::warn_if_readable_by_others(&path, "the remote signer file");
                Zeroizing::new(raw)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let file =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(file))
    }

    pub fn save(&self, state_dir: &Path) -> Result<()> {
        crate::auth::create_private_dir_all(state_dir)?;
        let mut json = Zeroizing::new(
            serde_json::to_string_pretty(self).context("serializing the remote signer")?,
        );
        json.push('\n');
        crate::auth::write_private_file(&remote_signer_path(state_dir), &json)
    }

    pub fn user_public_key(&self) -> Result<PublicKey> {
        PublicKey::from_hex(&self.user_pubkey)
            .context("invalid user_pubkey in the remote signer file")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignFailure {
    pub at: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum Signer {
    Local(Keys),
    Remote(Arc<RemoteSigner>),
}

impl Signer {
    pub fn load(config: &Config) -> Result<Option<Self>> {
        let remote = RemoteSignerFile::load(&config.agent.state_dir)?;
        match (&config.nostr.secret_key, remote) {
            (Some(_), Some(_)) => bail!(
                "both a Nostr secret key and a paired signer app ({}) are configured; remove one of them",
                remote_signer_path(&config.agent.state_dir).display()
            ),
            (Some(secret), None) => Ok(Some(Self::Local(
                Keys::parse(secret.expose_secret()).context("parsing Nostr secret key")?,
            ))),
            (None, Some(file)) => Ok(Some(Self::Remote(Arc::new(RemoteSigner::from_file(
                &file,
                SIGN_TIMEOUT,
            )?)))),
            (None, None) => Ok(None),
        }
    }

    pub fn require(config: &Config) -> Result<Self> {
        Self::load(config)?.context(
            "missing Nostr key: set SWING_NOSTR_SECRET_KEY or [nostr].secret_key, or pair a signer app on the dashboard's setup page or with `swing signer pair`",
        )
    }

    pub fn public_key(&self) -> PublicKey {
        match self {
            Self::Local(keys) => keys.public_key(),
            Self::Remote(remote) => remote.user,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    pub fn last_failure(&self) -> Option<SignFailure> {
        match self {
            Self::Local(_) => None,
            Self::Remote(remote) => remote.last_failure(),
        }
    }

    pub fn signer_relays(&self) -> &[String] {
        match self {
            Self::Local(_) => &[],
            Self::Remote(remote) => &remote.channel.relays,
        }
    }

    pub async fn sign(&self, builder: EventBuilder) -> Result<Event> {
        match self {
            Self::Local(keys) => Ok(builder.finalize(keys)?),
            Self::Remote(remote) => remote.sign(builder).await,
        }
    }

    pub async fn shutdown(&self) {
        if let Self::Remote(remote) = self {
            remote.shutdown().await;
        }
    }
}

#[derive(Debug)]
struct Channel {
    client: Client,
    app_keys: Keys,
    signer: PublicKey,
    relays: Vec<String>,
    timeout: Duration,
    started: tokio::sync::OnceCell<()>,
}

impl Channel {
    fn new(app_keys: Keys, signer: PublicKey, relays: Vec<String>, timeout: Duration) -> Self {
        Self {
            client: crate::nostr::bounded_client(MAX_SIGNER_EVENT_BYTES),
            app_keys,
            signer,
            relays,
            timeout,
            started: tokio::sync::OnceCell::new(),
        }
    }

    fn listening(
        client: Client,
        app_keys: Keys,
        signer: PublicKey,
        relays: Vec<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            client,
            app_keys,
            signer,
            relays,
            timeout,
            started: tokio::sync::OnceCell::new_with(Some(())),
        }
    }

    async fn start(&self) -> Result<()> {
        self.started
            .get_or_try_init(|| {
                listen(
                    &self.client,
                    &self.relays,
                    self.app_keys.public_key(),
                    Some(self.signer),
                )
            })
            .await?;
        Ok(())
    }

    async fn request(&self, req: NostrConnectRequest) -> Result<ResponseResult> {
        self.start().await?;
        let method = req.method();
        let mut notifications = self.client.notifications();
        let mut sent = HashSet::from([self.send(&req).await?]);
        let resend = resends(method);
        let mut ticks = tokio::time::interval_at(
            tokio::time::Instant::now() + RESEND_INTERVAL,
            RESEND_INTERVAL,
        );
        let answer = async {
            loop {
                tokio::select! {
                    notification = notifications.next() => {
                        let Some(notification) = notification else {
                            bail!("the connection to the relay closed");
                        };
                        let Some(message) = self.response(notification) else {
                            continue;
                        };
                        if sent.contains(message.id()) {
                            return read_response(method, message);
                        }
                    }
                    _ = ticks.tick(), if resend => match self.send(&req).await {
                        Ok(id) => {
                            sent.insert(id);
                        }
                        Err(e) => tracing::debug!("resending to the signer app failed: {e:#}"),
                    },
                }
            }
        };
        tokio::time::timeout(self.timeout, answer)
            .await
            .map_err(|_| anyhow!(NO_ANSWER_IN_TIME))?
    }

    async fn send(&self, req: &NostrConnectRequest) -> Result<String> {
        let message = NostrConnectMessage::request(req);
        let id = message.id().to_string();
        let event = NostrConnectEventBuilder::new(self.signer, message)
            .finalize(&self.app_keys)
            .context("encrypting the request to the signer app")?;
        self.client
            .send_event(&event)
            .await
            .context("sending the request to the signer app")?;
        Ok(id)
    }

    fn response(&self, notification: ClientNotification) -> Option<NostrConnectMessage> {
        let ClientNotification::Event { event, .. } = notification else {
            return None;
        };
        if event.kind != Kind::NostrConnect || event.pubkey != self.signer {
            return None;
        }
        let text =
            nip44::decrypt(self.app_keys.secret_key(), &event.pubkey, &event.content).ok()?;
        NostrConnectMessage::from_json(text)
            .ok()
            .filter(NostrConnectMessage::is_response)
    }

    async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}

// Only requests without side effects: a signer may have acted on a request whose answer was lost.
fn resends(method: NostrConnectMethod) -> bool {
    matches!(
        method,
        NostrConnectMethod::GetPublicKey | NostrConnectMethod::Ping
    )
}

// The error is read first: signers such as Amber and Clave answer failures with an empty `result`.
fn read_response(
    method: NostrConnectMethod,
    message: NostrConnectMessage,
) -> Result<ResponseResult> {
    let NostrConnectMessage::Response { result, error, .. } = message else {
        bail!("the signer app sent a request instead of an answer");
    };
    let result = result.filter(|r| !r.is_empty());
    let error = error
        .map(|e| crate::format::sanitize_display_text(&e, MAX_SIGNER_ERROR_CHARS))
        .filter(|e| !e.is_empty());
    if result.as_deref() == Some("auth_url") {
        bail!(
            "the signer app asks you to approve the request at {}",
            error.unwrap_or_default()
        );
    }
    if let Some(error) = error {
        bail!("the signer app refused the request: {error}");
    }
    let Some(result) = result else {
        bail!("the signer app answered without a result");
    };
    if result == "error" {
        bail!("the signer app refused the request");
    }
    ResponseResult::parse(method, result).context("reading the signer app's answer")
}

fn answer_filter(app: PublicKey, author: Option<PublicKey>) -> Filter {
    let filter = Filter::new().kind(Kind::NostrConnect).pubkey(app).limit(0);
    match author {
        Some(author) => filter.author(author),
        None => filter,
    }
}

async fn listen(
    client: &Client,
    relays: &[String],
    app: PublicKey,
    author: Option<PublicKey>,
) -> Result<()> {
    for relay in relays {
        client
            .add_relay(relay.as_str())
            .await
            .with_context(|| format!("adding relay {relay}"))?;
    }
    client.connect().await;
    // `since` would drop answers from a signer whose clock runs behind this one.
    let filter = answer_filter(app, author);
    client
        .subscribe(filter)
        .await
        .context("subscribing to the signer app's answers")?;
    Ok(())
}

// No `connect` is sent: the pairing already told the signer about this app, and signers such as Primal refuse a second `connect` carrying the pairing secret.
#[derive(Debug)]
pub struct RemoteSigner {
    channel: Channel,
    user: PublicKey,
    last_failure: Mutex<Option<SignFailure>>,
}

impl RemoteSigner {
    pub fn from_file(file: &RemoteSignerFile, timeout: Duration) -> Result<Self> {
        let app_keys = Keys::parse(&file.app_secret_key)
            .context("invalid app_secret_key in the remote signer file")?;
        let signer = PublicKey::from_hex(&file.signer_pubkey)
            .context("invalid signer_pubkey in the remote signer file")?;
        let user = file.user_public_key()?;
        for relay in &file.relays {
            RelayUrl::parse(relay)
                .with_context(|| format!("invalid relay {relay} in the remote signer file"))?;
        }
        Ok(Self {
            channel: Channel::new(app_keys, signer, file.relays.clone(), timeout),
            user,
            last_failure: Mutex::new(None),
        })
    }

    pub fn last_failure(&self) -> Option<SignFailure> {
        self.last_failure.lock().expect("sign failure lock").clone()
    }

    pub async fn sign(&self, builder: EventBuilder) -> Result<Event> {
        let result = self.request_signature(builder).await;
        *self.last_failure.lock().expect("sign failure lock") = match &result {
            Ok(_) => None,
            Err(e) => Some(SignFailure {
                at: Timestamp::now().as_secs(),
                message: format!("{e:#}"),
            }),
        };
        result
    }

    async fn request_signature(&self, builder: EventBuilder) -> Result<Event> {
        let unsigned = builder.finalize_unsigned(self.user);
        let expected_id = unsigned.compute_id();
        let event = self
            .channel
            .request(NostrConnectRequest::SignEvent(unsigned))
            .await?
            .to_sign_event()
            .context("the signer app answered with something other than a signed event")?;
        check_signed(&event, self.user, expected_id)?;
        Ok(event)
    }

    pub async fn shutdown(&self) {
        self.channel.shutdown().await;
    }
}

fn check_signed(event: &Event, user: PublicKey, expected_id: EventId) -> Result<()> {
    if event.pubkey != user {
        bail!(
            "the signer app signed with a different key ({})",
            event.pubkey.to_hex()
        );
    }
    if event.id != expected_id {
        bail!("the signer app returned a different event than the one requested");
    }
    event
        .verify()
        .context("the signer app returned an invalid signature")
}

#[cfg(test)]
mod tests;
