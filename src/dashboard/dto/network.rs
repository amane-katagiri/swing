use serde::{Deserialize, Serialize};

use super::AccountDto;

pub const REACHABILITY_GUIDE_URL: &str = concat!(
    "https://github.com/amane-katagiri/swing/blob/v",
    env!("CARGO_PKG_VERSION"),
    "/docs/guide/operation.md#外から届くか"
);

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkDto {
    pub reachability: String,
    pub nat: Option<String>,
    pub reachable: Vec<String>,
    pub unreachable: Vec<String>,
    pub unknown: Vec<String>,
    pub guide: String,
    pub anchor: AnchorDto,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnchorDto {
    pub advertise: bool,
    pub advertising: bool,
    pub max_peers: usize,
    pub anchors: Vec<AnchorPeerDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnchorPeerDto {
    pub peer_id: String,
    #[serde(flatten)]
    pub account: AccountDto,
    pub reason: String,
    pub sites: usize,
    pub connected: Option<bool>,
}

pub fn anchor_dto(
    config: &crate::config::AnchorConfig,
    snapshot: crate::anchor::Snapshot,
) -> AnchorDto {
    let connected = snapshot.connected;
    AnchorDto {
        advertise: config.advertise,
        advertising: snapshot.advertising,
        max_peers: config.max_peers,
        anchors: snapshot
            .selected
            .into_iter()
            .map(|s| AnchorPeerDto {
                connected: connected.as_ref().map(|c| c.contains(&s.peer_id)),
                account: AccountDto::new(&s.advertiser),
                reason: s.reason.token().to_string(),
                sites: s.sites,
                peer_id: s.peer_id,
            })
            .collect(),
    }
}

pub fn network_dto(autonat: crate::ipfs::AutoNat, anchor: AnchorDto) -> NetworkDto {
    NetworkDto {
        reachability: autonat.reachability.token().to_string(),
        nat: autonat.nat.map(|n| n.token().to_string()),
        reachable: autonat.reachable,
        unreachable: autonat.unreachable,
        unknown: autonat.unknown,
        guide: REACHABILITY_GUIDE_URL.to_string(),
        anchor,
    }
}
