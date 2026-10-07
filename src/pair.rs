use std::time::Duration;

use anyhow::{Result, bail};
use nostr_sdk::prelude::PublicKey;

use crate::config::Config;
use crate::mirror;
use crate::signer::{self, Pairing, PairingRequest, PairingState, RemoteSignerFile};

pub const DEFAULT_RELAY: &str = "wss://relay.primal.net";
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub async fn run(config: &Config, relays: &[String]) -> Result<()> {
    let relays = signer::parse_pairing_relays(relays)?;
    pair_and_save(
        config,
        PairingRequest::for_config(&config.nostr, relays),
        |uri| {
            match signer::qr_text(uri) {
                Ok(qr) => println!("{qr}\n"),
                Err(e) => eprintln!("could not draw the QR code ({e:#}); use the link below"),
            }
            println!("{uri}\n");
            println!(
                "scan the QR code with your signer app, or paste the link into it (waiting up to {} minutes)",
                signer::PAIRING_TIMEOUT.as_secs() / 60
            );
        },
    )
    .await?;
    println!(
        "if swing up is running, restart it to use the signer app (swing stop --restart, or restart the service)"
    );
    Ok(())
}

async fn pair_and_save(
    config: &Config,
    request: PairingRequest,
    show: impl FnOnce(&str),
) -> Result<()> {
    if config.nostr.secret_key.is_some() {
        bail!(
            "swing signs with a Nostr secret key ([nostr].secret_key or SWING_NOSTR_SECRET_KEY); remove it before pairing a signer app"
        );
    }
    let state_dir = &config.agent.state_dir;
    let current = RemoteSignerFile::load(state_dir)?
        .map(|file| file.user_public_key())
        .transpose()?;

    let pairing = Pairing::start(request)?;
    show(pairing.uri());
    let mut announced = false;
    let paired = loop {
        match pairing.state() {
            PairingState::Waiting => {}
            PairingState::Checking { user } => {
                check_account(current, user)?;
                if !announced {
                    println!(
                        "connected as {}; asking the signer app to sign a check event...",
                        mirror::npub(&user)
                    );
                    announced = true;
                }
            }
            PairingState::Ready(paired) => break paired,
            PairingState::Failed(e) => bail!("pairing failed: {e}"),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    };

    check_account(current, paired.user)?;
    paired.file.save(state_dir)?;
    match &paired.probe_error {
        None => println!("the check event was signed"),
        Some(e) => eprintln!(
            "warning: the check event was not signed ({}); allow SWING's requests in the signer app",
            crate::format::Sanitized(e)
        ),
    }
    println!(
        "paired: swing now signs as {} (saved to {})",
        mirror::npub(&paired.user),
        signer::remote_signer_path(state_dir).display()
    );
    Ok(())
}

fn check_account(current: Option<PublicKey>, user: PublicKey) -> Result<()> {
    match current {
        Some(own) if own != user => bail!(
            "the signer app signs as {}, not as this swing's {}; connect the same Nostr account",
            mirror::npub(&user),
            mirror::npub(&own)
        ),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::*;

    use super::*;

    fn config_in(dir: &std::path::Path, secret_key: Option<&str>) -> Config {
        let toml = format!(
            "[nostr]\n{}[agent]\nstate_dir = {:?}\n",
            secret_key.map_or(String::new(), |k| format!("secret_key = {k:?}\n")),
            dir.display().to_string()
        );
        crate::config::build_config_from_str(&toml, |_| None).unwrap()
    }

    fn request(relay: RelayUrl) -> PairingRequest {
        PairingRequest {
            relays: vec![relay],
            perms: signer::requested_perms(&[35981]),
            probe_kind: 35981,
            pairing_timeout: Duration::from_secs(10),
            relay_timeout: Duration::from_secs(5),
            probe_timeout: Duration::from_secs(3),
        }
    }

    async fn local_relay() -> (LocalRelay, RelayUrl) {
        let relay = LocalRelay::new();
        relay.run().await.unwrap();
        let url = relay.url().await;
        (relay, url)
    }

    #[tokio::test]
    async fn pairs_and_saves_the_signer_app() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_in(dir.path(), None);
        let (_relay, url) = local_relay().await;
        let user = Keys::generate();
        pair_and_save(&config, request(url), |uri| {
            crate::test_support::serve_test_signer(uri, &user, true)
        })
        .await
        .unwrap();
        let saved = RemoteSignerFile::load(dir.path()).unwrap().unwrap();
        assert_eq!(saved.user_public_key().unwrap(), user.public_key());
    }

    #[tokio::test]
    async fn refuses_while_a_secret_key_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::generate();
        let config = config_in(dir.path(), Some(&keys.secret_key().to_secret_hex()));
        let err = pair_and_save(
            &config,
            request(RelayUrl::parse("ws://127.0.0.1:1").unwrap()),
            |_| panic!("no QR code should be shown"),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("secret key"), "{err}");
        assert!(RemoteSignerFile::load(dir.path()).unwrap().is_none());
    }

    #[tokio::test]
    async fn keeps_the_paired_account_when_another_one_connects() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_in(dir.path(), None);
        let (_relay, url) = local_relay().await;
        let own = Keys::generate();
        pair_and_save(&config, request(url.clone()), |uri| {
            crate::test_support::serve_test_signer(uri, &own, true)
        })
        .await
        .unwrap();
        let before = RemoteSignerFile::load(dir.path()).unwrap().unwrap();

        let other = Keys::generate();
        let err = pair_and_save(&config, request(url), |uri| {
            crate::test_support::serve_test_signer(uri, &other, true)
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("same Nostr account"), "{err}");
        assert_eq!(RemoteSignerFile::load(dir.path()).unwrap(), Some(before));
    }
}
