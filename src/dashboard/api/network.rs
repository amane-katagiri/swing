use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use super::super::AppState;
use super::super::dto;
use super::super::error::{ApiError, upstream};

pub async fn network(
    State(state): State<Arc<AppState>>,
) -> Result<Json<dto::NetworkDto>, ApiError> {
    let ipfs = state.require_ipfs().await?;
    let autonat = ipfs.autonat().await.map_err(upstream)?;
    let anchor = dto::anchor_dto(&state.config.anchor, state.anchors.snapshot());
    Ok(Json(dto::network_dto(autonat, anchor)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::http::StatusCode;

    use super::super::super::router;
    use super::super::super::test_support::*;
    use super::AppState;
    use super::dto;

    async fn state_with_kubo(kubo: axum::Router) -> Arc<AppState> {
        let state = test_state();
        let (_, secret_hex) = test_config(true);
        let relay = crate::nostr::RelayClient::connect(test_keys(&secret_hex).unwrap(), &[])
            .await
            .unwrap();
        let addr = crate::test_support::serve_router(kubo).await;
        let ipfs = crate::ipfs::IpfsClient::new(format!("http://{addr}"));
        state.set_ready(Arc::new(relay), ipfs).await;
        state
    }

    #[tokio::test]
    async fn network_without_ipfs_reports_service_unavailable() {
        let (status, json) = send_json(router(test_state()), "GET", "/api/network", None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "agent is not ready");
    }

    #[tokio::test]
    async fn network_reports_what_autonat_found() {
        let kubo = axum::Router::new().route(
            "/api/v0/swarm/addrs/autonat",
            axum::routing::post(|| async {
                r#"{"reachability":"Private","nat":"double-nat","unreachable":["/ip4/198.51.100.7/tcp/4001"],"unknown":["/ip4/198.51.100.7/udp/4001/quic-v1"]}"#
            }),
        );
        let state = state_with_kubo(kubo).await;
        let (status, json) = send_json(router(state), "GET", "/api/network", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!({
                "reachability": "private",
                "nat": "double-nat",
                "reachable": [],
                "unreachable": ["/ip4/198.51.100.7/tcp/4001"],
                "unknown": ["/ip4/198.51.100.7/udp/4001/quic-v1"],
                "guide": dto::network::REACHABILITY_GUIDE_URL,
                "anchor": {
                    "advertise": true,
                    "advertising": false,
                    "max_peers": 4,
                    "anchors": [],
                },
            })
        );
    }

    #[tokio::test]
    async fn network_lists_the_chosen_anchors_and_what_the_agent_saw_connected() {
        use crate::test_support::{PEER_A, PEER_B};
        let kubo = axum::Router::new().route(
            "/api/v0/swarm/addrs/autonat",
            axum::routing::post(|| async { r#"{"reachability":"Public"}"# }),
        );
        let state = state_with_kubo(kubo).await;
        let advertiser = nostr_sdk::prelude::Keys::generate().public_key();
        state.anchors.set_advertising(true);
        state.anchors.set_selected(vec![
            crate::anchor::Selected {
                peer_id: PEER_A.to_string(),
                advertiser,
                reason: crate::anchor::Reason::Mutual,
                sites: 0,
            },
            crate::anchor::Selected {
                peer_id: PEER_B.to_string(),
                advertiser,
                reason: crate::anchor::Reason::Coverage,
                sites: 3,
            },
        ]);
        state
            .anchors
            .set_connected(Some(BTreeSet::from([PEER_A.to_string()])));
        let (status, json) =
            send_json(router(Arc::clone(&state)), "GET", "/api/network", None).await;
        assert_eq!(status, StatusCode::OK);
        let anchor = &json["anchor"];
        assert_eq!(anchor["advertising"], true);
        assert_eq!(
            anchor["anchors"][0],
            serde_json::json!({
                "peer_id": PEER_A,
                "pubkey": advertiser.to_hex(),
                "npub": crate::mirror::npub(&advertiser),
                "fingerprint": crate::fingerprint::fingerprint(&advertiser),
                "reason": "mutual",
                "sites": 0,
                "connected": true,
            })
        );
        let second = &anchor["anchors"][1];
        assert_eq!(second["reason"], "coverage");
        assert_eq!(second["sites"], 3);
        assert_eq!(second["connected"], false);

        state.anchors.set_connected(None);
        let (_, json) = send_json(router(state), "GET", "/api/network", None).await;
        assert!(json["anchor"]["anchors"][0]["connected"].is_null());
    }

    fn failing_kubo(message: &'static str) -> axum::Router {
        axum::Router::new().route(
            "/api/v0/swarm/addrs/autonat",
            axum::routing::post(move || async move {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(r#"{{"Message":"{message}","Code":0,"Type":"error"}}"#),
                )
            }),
        )
    }

    #[tokio::test]
    async fn network_reports_an_offline_kubo_as_offline() {
        let state = state_with_kubo(failing_kubo(
            "this command must be run in online mode. Try running 'ipfs daemon' first",
        ))
        .await;
        let (status, json) = send_json(router(state), "GET", "/api/network", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["reachability"], "offline");
        assert!(json["nat"].is_null());
        for list in ["reachable", "unreachable", "unknown"] {
            assert_eq!(json[list], serde_json::json!([]), "{list}");
        }
    }

    #[tokio::test]
    async fn network_reports_a_kubo_failure_as_bad_gateway() {
        let state = state_with_kubo(failing_kubo("routing: not supported")).await;
        let (status, json) = send_json(router(state), "GET", "/api/network", None).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        let message = json["error"].as_str().unwrap();
        assert!(message.contains("not supported"), "{message}");
    }

    #[test]
    fn the_reachability_guide_points_at_a_heading_in_the_operation_guide() {
        let anchor = dto::network::REACHABILITY_GUIDE_URL
            .rsplit_once('#')
            .unwrap()
            .1;
        let guide = include_str!("../../../docs/guide/operation.md");
        assert!(
            guide
                .lines()
                .any(|l| l.trim_start_matches('#').trim() == anchor),
            "{anchor}"
        );
    }
}
