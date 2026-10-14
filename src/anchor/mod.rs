use std::collections::BTreeSet;
use std::sync::Mutex;

use crate::ipfs::Reachability;
use crate::nostr::{AdReachability, AnchorAd};

mod peering;
mod select;

pub use peering::{PeeringPlan, apply_peering, plan_peering};
pub use select::{Candidate, Reason, Selected, candidates, select_anchors, usable_ads};

pub const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
pub const PUBLIC_AFTER: u64 = 10 * 60;
pub const NOT_PUBLIC_AFTER: u64 = 20 * 60;
pub const AD_TTL: u64 = 3 * 86_400;
pub const AD_REFRESH: u64 = 86_400;
pub const AD_RETRY: u64 = 10 * 60;
pub const MAX_PEERS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    Public,
    NotPublic,
}

impl From<Reachability> for Observed {
    fn from(r: Reachability) -> Self {
        match r {
            Reachability::Public => Self::Public,
            Reachability::Private | Reachability::Unknown | Reachability::Offline => {
                Self::NotPublic
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Undecided,
    Public,
    NotPublic,
}

#[derive(Debug, Clone, Copy)]
pub struct Hysteresis {
    settled: Settled,
    since: Option<(Observed, u64)>,
}

impl Default for Hysteresis {
    fn default() -> Self {
        Self {
            settled: Settled::Undecided,
            since: None,
        }
    }
}

impl Hysteresis {
    pub fn observe(&mut self, observed: Observed, now: u64) -> Settled {
        let since = match self.since {
            Some((last, at)) if last == observed => at,
            _ => now,
        };
        self.since = Some((observed, since));
        let held = now.saturating_sub(since);
        match observed {
            Observed::Public if held >= PUBLIC_AFTER => self.settled = Settled::Public,
            Observed::NotPublic if held >= NOT_PUBLIC_AFTER => self.settled = Settled::NotPublic,
            _ => {}
        }
        self.settled
    }

    pub fn settled(&self) -> Settled {
        self.settled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    pub reachability: AdReachability,
    pub created_at: u64,
    pub expiration: u64,
}

impl Published {
    pub fn is_live_public(&self, now: u64) -> bool {
        self.reachability == AdReachability::Public && self.expiration > now
    }
}

impl From<&AnchorAd> for Published {
    fn from(ad: &AnchorAd) -> Self {
        Self {
            reachability: ad.reachability,
            created_at: ad.created_at,
            expiration: ad.expiration,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Known {
    Unknown,
    Known(Option<Published>),
}

pub fn ad_to_publish(
    settled: Settled,
    enabled: bool,
    published: Known,
    now: u64,
) -> Option<AdReachability> {
    if enabled && settled == Settled::Public {
        let fresh = matches!(
            published,
            Known::Known(Some(p))
                if p.is_live_public(now) && now < p.created_at.saturating_add(AD_REFRESH)
        );
        return (!fresh).then_some(AdReachability::Public);
    }
    let withdraw = !enabled || settled == Settled::NotPublic;
    match published {
        Known::Known(Some(p)) if withdraw && p.is_live_public(now) => Some(AdReachability::Private),
        _ => None,
    }
}

pub fn next_created_at(published: Known, now: u64) -> u64 {
    match published {
        Known::Known(Some(p)) => now.max(p.created_at.saturating_add(1)),
        _ => now,
    }
}

#[derive(Debug, Default)]
struct StatusInner {
    advertising: bool,
    selected: Vec<Selected>,
    managed: BTreeSet<String>,
    connected: Option<BTreeSet<String>>,
}

// Kept by the dashboard state so the peerings SWING added are still known after the agent restarts.
#[derive(Debug, Default)]
pub struct AnchorStatus {
    inner: Mutex<StatusInner>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub advertising: bool,
    pub selected: Vec<Selected>,
    pub connected: Option<BTreeSet<String>>,
}

impl AnchorStatus {
    pub fn set_advertising(&self, advertising: bool) {
        self.inner.lock().unwrap().advertising = advertising;
    }

    pub fn set_selected(&self, selected: Vec<Selected>) {
        self.inner.lock().unwrap().selected = selected;
    }

    pub fn selected_ids(&self) -> BTreeSet<String> {
        self.inner
            .lock()
            .unwrap()
            .selected
            .iter()
            .map(|s| s.peer_id.clone())
            .collect()
    }

    pub fn managed(&self) -> BTreeSet<String> {
        self.inner.lock().unwrap().managed.clone()
    }

    pub fn set_managed(&self, managed: BTreeSet<String>) {
        self.inner.lock().unwrap().managed = managed;
    }

    pub fn has_selected(&self) -> bool {
        !self.inner.lock().unwrap().selected.is_empty()
    }

    pub fn set_connected(&self, connected: Option<BTreeSet<String>>) {
        self.inner.lock().unwrap().connected = connected;
    }

    pub fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock().unwrap();
        Snapshot {
            advertising: inner.advertising,
            selected: inner.selected.clone(),
            connected: inner.connected.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_is_settled_only_after_it_holds_for_a_while() {
        let mut h = Hysteresis::default();
        assert_eq!(h.observe(Observed::Public, 0), Settled::Undecided);
        assert_eq!(
            h.observe(Observed::Public, PUBLIC_AFTER - 1),
            Settled::Undecided
        );
        assert_eq!(h.observe(Observed::Public, PUBLIC_AFTER), Settled::Public);
    }

    #[test]
    fn a_flap_restarts_the_clock_and_keeps_the_settled_state() {
        let mut h = Hysteresis::default();
        h.observe(Observed::Public, 0);
        assert_eq!(h.observe(Observed::Public, PUBLIC_AFTER), Settled::Public);
        let t = PUBLIC_AFTER + 60;
        assert_eq!(h.observe(Observed::NotPublic, t), Settled::Public);
        assert_eq!(
            h.observe(Observed::NotPublic, t + NOT_PUBLIC_AFTER - 1),
            Settled::Public
        );
        assert_eq!(
            h.observe(Observed::Public, t + NOT_PUBLIC_AFTER),
            Settled::Public
        );
        let t2 = t + NOT_PUBLIC_AFTER + 60;
        assert_eq!(h.observe(Observed::NotPublic, t2), Settled::Public);
        assert_eq!(
            h.observe(Observed::NotPublic, t2 + NOT_PUBLIC_AFTER),
            Settled::NotPublic
        );
        assert_eq!(
            h.observe(Observed::Public, t2 + NOT_PUBLIC_AFTER + 1),
            Settled::NotPublic
        );
    }

    #[test]
    fn unknown_and_offline_count_as_not_public() {
        for r in [
            Reachability::Private,
            Reachability::Unknown,
            Reachability::Offline,
        ] {
            assert_eq!(Observed::from(r), Observed::NotPublic);
        }
        assert_eq!(Observed::from(Reachability::Public), Observed::Public);
        let mut h = Hysteresis::default();
        h.observe(Observed::NotPublic, 0);
        assert_eq!(
            h.observe(Observed::NotPublic, NOT_PUBLIC_AFTER),
            Settled::NotPublic
        );
    }

    fn published(reachability: AdReachability, created_at: u64) -> Known {
        Known::Known(Some(Published {
            reachability,
            created_at,
            expiration: created_at + AD_TTL,
        }))
    }

    #[test]
    fn a_settled_public_node_advertises_and_refreshes_daily() {
        let now = 10_000_000;
        assert_eq!(
            ad_to_publish(Settled::Public, true, Known::Known(None), now),
            Some(AdReachability::Public)
        );
        assert_eq!(
            ad_to_publish(Settled::Public, true, Known::Unknown, now),
            Some(AdReachability::Public)
        );
        assert_eq!(
            ad_to_publish(
                Settled::Public,
                true,
                published(AdReachability::Public, now - AD_REFRESH + 1),
                now
            ),
            None
        );
        assert_eq!(
            ad_to_publish(
                Settled::Public,
                true,
                published(AdReachability::Public, now - AD_REFRESH),
                now
            ),
            Some(AdReachability::Public)
        );
        assert_eq!(
            ad_to_publish(
                Settled::Public,
                true,
                published(AdReachability::Private, now - 10),
                now
            ),
            Some(AdReachability::Public)
        );
    }

    #[test]
    fn a_public_ad_is_withdrawn_when_not_public_or_disabled() {
        let now = 10_000_000;
        let live = published(AdReachability::Public, now - 100);
        assert_eq!(
            ad_to_publish(Settled::NotPublic, true, live, now),
            Some(AdReachability::Private)
        );
        assert_eq!(
            ad_to_publish(Settled::Public, false, live, now),
            Some(AdReachability::Private)
        );
        assert_eq!(
            ad_to_publish(Settled::Undecided, false, live, now),
            Some(AdReachability::Private)
        );
        assert_eq!(ad_to_publish(Settled::Undecided, true, live, now), None);
        assert_eq!(
            ad_to_publish(Settled::NotPublic, true, Known::Unknown, now),
            None
        );
        assert_eq!(
            ad_to_publish(Settled::NotPublic, true, Known::Known(None), now),
            None
        );
        assert_eq!(
            ad_to_publish(
                Settled::NotPublic,
                true,
                published(AdReachability::Private, now - 100),
                now
            ),
            None
        );
        assert_eq!(
            ad_to_publish(
                Settled::NotPublic,
                true,
                published(AdReachability::Public, now - AD_TTL),
                now
            ),
            None
        );
    }

    #[test]
    fn a_new_ad_is_dated_after_the_one_it_replaces() {
        assert_eq!(next_created_at(Known::Unknown, 100), 100);
        assert_eq!(
            next_created_at(published(AdReachability::Public, 100), 100),
            101
        );
        assert_eq!(
            next_created_at(published(AdReachability::Public, 50), 100),
            100
        );
    }
}
