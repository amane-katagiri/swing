use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::mirror;
use crate::settings;
use crate::signer::{self, Pairing, PairingRequest, PairingState, Signer};

use super::AppState;
use super::dto;
use super::error::{ApiError, AppJson, blocking, internal, settings_error};

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    secret_key: Option<Zeroizing<String>>,
    #[serde(default)]
    remote_signer: bool,
    #[serde(default)]
    items: BTreeMap<String, settings::InputValue>,
}

fn ensure_setup_mode(state: &AppState) -> Result<(), ApiError> {
    if state.setup_mode() {
        Ok(())
    } else {
        Err(ApiError::Conflict(
            "swing is already configured; setup is no longer available".to_string(),
        ))
    }
}

fn uses_signer_app(state: &AppState) -> bool {
    state.signer.as_ref().is_some_and(Signer::is_remote)
}

fn ensure_can_pair(state: &AppState) -> Result<(), ApiError> {
    if state.setup_mode() || uses_signer_app(state) {
        Ok(())
    } else {
        Err(ApiError::Conflict(
            "a signer app can be paired only during setup or when swing already uses one"
                .to_string(),
        ))
    }
}

fn ready_pairing(state: &AppState) -> Result<Box<signer::PairedSigner>, ApiError> {
    match state.pairing_state() {
        Some(PairingState::Ready(paired)) => Ok(paired),
        _ => Err(ApiError::Conflict(
            "no signer app is connected yet; scan the QR code first".to_string(),
        )),
    }
}

fn schedule_restart(state: &AppState) {
    let exit = state.exit.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        exit.restart();
    });
}

pub async fn setup(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<SetupRequest>,
) -> Result<Response, ApiError> {
    ensure_setup_mode(&state)?;
    let mut writes = state.lock_config_writes().await;
    if writes.setup_done {
        return Err(ApiError::Conflict(
            "setup is already done; swing is restarting".to_string(),
        ));
    }
    let config = Arc::clone(&state.config);
    let items = req.items;
    let pubkey = if req.remote_signer {
        let paired = ready_pairing(&state)?;
        let user = blocking(move || {
            settings::setup(&config, None, &items).map_err(settings_error)?;
            paired
                .file
                .save(&config.agent.state_dir)
                .map_err(|e| internal("saving the signer app connection failed", e))?;
            Ok(paired.user)
        })
        .await?;
        *state.pairing.lock().expect("pairing lock") = None;
        user
    } else {
        let keys = settings::setup_keys(req.secret_key.as_ref().map(|s| s.as_str()))
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
        blocking(move || {
            settings::setup(&config, Some(&keys), &items).map_err(settings_error)?;
            Ok(keys.public_key())
        })
        .await?
    };
    writes.setup_done = true;
    drop(writes);
    let npub = mirror::npub(&pubkey);
    schedule_restart(&state);
    let body = Json(serde_json::json!({ "ok": true, "npub": npub, "restart": true }));
    Ok((StatusCode::OK, body).into_response())
}

pub async fn reconnect_signer(State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    if !uses_signer_app(&state) {
        return Err(ApiError::Conflict(
            "swing does not use a signer app".to_string(),
        ));
    }
    let paired = ready_pairing(&state)?;
    let own = state.own_pubkey.expect("a signer implies a public key");
    if paired.user != own {
        return Err(ApiError::Conflict(format!(
            "the signer app signs as {}, not as this swing's {}; connect the same Nostr account",
            mirror::npub(&paired.user),
            mirror::npub(&own)
        )));
    }
    let writes = state.lock_config_writes().await;
    let state_dir = state.config.agent.state_dir.clone();
    blocking(move || {
        paired
            .file
            .save(&state_dir)
            .map_err(|e| internal("saving the signer app connection failed", e))
    })
    .await?;
    drop(writes);
    *state.pairing.lock().expect("pairing lock") = None;
    state.restart_required.store(true, Ordering::SeqCst);
    schedule_restart(&state);
    let body = Json(serde_json::json!({ "ok": true, "npub": mirror::npub(&own), "restart": true }));
    Ok((StatusCode::OK, body).into_response())
}

#[derive(Debug, Deserialize)]
pub struct StartPairingRequest {
    relays: Vec<String>,
}

pub async fn start_pairing(
    State(state): State<Arc<AppState>>,
    AppJson(req): AppJson<StartPairingRequest>,
) -> Result<Json<dto::PairingStartDto>, ApiError> {
    ensure_can_pair(&state)?;
    let bad_request = |e: anyhow::Error| ApiError::BadRequest(format!("{e:#}"));
    let relays = signer::parse_pairing_relays(&req.relays).map_err(bad_request)?;
    let pairing = Pairing::start(PairingRequest::for_config(&state.config.nostr, relays))
        .map_err(bad_request)?;
    let uri = pairing.uri().to_string();
    let qr_svg = signer::qr_svg(&uri).map_err(|e| internal("drawing the QR code failed", e))?;
    *state.pairing.lock().expect("pairing lock") = Some(pairing);
    Ok(Json(dto::PairingStartDto { uri, qr_svg }))
}

pub async fn pairing_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::PairingStatusDto>, ApiError> {
    ensure_can_pair(&state)?;
    Ok(Json(dto::pairing_status_dto(
        state.pairing_state().as_ref(),
    )))
}

#[cfg(test)]
mod tests {
    use super::super::router;
    use super::super::test_support::*;
    use super::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use nostr_sdk::prelude::{Keys, PublicKey};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::signer::{Pairing, PairingState, Signer};

    fn setup_mode_state(dir: &std::path::Path) -> Arc<AppState> {
        let (mut config, _secret_hex) = test_config_in_dir(dir, true, false);
        config.agent.state_dir = dir.join("data");
        build_state(config, test_exit(), None, TEST_TOKEN)
    }

    #[tokio::test]
    async fn post_setup_generates_a_key_and_schedules_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (config, _secret_hex) = test_config_in_dir(dir.path(), true, false);
        let state = build_state(config, test_exit(), None, TEST_TOKEN);
        assert!(state.setup_mode());
        let app = router(state);
        let body = serde_json::json!({ "secret_key": null, "items": {} }).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/api/setup")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = error_body(resp).await;
        assert_eq!(json["ok"], true);
        assert_eq!(json["restart"], true);
        assert!(json["npub"].as_str().unwrap().starts_with("npub1"));
        assert!(dir.path().join("swing.toml").exists());
    }

    #[tokio::test]
    async fn setup_after_put_config_keeps_the_put_item() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        let put = serde_json::json!({ "items": { "policy.max_total_storage": "20GB" } });
        let (status, _) = send_json(app.clone(), "PUT", "/api/config", Some(put)).await;
        assert_eq!(status, StatusCode::OK);
        let body = serde_json::json!({ "secret_key": null, "items": {} });
        let (status, _) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::OK);

        let path = dir.path().join("swing.toml");
        let saved = crate::config::Config::load(Some(&path)).unwrap();
        assert_eq!(saved.policy.max_total_storage, 20 * (1u64 << 30));
        assert!(saved.nostr.secret_key.is_some());
    }

    #[tokio::test]
    async fn a_second_setup_before_the_restart_is_conflict_and_keeps_the_first_key() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        let body = serde_json::json!({ "secret_key": null, "items": {} });
        let (status, first) =
            send_json(app.clone(), "POST", "/api/setup", Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);

        let path = dir.path().join("swing.toml");
        let saved = crate::config::Config::load(Some(&path)).unwrap();
        let secret = saved.nostr.secret_key.unwrap();
        let keys = Keys::parse(secret.expose_secret()).unwrap();
        assert_eq!(first["npub"], crate::mirror::npub(&keys.public_key()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn setup_into_an_unwritable_config_directory_is_internal_error() {
        let dir = tempfile::tempdir().unwrap();
        let Some(locked) = read_only_config_dir(dir.path()) else {
            return;
        };
        let app = router(setup_mode_state(&locked));
        let body = serde_json::json!({ "secret_key": null, "items": {} });
        let (status, _) = send_json(app.clone(), "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let bad = serde_json::json!({ "secret_key": "not-a-key", "items": {} });
        let (status, _) = send_json(app, "POST", "/api/setup", Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signer_pairing_is_not_available_with_a_local_key() {
        let app = router(test_state());
        let body = serde_json::json!({ "relays": ["wss://relay.example"] });
        let (status, _) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = send_json(app, "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn starting_a_pairing_returns_a_qr_code_and_waits_for_the_signer() {
        let dir = tempfile::tempdir().unwrap();
        let state = setup_mode_state(dir.path());
        let app = router(Arc::clone(&state));
        let (status, json) = send_json(app.clone(), "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "idle");

        let body = serde_json::json!({ "relays": ["ws://127.0.0.1:1"] });
        let (status, json) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let uri = reqwest::Url::parse(json["uri"].as_str().unwrap()).unwrap();
        assert_eq!(uri.scheme(), "nostrconnect");
        let perms = uri
            .query_pairs()
            .find(|(k, _)| k == "perms")
            .map(|(_, v)| v.into_owned());
        assert_eq!(
            perms.as_deref(),
            Some("get_public_key,sign_event:35981,sign_event:35980,sign_event:30000")
        );
        assert!(json["qr_svg"].as_str().unwrap().contains("<svg"));

        let (status, json) = send_json(app, "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "waiting");
    }

    #[tokio::test]
    async fn invalid_signer_relays_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        for relays in [
            serde_json::json!([]),
            serde_json::json!(["https://relay.example"]),
            serde_json::json!([
                "wss://a", "wss://b", "wss://c", "wss://d", "wss://e", "wss://f"
            ]),
        ] {
            let body = serde_json::json!({ "relays": relays });
            let (status, _) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{relays}");
        }
    }

    fn remote_signer_file(user: PublicKey) -> crate::signer::RemoteSignerFile {
        crate::signer::RemoteSignerFile {
            app_secret_key: Keys::generate().secret_key().to_secret_hex().into(),
            signer_pubkey: Keys::generate().public_key().to_hex(),
            relays: vec!["wss://relay.example".to_string()],
            user_pubkey: user.to_hex(),
        }
    }

    fn remote_signer_state(dir: &std::path::Path, user: PublicKey) -> Arc<AppState> {
        let (mut config, _secret_hex) = test_config_in_dir(dir, true, false);
        config.agent.state_dir = dir.join("data");
        let remote = crate::signer::RemoteSigner::from_file(
            &remote_signer_file(user),
            Duration::from_secs(1),
        )
        .unwrap();
        build_state(
            config,
            test_exit(),
            Some(Signer::Remote(Arc::new(remote))),
            TEST_TOKEN,
        )
    }

    fn finish_pairing(state: &AppState, file: crate::signer::RemoteSignerFile, user: PublicKey) {
        *state.pairing.lock().unwrap() = Some(Pairing::finished(PairingState::Ready(Box::new(
            crate::signer::PairedSigner {
                file,
                user,
                probe_signed: true,
                probe_error: None,
            },
        ))));
    }

    #[tokio::test]
    async fn a_running_signer_app_user_can_pair_again() {
        let dir = tempfile::tempdir().unwrap();
        let user = Keys::generate().public_key();
        let state = remote_signer_state(dir.path(), user);
        let app = router(Arc::clone(&state));
        let (status, json) = send_json(app.clone(), "GET", "/api/overview", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["signer"]["remote"], true);
        assert_eq!(
            json["signer"]["relays"],
            serde_json::json!(["wss://relay.example"])
        );

        let body = serde_json::json!({ "relays": ["ws://127.0.0.1:1"] });
        let (status, _) = send_json(app.clone(), "POST", "/api/setup/signer", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send_json(app.clone(), "POST", "/api/signer/reconnect", None).await;
        assert_eq!(status, StatusCode::CONFLICT);

        let file = remote_signer_file(user);
        finish_pairing(&state, file.clone(), user);
        let (status, json) = send_json(app, "POST", "/api/signer/reconnect", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["restart"], true);
        let saved = crate::signer::RemoteSignerFile::load(&state.config.agent.state_dir)
            .unwrap()
            .unwrap();
        assert_eq!(saved, file);
        assert!(state.pairing_state().is_none());
    }

    #[tokio::test]
    async fn reconnecting_a_different_account_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let user = Keys::generate().public_key();
        let state = remote_signer_state(dir.path(), user);
        let other = Keys::generate().public_key();
        finish_pairing(&state, remote_signer_file(other), other);
        let (status, json) = send_json(
            router(Arc::clone(&state)),
            "POST",
            "/api/signer/reconnect",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .contains("same Nostr account")
        );
        assert!(
            crate::signer::RemoteSignerFile::load(&state.config.agent.state_dir)
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn reconnecting_needs_a_signer_app() {
        let (status, _) =
            send_json(router(test_state()), "POST", "/api/signer/reconnect", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn setup_with_a_signer_needs_a_finished_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(setup_mode_state(dir.path()));
        let body = serde_json::json!({ "remote_signer": true, "items": {} });
        let (status, _) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn setup_with_a_paired_signer_saves_it_without_a_secret_key() {
        let dir = tempfile::tempdir().unwrap();
        let state = setup_mode_state(dir.path());
        let user = Keys::generate().public_key();
        let file = remote_signer_file(user);
        finish_pairing(&state, file.clone(), user);
        let app = router(Arc::clone(&state));
        let (status, json) = send_json(app.clone(), "GET", "/api/setup/signer", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "ready");
        assert_eq!(json["probe_signed"], true);

        let body = serde_json::json!({ "remote_signer": true, "items": {} });
        let (status, json) = send_json(app, "POST", "/api/setup", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["npub"], crate::mirror::npub(&user));
        let saved = crate::signer::RemoteSignerFile::load(&state.config.agent.state_dir)
            .unwrap()
            .unwrap();
        assert_eq!(saved, file);
        let written = std::fs::read_to_string(dir.path().join("swing.toml")).unwrap();
        assert!(!written.contains("secret_key"));
        assert!(state.pairing_state().is_none());
    }

    #[tokio::test]
    async fn post_setup_when_already_configured_is_conflict() {
        let app = router(test_state());
        let body = serde_json::json!({ "secret_key": null, "items": {} }).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/api/setup")
            .header("Host", "127.0.0.1:8082")
            .header("x-swing-dashboard", "1")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = call(app, req).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }
}
