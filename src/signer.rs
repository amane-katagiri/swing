use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use nostr_connect::prelude::{ErrorKind as ConnectErrorKind, NostrConnect};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;

pub const REMOTE_SIGNER_FILE: &str = "remote-signer.json";
pub const SIGN_TIMEOUT: Duration = Duration::from_secs(90);
pub const PAIRING_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
pub const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const APP_NAME: &str = "SWING";
const SECRET_BYTES: usize = 16;

pub fn remote_signer_path(state_dir: &Path) -> PathBuf {
    state_dir.join(REMOTE_SIGNER_FILE)
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSignerFile {
    pub app_secret_key: String,
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
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let file =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(file))
    }

    pub fn save(&self, state_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("creating state dir {}", state_dir.display()))?;
        let json = serde_json::to_string_pretty(self).context("serializing the remote signer")?;
        crate::auth::write_private_file(&remote_signer_path(state_dir), &format!("{json}\n"))
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
            "missing Nostr key: set SWING_NOSTR_SECRET_KEY or [nostr].secret_key, or pair a signer app on the dashboard's setup page",
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
            Self::Remote(remote) => &remote.relays,
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

// Requests go straight to the signer without a `connect`: after a nostrconnect://
// pairing the signer already knows this app, and signers such as Primal refuse a
// second `connect` carrying the pairing secret.
#[derive(Debug)]
pub struct RemoteSigner {
    client: Client,
    app_keys: Keys,
    signer: PublicKey,
    user: PublicKey,
    relays: Vec<String>,
    timeout: Duration,
    started: tokio::sync::OnceCell<()>,
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
            client: Client::new(),
            app_keys,
            signer,
            user,
            relays: file.relays.clone(),
            timeout,
            started: tokio::sync::OnceCell::new(),
            last_failure: Mutex::new(None),
        })
    }

    async fn start(&self) -> Result<()> {
        self.started
            .get_or_try_init(|| async {
                for relay in &self.relays {
                    self.client
                        .add_relay(relay.as_str())
                        .await
                        .with_context(|| format!("adding relay {relay}"))?;
                }
                self.client.connect().await;
                // `since` would drop answers from a signer whose clock runs behind this one.
                let filter = Filter::new()
                    .kind(Kind::NostrConnect)
                    .pubkey(self.app_keys.public_key())
                    .limit(0);
                self.client
                    .subscribe(filter)
                    .await
                    .context("subscribing to the signer app's answers")?;
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        Ok(())
    }

    async fn request(&self, req: NostrConnectRequest) -> Result<ResponseResult> {
        self.start().await?;
        let method = req.method();
        let message = NostrConnectMessage::request(&req);
        let id = message.id().to_string();
        let event = NostrConnectEventBuilder::new(self.signer, message)
            .finalize(&self.app_keys)
            .context("encrypting the request to the signer app")?;
        let mut notifications = self.client.notifications();
        self.client
            .send_event(&event)
            .await
            .context("sending the request to the signer app")?;
        let answer = async {
            while let Some(notification) = notifications.next().await {
                let ClientNotification::Event { event, .. } = notification else {
                    continue;
                };
                if event.kind != Kind::NostrConnect || event.pubkey != self.signer {
                    continue;
                }
                let Ok(text) =
                    nip44::decrypt(self.app_keys.secret_key(), &event.pubkey, &event.content)
                else {
                    continue;
                };
                let Ok(message) = NostrConnectMessage::from_json(text) else {
                    continue;
                };
                if message.id() != id || !message.is_response() {
                    continue;
                }
                let response = message
                    .to_response(method)
                    .context("reading the signer app's answer")?;
                if response.is_auth_url() {
                    bail!(
                        "the signer app asks you to approve the request at {}",
                        response.error.unwrap_or_default()
                    );
                }
                if let Some(error) = response.error {
                    bail!("the signer app refused the request: {error}");
                }
                return response
                    .result
                    .context("the signer app answered without a result");
            }
            bail!("the connection to the relay closed")
        };
        tokio::time::timeout(self.timeout, answer)
            .await
            .map_err(|_| {
                anyhow!(
                    "the signer app did not answer in time; check that it is running and approve the request"
                )
            })?
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
            .request(NostrConnectRequest::SignEvent(unsigned))
            .await?
            .to_sign_event()
            .context("the signer app answered with something other than a signed event")?;
        check_signed(&event, self.user, expected_id)?;
        Ok(event)
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
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

fn describe_connect_error(e: nostr_connect::prelude::Error) -> anyhow::Error {
    match e.kind() {
        ConnectErrorKind::Timeout => anyhow!(
            "the signer app did not answer in time; check that it is running and approve the request"
        ),
        ConnectErrorKind::Rejected => anyhow!("the signer app refused the request: {e}"),
        _ => anyhow!("talking to the signer app failed: {e}"),
    }
}

pub fn requested_perms(kinds: &[u16]) -> String {
    std::iter::once("get_public_key".to_string())
        .chain(kinds.iter().map(|k| format!("sign_event:{k}")))
        .collect::<Vec<_>>()
        .join(",")
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
    let client_uri = NostrConnectUri::client_with_secret(
        app_keys.public_key(),
        request.relays.iter().cloned(),
        APP_NAME,
        secret.clone(),
    );
    let connect = NostrConnect::new(client_uri, app_keys.clone(), request.pairing_timeout, None)
        .map_err(|e| anyhow!("preparing the signer app connection: {e}"))?;
    let connected = async {
        let user = connect
            .get_public_key_async()
            .await
            .map_err(describe_connect_error)?;
        let signer = match connect.bunker_uri().await.map_err(describe_connect_error)? {
            NostrConnectUri::Bunker {
                remote_signer_public_key,
                ..
            } => remote_signer_public_key,
            NostrConnectUri::Client { .. } => bail!("the signer app did not identify itself"),
        };
        Ok((user, signer))
    };
    let unreachable = async {
        tokio::time::sleep(request.relay_timeout).await;
        let status = connect.status().await;
        if status.values().any(|s| *s == RelayStatus::Connected) {
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
    connect.shutdown().await;
    let (user, signer) = connected?;
    *state.lock().expect("pairing state lock") = PairingState::Checking { user };

    let file = RemoteSignerFile {
        app_secret_key: app_keys.secret_key().to_secret_hex(),
        signer_pubkey: signer.to_hex(),
        relays: request.relays.iter().map(|r| r.to_string()).collect(),
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

// Signs a throwaway event of the report kind through a fresh connection, which is
// also exactly how the agent reconnects after a restart.
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
    fn remote_signer_file_round_trips_and_hides_the_app_key() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RemoteSignerFile::load(dir.path()).unwrap().is_none());
        let app = Keys::generate();
        let file = RemoteSignerFile {
            app_secret_key: app.secret_key().to_secret_hex(),
            signer_pubkey: Keys::generate().public_key().to_hex(),
            relays: vec!["wss://relay.example".to_string()],
            user_pubkey: Keys::generate().public_key().to_hex(),
        };
        file.save(dir.path()).unwrap();
        assert_eq!(
            RemoteSignerFile::load(dir.path()).unwrap(),
            Some(file.clone())
        );
        assert!(!format!("{file:?}").contains(&file.app_secret_key));
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

    fn config_in(dir: &Path, secret_key: Option<&str>) -> Config {
        let toml = format!(
            "[nostr]\n{}[agent]\nstate_dir = {:?}\n",
            secret_key.map_or(String::new(), |k| format!("secret_key = {k:?}\n")),
            dir.display().to_string()
        );
        crate::config::build_config_from_str(&toml, |_| None).unwrap()
    }

    fn saved_file() -> RemoteSignerFile {
        RemoteSignerFile {
            app_secret_key: Keys::generate().secret_key().to_secret_hex(),
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

    // Refuses `connect` like Primal does for an app it already knows, so the tests
    // fail if SWING ever sends one.
    struct TestSigner {
        sign: bool,
    }

    impl nostr_connect::prelude::NostrConnectSignerActions for TestSigner {
        fn approve(
            &self,
            _app: &PublicKey,
            req: &nostr_connect::prelude::NostrConnectRequest,
        ) -> bool {
            match req {
                NostrConnectRequest::Connect { .. } => false,
                NostrConnectRequest::SignEvent(_) => self.sign,
                _ => true,
            }
        }
    }

    async fn pair_with(user: &Keys, sign: bool) -> (LocalRelay, PairedSigner) {
        use nostr_connect::prelude::{NostrConnectKeys, NostrConnectRemoteSigner};

        let relay = LocalRelay::new();
        relay.run().await.unwrap();
        let pairing = Pairing::start(PairingRequest {
            relays: vec![relay.url().await],
            perms: requested_perms(&[35981]),
            probe_kind: 35981,
            pairing_timeout: Duration::from_secs(10),
            relay_timeout: Duration::from_secs(5),
            probe_timeout: Duration::from_secs(3),
        })
        .unwrap();
        // The app has to be listening before the signer answers the QR code.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let uri = NostrConnectUri::parse(pairing.uri()).unwrap();
        let remote = NostrConnectRemoteSigner::from_uri(
            uri,
            NostrConnectKeys::new(Keys::generate(), user.clone()),
            None,
        )
        .unwrap();
        tokio::spawn(async move { remote.serve(TestSigner { sign }).await });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            match pairing.state() {
                PairingState::Ready(paired) => return (relay, *paired),
                PairingState::Failed(e) => panic!("pairing failed: {e}"),
                _ if tokio::time::Instant::now() > deadline => panic!("pairing timed out"),
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
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

    #[tokio::test]
    async fn an_unreachable_relay_fails_the_pairing_quickly() {
        let pairing = Pairing::start(PairingRequest {
            relays: vec![RelayUrl::parse("ws://127.0.0.1:1").unwrap()],
            perms: requested_perms(&[35981]),
            probe_kind: 35981,
            pairing_timeout: Duration::from_secs(60),
            relay_timeout: Duration::from_millis(500),
            probe_timeout: Duration::from_secs(3),
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

    #[tokio::test]
    async fn answers_from_a_signer_whose_clock_runs_behind_are_received() {
        let relay = LocalRelay::new();
        relay.run().await.unwrap();
        let url = relay.url().await;
        let user = Keys::generate();
        let signer_keys = Keys::generate();
        let app = Keys::generate();

        let signer_client = Client::new();
        signer_client.add_relay(url.clone()).await.unwrap();
        signer_client.connect().await;
        signer_client
            .subscribe(
                Filter::new()
                    .kind(Kind::NostrConnect)
                    .pubkey(signer_keys.public_key())
                    .limit(0),
            )
            .await
            .unwrap();
        let mut notifications = signer_client.notifications();
        let answering = {
            let (user, signer_keys, signer_client) =
                (user.clone(), signer_keys.clone(), signer_client.clone());
            tokio::spawn(async move {
                while let Some(notification) = notifications.next().await {
                    let ClientNotification::Event { event, .. } = notification else {
                        continue;
                    };
                    let text =
                        nip44::decrypt(signer_keys.secret_key(), &event.pubkey, &event.content)
                            .unwrap();
                    let message = NostrConnectMessage::from_json(text).unwrap();
                    let id = message.id().to_string();
                    let NostrConnectRequest::SignEvent(unsigned) = message.to_request().unwrap()
                    else {
                        panic!("only sign_event is expected");
                    };
                    let signed = unsigned.finalize(&user).unwrap();
                    let answer = NostrConnectMessage::response(
                        id,
                        NostrConnectResponse::with_result(ResponseResult::SignEvent(Box::new(
                            signed,
                        ))),
                    );
                    let behind = Timestamp::from_secs(Timestamp::now().as_secs() - 600);
                    let content = nip44::encrypt(
                        signer_keys.secret_key(),
                        &event.pubkey,
                        answer.as_json(),
                        nip44::Version::default(),
                    )
                    .unwrap();
                    let reply = EventBuilder::new(Kind::NostrConnect, content)
                        .tag(Tag::public_key(event.pubkey))
                        .custom_created_at(behind)
                        .finalize(&signer_keys)
                        .unwrap();
                    signer_client.send_event(&reply).await.unwrap();
                    return;
                }
            })
        };

        let file = RemoteSignerFile {
            app_secret_key: app.secret_key().to_secret_hex(),
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
        answering.await.unwrap();
        remote.shutdown().await;
        signer_client.shutdown().await;
    }
}
