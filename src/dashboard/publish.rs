use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nostr_sdk::prelude::Timestamp;
use tokio::sync::MutexGuard;

use crate::config;
use crate::mfs::MfsLayout;
use crate::nip05;
use crate::publish;

use super::AppState;
use super::dto;
use super::error::{ApiError, internal, upstream};

pub(super) struct PublishFields {
    pub site: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub modes: publish::ModeOverrides,
}

pub(super) enum PublishOutcome {
    Success(Box<dto::PublishResultDto>),
    CheckFailed(Response),
}

fn check_failed(error: String, mut body: serde_json::Value) -> Response {
    body["error"] = serde_json::Value::String(error);
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

async fn run_publish_nip05(
    nip05_mode: config::CheckMode,
    site: &str,
    pubkey_hex: &str,
) -> Result<dto::Nip05ResultDto, Response> {
    if nip05_mode == config::CheckMode::Off {
        return Ok(dto::nip05_off_dto());
    }
    let verifier = nip05::HttpNip05Verifier::public_only();
    let outcome = publish::check_nip05(&verifier, nip05_mode, site, pubkey_hex).await;
    if let Some(abort) = outcome.abort {
        let body = serde_json::json!({ "nip05": dto::nip05_result_dto(&outcome.result) });
        return Err(check_failed(abort, body));
    }
    Ok(dto::nip05_result_dto(&outcome.result))
}

pub(super) struct PublishLock<'a> {
    _guard: MutexGuard<'a, ()>,
}

pub(super) fn try_lock_publish(state: &AppState) -> Result<PublishLock<'_>, ApiError> {
    state
        .publish_lock
        .try_lock()
        .map(|guard| PublishLock { _guard: guard })
        .map_err(|_| ApiError::Conflict("a publish is already running".to_string()))
}

pub(super) async fn run_publish(
    state: &AppState,
    _publishing: &PublishLock<'_>,
    dir: &std::path::Path,
    fields: PublishFields,
) -> Result<PublishOutcome, ApiError> {
    publish::validate_site_fields(&fields.site, fields.url.as_deref()).map_err(|e| match e {
        publish::SiteFieldError::InvalidD(err) => {
            ApiError::BadRequest(format!("invalid site: {err:#}"))
        }
        publish::SiteFieldError::InvalidUrl(url) => {
            ApiError::BadRequest(format!("invalid url: {url} is not an http or https URL"))
        }
    })?;
    let title = publish::normalize_title(fields.title.as_deref()).map_err(|_| {
        ApiError::BadRequest(
            "invalid title: must not exceed 256 bytes and must not contain control characters"
                .to_string(),
        )
    })?;
    let modes =
        publish::resolve_modes(&fields.modes, &state.config.publish).map_err(|(name, e)| {
            ApiError::BadRequest(format!("invalid {}: {e:#}", name.replace('-', "_")))
        })?;
    publish::refuse_protected_paths(dir, &state.config)
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;

    let pubkey_hex = state.require_own_pubkey()?.to_hex();

    let nip05_dto = match run_publish_nip05(modes.nip05, &fields.site, &pubkey_hex).await {
        Ok(dto) => dto,
        Err(resp) => return Ok(PublishOutcome::CheckFailed(resp)),
    };

    let site = crate::ipfs::SiteListing::read_async(dir)
        .await
        .map_err(|e| internal("listing the uploaded files failed", e))?;
    let local = publish::LocalChecks::evaluate(
        &site.entries(),
        modes.check_dotfiles,
        modes.check_size,
        &state.config.publish.dotfiles_allow,
    );
    if let Some(abort) = local.abort_message() {
        let body = serde_json::json!({
            "nip05": nip05_dto,
            "checks": dto::publish_checks_dto(&local, None),
        });
        return Ok(PublishOutcome::CheckFailed(check_failed(abort, body)));
    }

    let ipfs = state.require_ipfs().await?;
    let relay = state.require_relay().await?;
    let layout = MfsLayout::new(state.config.ipfs.mfs_root.clone());
    let created_at = Timestamp::now();
    let stage = publish::add_and_measure(
        &ipfs,
        &layout,
        &pubkey_hex,
        &fields.site,
        created_at.as_secs(),
        site,
    )
    .await
    .map_err(upstream)?;

    let unchanged = publish::check_unchanged(
        &relay,
        modes.check_unchanged,
        state.config.nostr.site_event_kind,
        &fields.site,
        &stage.cid,
    )
    .await;
    let mut result = Box::new(dto::PublishResultDto {
        published: false,
        gateway_url: dto::gateway_url(state.config.dashboard.gateway.as_deref(), &stage.cid, true),
        site: fields.site,
        url: fields.url,
        title: title.map(str::to_string),
        message: fields.message,
        nip05: nip05_dto,
        checks: dto::publish_checks_dto(&local, Some(&unchanged)),
        cid: stage.cid,
        size: stage.size,
        created_at: None,
        mfs_path: None,
        relays: Vec::new(),
        pruned: Vec::new(),
        prune_error: None,
    });

    if unchanged.stops_publish() {
        stage.version.withdraw().await.map_err(upstream)?;
        return Ok(PublishOutcome::Success(result));
    }

    let signed = publish::sign_site_event(
        &relay,
        &publish::SiteAnnouncement {
            site_event_kind: state.config.nostr.site_event_kind,
            d: &result.site,
            cid: &result.cid,
            url: result.url.as_deref(),
            size: result.size,
            title,
            message: result.message.as_deref(),
            created_at,
        },
    )
    .await;
    let event = match signed {
        Ok(event) => event,
        Err(e) => return Err(upstream(stage.version.fail(e).await)),
    };
    result.mfs_path = Some(stage.version.keep());
    let relay_results = publish::send_site_event(&relay, &event)
        .await
        .map_err(upstream)?;
    if !relay_results.iter().any(|r| r.ok) {
        return Err(upstream(anyhow::anyhow!(publish::NO_RELAY_ACCEPTED)));
    }
    state.activity.record_published(created_at.as_secs());

    let site_path = layout.publish_site(&pubkey_hex, &result.site);
    let prune = publish::prune_old_versions_collect(
        &ipfs,
        &site_path,
        created_at.as_secs(),
        state.config.publish.keep_versions,
    )
    .await;

    result.published = true;
    result.created_at = Some(created_at.as_secs());
    result.relays = dto::relay_results_dto(&relay_results);
    result.pruned = prune.pruned().into_iter().map(str::to_string).collect();
    result.prune_error = prune.error_summary();
    Ok(PublishOutcome::Success(result))
}
