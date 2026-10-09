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
mod tests {
    use super::*;

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
    fn remote_signer_file_round_trips_and_hides_the_app_key() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RemoteSignerFile::load(dir.path()).unwrap().is_none());
        let app = Keys::generate();
        let file = RemoteSignerFile {
            app_secret_key: app.secret_key().to_secret_hex().into(),
            signer_pubkey: Keys::generate().public_key().to_hex(),
            relays: vec!["wss://relay.example".to_string()],
            user_pubkey: Keys::generate().public_key().to_hex(),
        };
        file.save(dir.path()).unwrap();
        assert_eq!(
            RemoteSignerFile::load(dir.path()).unwrap(),
            Some(file.clone())
        );
        assert!(!format!("{file:?}").contains(file.app_secret_key.as_str()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(remote_signer_path(dir.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn remote_signer_file_save_creates_state_dir_as_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("nested").join("state");
        saved_file().save(&state_dir).unwrap();
        let mode = std::fs::metadata(&state_dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
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
        RemoteSignerFile {
            app_secret_key: Keys::generate().secret_key().to_secret_hex().into(),
            signer_pubkey: Keys::generate().public_key().to_hex(),
            relays: vec!["ws://127.0.0.1:1".to_string()],
            user_pubkey: Keys::generate().public_key().to_hex(),
        }
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
        let unsigned =
            EventBuilder::new(Kind::Custom(35981), "").finalize_unsigned(user.public_key());
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

        let file = RemoteSignerFile {
            app_secret_key: app.secret_key().to_secret_hex().into(),
            signer_pubkey: signer_keys.public_key().to_hex(),
            relays: vec![url.to_string()],
            user_pubkey: user.public_key().to_hex(),
        };
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
        let err =
            read_response(NostrConnectMethod::GetPublicKey, answer(Some(""), None)).unwrap_err();
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
        let file = RemoteSignerFile {
            app_secret_key: Keys::generate().secret_key().to_secret_hex().into(),
            signer_pubkey: signer_keys.public_key().to_hex(),
            relays: vec![url.to_string()],
            user_pubkey: Keys::generate().public_key().to_hex(),
        };
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
}
