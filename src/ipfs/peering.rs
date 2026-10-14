use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{AutoNat, IpfsClient, percent_encode_segment};

const PEERING_TIMEOUT: Duration = Duration::from_secs(10);

pub trait KuboNode {
    fn peer_id(&self) -> impl Future<Output = Result<String>> + Send;
    fn autonat(&self) -> impl Future<Output = Result<AutoNat>> + Send;
    fn peering_list(&self) -> impl Future<Output = Result<BTreeSet<String>>> + Send;
    fn peering_add(&self, peer_id: &str) -> impl Future<Output = Result<()>> + Send;
    fn peering_remove(&self, peer_id: &str) -> impl Future<Output = Result<()>> + Send;
    fn connected_peers(&self) -> impl Future<Output = Result<BTreeSet<String>>> + Send;
}

#[derive(Debug, Deserialize)]
struct PeeringLsResponse {
    #[serde(rename = "Peers")]
    peers: Option<Vec<PeeringPeer>>,
}

#[derive(Debug, Deserialize)]
struct PeeringPeer {
    #[serde(rename = "ID")]
    id: String,
}

#[derive(Debug, Deserialize)]
struct SwarmPeersResponse {
    #[serde(rename = "Peers")]
    peers: Option<Vec<SwarmPeer>>,
}

#[derive(Debug, Deserialize)]
struct SwarmPeer {
    #[serde(rename = "Peer")]
    peer: String,
}

pub fn parse_peering_ls(text: &str) -> Result<BTreeSet<String>> {
    let parsed: PeeringLsResponse =
        serde_json::from_str(text).context("parsing swarm/peering/ls response")?;
    Ok(parsed
        .peers
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
        .collect())
}

pub fn parse_swarm_peers(text: &str) -> Result<BTreeSet<String>> {
    let parsed: SwarmPeersResponse =
        serde_json::from_str(text).context("parsing swarm/peers response")?;
    Ok(parsed
        .peers
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.peer)
        .collect())
}

impl IpfsClient {
    pub async fn peering_list(&self) -> Result<BTreeSet<String>> {
        let text = self.call("swarm/peering/ls", "", PEERING_TIMEOUT).await?;
        parse_peering_ls(&text)
    }

    // A bare /p2p/<id> is enough: Kubo's host resolves the addresses through peer routing when it dials.
    pub async fn peering_add(&self, peer_id: &str) -> Result<()> {
        self.call(
            "swarm/peering/add",
            &format!("arg=/p2p/{}", percent_encode_segment(peer_id)),
            PEERING_TIMEOUT,
        )
        .await
        .map(drop)
    }

    pub async fn peering_remove(&self, peer_id: &str) -> Result<()> {
        self.call(
            "swarm/peering/rm",
            &format!("arg={}", percent_encode_segment(peer_id)),
            PEERING_TIMEOUT,
        )
        .await
        .map(drop)
    }

    pub async fn connected_peers(&self) -> Result<BTreeSet<String>> {
        let text = self.call("swarm/peers", "", PEERING_TIMEOUT).await?;
        parse_swarm_peers(&text)
    }
}

impl KuboNode for IpfsClient {
    async fn peer_id(&self) -> Result<String> {
        IpfsClient::peer_id(self).await
    }

    async fn autonat(&self) -> Result<AutoNat> {
        IpfsClient::autonat(self).await
    }

    async fn peering_list(&self) -> Result<BTreeSet<String>> {
        IpfsClient::peering_list(self).await
    }

    async fn peering_add(&self, peer_id: &str) -> Result<()> {
        IpfsClient::peering_add(self, peer_id).await
    }

    async fn peering_remove(&self, peer_id: &str) -> Result<()> {
        IpfsClient::peering_remove(self, peer_id).await
    }

    async fn connected_peers(&self) -> Result<BTreeSet<String>> {
        IpfsClient::connected_peers(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "12D3KooWQqg6fSEnehBFPAzVpSwWzFheYrfktHPXLoNDJUisx9LJ";

    #[test]
    fn peering_ls_reads_the_ids_and_an_empty_list() {
        let text = format!(r#"{{"Peers":[{{"ID":"{PEER}","Addrs":[]}}]}}"#);
        assert_eq!(
            parse_peering_ls(&text).unwrap(),
            BTreeSet::from([PEER.to_string()])
        );
        assert!(parse_peering_ls(r#"{"Peers":null}"#).unwrap().is_empty());
        assert!(parse_peering_ls("{}").unwrap().is_empty());
    }

    #[test]
    fn swarm_peers_reads_the_peer_ids() {
        let text = format!(
            r#"{{"Peers":[{{"Addr":"/ip4/172.22.0.3/udp/4001/quic-v1","Peer":"{PEER}","Identify":{{}}}}]}}"#
        );
        assert_eq!(
            parse_swarm_peers(&text).unwrap(),
            BTreeSet::from([PEER.to_string()])
        );
        assert!(parse_swarm_peers(r#"{"Peers":null}"#).unwrap().is_empty());
    }

    #[tokio::test]
    async fn peering_calls_send_a_bare_p2p_address_and_the_id() {
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let record = |seen: Arc<Mutex<Vec<String>>>| {
            move |uri: axum::http::Uri| async move {
                seen.lock().unwrap().push(uri.to_string());
                format!(r#"{{"ID":"{PEER}","Status":"success"}}"#)
            }
        };
        let router = axum::Router::new()
            .route(
                "/api/v0/swarm/peering/add",
                axum::routing::post(record(Arc::clone(&seen))),
            )
            .route(
                "/api/v0/swarm/peering/rm",
                axum::routing::post(record(Arc::clone(&seen))),
            );
        let addr = crate::test_support::serve_router(router).await;
        let client = IpfsClient::new(format!("http://{addr}"));
        client.peering_add(PEER).await.unwrap();
        client.peering_remove(PEER).await.unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            [
                format!("/api/v0/swarm/peering/add?arg=/p2p/{PEER}"),
                format!("/api/v0/swarm/peering/rm?arg={PEER}"),
            ]
        );
    }
}
