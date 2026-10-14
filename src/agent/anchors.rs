use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use nostr_sdk::prelude::*;
use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use crate::anchor::{
    self, AD_RETRY, AD_TTL, AnchorStatus, Hysteresis, Known, Observed, Published, Selected, Settled,
};
use crate::ipfs::{KuboNode, KuboStore};
use crate::nip05::Nip05Verify;
use crate::nostr::{self, AnchorRelay, ReportRelay};
use crate::state::{self, SiteKey};

use super::{Agent, now_secs};

#[derive(Debug)]
pub(super) struct AdBook {
    pub(super) hysteresis: Hysteresis,
    pub(super) peer_id: Option<String>,
    pub(super) published: Known,
    pub(super) retry_at: u64,
}

impl Default for AdBook {
    fn default() -> Self {
        Self {
            hysteresis: Hysteresis::default(),
            peer_id: None,
            published: Known::Unknown,
            retry_at: 0,
        }
    }
}

impl AdBook {
    fn nothing_to_withdraw(&self, now: u64) -> bool {
        matches!(self.published, Known::Known(p) if !p.is_some_and(|p| p.is_live_public(now)))
    }
}

#[derive(Default)]
pub(super) struct AnchorAgentState {
    pub(super) status: Arc<AnchorStatus>,
    pub(super) book: tokio::sync::Mutex<AdBook>,
    peer_id: Mutex<Option<String>>,
    followed_sites: Mutex<Vec<(PublicKey, String)>>,
    pub(super) check: Mutex<Option<AbortHandle>>,
    pub(super) refresh: Mutex<Option<AbortHandle>>,
}

impl AnchorAgentState {
    pub(super) fn with_status(status: Arc<AnchorStatus>) -> Self {
        Self {
            status,
            ..Self::default()
        }
    }
}

fn own_published(
    events: Vec<Event>,
    own: &PublicKey,
    peer_id: &str,
    now: u64,
) -> Option<Published> {
    nostr::latest_anchor_ads(events, now)
        .iter()
        .find(|ad| ad.advertiser == *own && ad.peer_id == peer_id)
        .map(Published::from)
}

pub(super) fn holds_by_reporter(
    events: Vec<Event>,
    report_kind: u16,
    site_event_kind: u16,
    now: u64,
) -> HashMap<PublicKey, BTreeSet<SiteKey>> {
    let mut holds: HashMap<PublicKey, BTreeSet<SiteKey>> = HashMap::new();
    for report in nostr::counted_replica_reports(events, report_kind, site_event_kind, now) {
        holds
            .entry(report.reporter)
            .or_default()
            .insert(state::site_key(&report.author.to_hex(), &report.d));
    }
    holds
}

impl<C, N, R> Agent<C, N, R>
where
    C: KuboStore + KuboNode,
    N: Nip05Verify,
    R: ReportRelay + AnchorRelay,
{
    pub(super) fn set_followed_sites(&self, sites: Vec<(PublicKey, String)>) {
        *self.anchors.followed_sites.lock().unwrap() = sites;
    }

    async fn own_peer_id(&self) -> Option<String> {
        if let Some(id) = self.anchors.peer_id.lock().unwrap().clone() {
            return Some(id);
        }
        match self.ipfs.peer_id().await {
            Ok(id) => {
                *self.anchors.peer_id.lock().unwrap() = Some(id.clone());
                Some(id)
            }
            Err(e) => {
                warn!(error = %format!("{e:#}"), "reading our PeerID failed; trying again later");
                None
            }
        }
    }

    // Kubo may have restarted with another repository, so the cached PeerID is read again after any failed call.
    fn forget_peer_id(&self) {
        *self.anchors.peer_id.lock().unwrap() = None;
    }

    async fn check_anchor_connections(&self) {
        let selected = self.anchors.status.selected_ids();
        if selected.is_empty() {
            self.anchors.status.set_connected(None);
            return;
        }
        match self.ipfs.connected_peers().await {
            Ok(peers) => self
                .anchors
                .status
                .set_connected(Some(selected.intersection(&peers).cloned().collect())),
            Err(e) => {
                debug!(error = %format!("{e:#}"), "listing Kubo's connected peers failed");
                self.forget_peer_id();
                self.anchors.status.set_connected(None);
            }
        }
    }

    pub(super) async fn check_anchor_ad(&self) {
        let now = now_secs();
        let enabled = self.config.anchor.advertise;
        let mut book = self.anchors.book.lock().await;
        if !enabled && book.nothing_to_withdraw(now) {
            self.anchors.status.set_advertising(false);
            return;
        }
        let settled = if enabled {
            let observed = match self.ipfs.autonat().await {
                Ok(autonat) => Observed::from(autonat.reachability),
                Err(e) => {
                    debug!(error = %format!("{e:#}"), "reading AutoNAT failed; counting the node as not reachable");
                    self.forget_peer_id();
                    Observed::NotPublic
                }
            };
            book.hysteresis.observe(observed, now)
        } else {
            book.hysteresis.settled()
        };
        self.publish_anchor_ad(&mut book, settled, enabled, now)
            .await;
        self.anchors.status.set_advertising(matches!(
            book.published,
            Known::Known(Some(p)) if p.is_live_public(now)
        ));
    }

    async fn publish_anchor_ad(
        &self,
        book: &mut AdBook,
        settled: Settled,
        enabled: bool,
        now: u64,
    ) {
        if settled == Settled::Undecided && enabled {
            return;
        }
        let Some(peer_id) = self.own_peer_id().await else {
            return;
        };
        if book.peer_id.as_deref() != Some(peer_id.as_str()) {
            book.peer_id = Some(peer_id.clone());
            book.published = Known::Unknown;
        }
        if now < book.retry_at {
            return;
        }
        if book.published == Known::Unknown {
            match self.reporter.fetch_own_anchor_ads(&peer_id).await {
                Ok(events) => {
                    book.published = Known::Known(own_published(events, &self.own, &peer_id, now))
                }
                Err(e) => {
                    warn!(error = %format!("{e:#}"), "fetching our own anchor ad failed; trying again later");
                    book.retry_at = now + AD_RETRY;
                }
            }
        }
        let Some(reachability) = anchor::ad_to_publish(settled, enabled, book.published, now)
        else {
            return;
        };
        let created_at = anchor::next_created_at(book.published, now);
        let expiration = created_at + AD_TTL;
        let ad = nostr::build_anchor_ad_builder(
            &peer_id,
            reachability,
            Timestamp::from_secs(expiration),
        )
        .custom_created_at(Timestamp::from_secs(created_at));
        match self.reporter.send_anchor_ad(ad).await {
            Ok(true) => {
                info!(peer_id = %peer_id, reachability = reachability.token(), "sent anchor ad");
                book.published = Known::Known(Some(Published {
                    reachability,
                    created_at,
                    expiration,
                }));
            }
            Ok(false) => {
                warn!(peer_id = %peer_id, "no relay accepted the anchor ad; will retry");
                book.retry_at = now + AD_RETRY;
            }
            Err(e) => {
                warn!(peer_id = %peer_id, error = format!("{e:#}"), "sending the anchor ad failed; will retry");
                book.retry_at = now + AD_RETRY;
            }
        }
    }

    pub(super) async fn refresh_anchors(&self, targets: &[PublicKey]) {
        self.reselect_anchors(targets).await;
        self.check_anchor_connections().await;
    }

    async fn reselect_anchors(&self, targets: &[PublicKey]) {
        let cap = self.config.anchor.max_peers;
        let selected = if cap == 0 {
            Vec::new()
        } else {
            match self.choose_anchors(targets, cap).await {
                Some(selected) => selected,
                None => return,
            }
        };
        let desired: BTreeSet<String> = selected.iter().map(|s| s.peer_id.clone()).collect();
        let status = &self.anchors.status;
        match anchor::apply_peering(&self.ipfs, &desired, &status.managed()).await {
            Ok(managed) => status.set_managed(managed),
            Err(e) => {
                warn!(error = %format!("{e:#}"), "listing Kubo's peerings failed; applying the anchors on the next poll");
                self.forget_peer_id();
            }
        }
        status.set_selected(selected);
    }

    async fn choose_anchors(&self, targets: &[PublicKey], cap: usize) -> Option<Vec<Selected>> {
        if targets.is_empty() {
            return Some(Vec::new());
        }
        let own_peer_id = self.own_peer_id().await?;
        let events = match self.reporter.fetch_anchor_ads(targets).await {
            Ok(events) => events,
            Err(e) => {
                warn!(error = %format!("{e:#}"), "fetching anchor ads failed; keeping the anchors as they are");
                return None;
            }
        };
        let now = now_secs();
        let ads = anchor::usable_ads(nostr::latest_anchor_ads(events, now), &own_peer_id, now);
        if ads.is_empty() {
            return Some(Vec::new());
        }
        let advertisers: Vec<PublicKey> = ads
            .iter()
            .map(|ad| ad.advertiser)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let (mutual, holds) = tokio::join!(
            self.anchor_mutual(&advertisers),
            self.anchor_holds(&advertisers, now)
        );
        let candidates = anchor::candidates(&ads, &mutual, &holds);
        Some(anchor::select_anchors(
            &candidates,
            &self.anchors.status.selected_ids(),
            cap,
        ))
    }

    async fn anchor_mutual(&self, advertisers: &[PublicKey]) -> HashSet<PublicKey> {
        self.reporter
            .fetch_followers_among(&self.config.nostr.mirror_set, advertisers, self.own)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %format!("{e:#}"), "fetching the advertisers' follow sets failed; treating none as mutual this time");
                HashSet::new()
            })
    }

    async fn anchor_holds(
        &self,
        advertisers: &[PublicKey],
        now: u64,
    ) -> HashMap<PublicKey, BTreeSet<SiteKey>> {
        let site_kind = self.config.nostr.site_event_kind;
        let sites: Vec<Coordinate> = self
            .anchors
            .followed_sites
            .lock()
            .unwrap()
            .iter()
            .map(|(author, d)| nostr::site_coordinate(site_kind, author, d))
            .collect();
        if sites.is_empty() {
            return HashMap::new();
        }
        let report_kind = self.config.nostr.replica_event_kind;
        match self
            .reporter
            .fetch_reports_by(report_kind, &sites, advertisers)
            .await
        {
            Ok(events) => holds_by_reporter(events, report_kind, site_kind, now),
            Err(e) => {
                warn!(error = %format!("{e:#}"), "fetching the advertisers' replica reports failed; choosing anchors without coverage this time");
                HashMap::new()
            }
        }
    }
}

#[cfg(test)]
mod tests;
