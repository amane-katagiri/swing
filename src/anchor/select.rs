use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use nostr_sdk::prelude::PublicKey;

use crate::nostr::AnchorAd;
use crate::state::SiteKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub peer_id: String,
    pub advertiser: PublicKey,
    pub mutual: bool,
    pub holds: Arc<BTreeSet<SiteKey>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Mutual,
    Coverage,
}

impl Reason {
    pub fn token(self) -> &'static str {
        match self {
            Self::Mutual => "mutual",
            Self::Coverage => "coverage",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub peer_id: String,
    pub advertiser: PublicKey,
    pub reason: Reason,
    pub sites: usize,
}

impl Selected {
    fn of(c: &Candidate, reason: Reason) -> Self {
        Self {
            peer_id: c.peer_id.clone(),
            advertiser: c.advertiser,
            reason,
            sites: c.holds.len(),
        }
    }
}

pub fn usable_ads(ads: Vec<AnchorAd>, own_peer_id: &str, now: u64) -> Vec<AnchorAd> {
    ads.into_iter()
        .filter(|ad| ad.usable_at(now) && ad.peer_id != own_peer_id)
        .collect()
}

// Takes ads already passed through `usable_ads`. Several advertisers may name one PeerID; a mutual advertiser wins, then the lowest key, so the choice is stable.
pub fn candidates(
    ads: &[AnchorAd],
    mutual: &HashSet<PublicKey>,
    holds: &HashMap<PublicKey, BTreeSet<SiteKey>>,
) -> Vec<Candidate> {
    let mut by_peer: BTreeMap<&str, &AnchorAd> = BTreeMap::new();
    let rank = |a: &AnchorAd| (!mutual.contains(&a.advertiser), a.advertiser);
    for ad in ads {
        by_peer
            .entry(ad.peer_id.as_str())
            .and_modify(|cur| {
                if rank(ad) < rank(cur) {
                    *cur = ad;
                }
            })
            .or_insert(ad);
    }
    let mut shared: HashMap<PublicKey, Arc<BTreeSet<SiteKey>>> = HashMap::new();
    by_peer
        .into_values()
        .map(|ad| Candidate {
            peer_id: ad.peer_id.clone(),
            advertiser: ad.advertiser,
            mutual: mutual.contains(&ad.advertiser),
            holds: Arc::clone(shared.entry(ad.advertiser).or_insert_with(|| {
                Arc::new(holds.get(&ad.advertiser).cloned().unwrap_or_default())
            })),
        })
        .collect()
}

// The coverage phase compares advertisers rather than PeerIDs: every PeerID of one advertiser holds the same sites, so a second one never adds coverage.
pub fn select_anchors(
    candidates: &[Candidate],
    previous: &BTreeSet<String>,
    cap: usize,
) -> Vec<Selected> {
    let mut chosen: Vec<Selected> = Vec::new();
    let mut covered: BTreeSet<&SiteKey> = BTreeSet::new();
    let fresh = |c: &Candidate| !previous.contains(&c.peer_id);

    let mut mutual: Vec<&Candidate> = candidates.iter().filter(|c| c.mutual).collect();
    mutual.sort_by_key(|&c| (fresh(c), Reverse(c.holds.len()), c.peer_id.as_str()));
    let mut advertisers: HashSet<PublicKey> = HashSet::new();
    for c in mutual {
        if chosen.len() >= cap {
            break;
        }
        if !advertisers.insert(c.advertiser) {
            continue;
        }
        covered.extend(c.holds.iter());
        chosen.push(Selected::of(c, Reason::Mutual));
    }

    let mut by_advertiser: BTreeMap<PublicKey, &Candidate> = BTreeMap::new();
    for c in candidates
        .iter()
        .filter(|c| !advertisers.contains(&c.advertiser))
    {
        by_advertiser
            .entry(c.advertiser)
            .and_modify(|cur| {
                if (fresh(c), c.peer_id.as_str()) < (fresh(cur), cur.peer_id.as_str()) {
                    *cur = c;
                }
            })
            .or_insert(c);
    }
    while chosen.len() < cap {
        let best = by_advertiser
            .values()
            .map(|&c| (c.holds.iter().filter(|s| !covered.contains(s)).count(), c))
            .filter(|(gain, _)| *gain > 0)
            .min_by_key(|&(gain, c)| (Reverse(gain), fresh(c), c.peer_id.as_str()));
        let Some((_, c)) = best else {
            break;
        };
        by_advertiser.remove(&c.advertiser);
        covered.extend(c.holds.iter());
        chosen.push(Selected::of(c, Reason::Coverage));
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nostr::AdReachability;
    use crate::test_support::keys;

    fn cand(peer: &str, advertiser: PublicKey, mutual: bool, holds: &[&str]) -> Candidate {
        Candidate {
            peer_id: peer.to_string(),
            advertiser,
            mutual,
            holds: Arc::new(holds.iter().map(|s| s.to_string()).collect()),
        }
    }

    fn ids(selected: &[Selected]) -> Vec<(&str, Reason, usize)> {
        selected
            .iter()
            .map(|s| (s.peer_id.as_str(), s.reason, s.sites))
            .collect()
    }

    #[test]
    fn mutual_advertisers_come_first_then_greedy_coverage() {
        let (m, a, b, c) = (
            keys().public_key(),
            keys().public_key(),
            keys().public_key(),
            keys().public_key(),
        );
        let cands = vec![
            cand("pa", a, false, &["s1", "s2", "s3"]),
            cand("pb", b, false, &["s3", "s4"]),
            cand("pc", c, false, &["s1", "s2"]),
            cand("pm", m, true, &["s1"]),
        ];
        let picked = select_anchors(&cands, &BTreeSet::new(), 4);
        assert_eq!(
            ids(&picked),
            [
                ("pm", Reason::Mutual, 1),
                ("pa", Reason::Coverage, 3),
                ("pb", Reason::Coverage, 2),
            ]
        );
    }

    #[test]
    fn the_cap_limits_both_phases_and_zero_selects_nothing() {
        let cands: Vec<Candidate> = (0..6)
            .map(|i| {
                cand(
                    &format!("p{i}"),
                    keys().public_key(),
                    i < 3,
                    &[&format!("s{i}")],
                )
            })
            .collect();
        assert_eq!(select_anchors(&cands, &BTreeSet::new(), 2).len(), 2);
        assert!(
            select_anchors(&cands, &BTreeSet::new(), 2)
                .iter()
                .all(|s| s.reason == Reason::Mutual)
        );
        assert!(select_anchors(&cands, &BTreeSet::new(), 0).is_empty());
        assert_eq!(select_anchors(&cands, &BTreeSet::new(), 10).len(), 6);
    }

    #[test]
    fn anchors_that_add_no_coverage_are_left_out() {
        let a = keys().public_key();
        let cands = vec![
            cand("p1", a, false, &["s1"]),
            cand("p2", a, false, &["s1"]),
            cand("p3", keys().public_key(), false, &[]),
        ];
        assert_eq!(
            ids(&select_anchors(&cands, &BTreeSet::new(), 4)),
            [("p1", Reason::Coverage, 1)]
        );
    }

    #[test]
    fn a_mutual_advertiser_takes_one_slot_in_the_mutual_phase() {
        let m = keys().public_key();
        let cands = vec![cand("p1", m, true, &["s1"]), cand("p2", m, true, &["s1"])];
        assert_eq!(
            ids(&select_anchors(&cands, &BTreeSet::new(), 4)),
            [("p1", Reason::Mutual, 1)]
        );
    }

    #[test]
    fn ties_keep_previous_choices_then_go_by_peer_id() {
        let cands = vec![
            cand("pa", keys().public_key(), false, &["s1"]),
            cand("pb", keys().public_key(), false, &["s1"]),
            cand("ma", keys().public_key(), true, &[]),
            cand("mb", keys().public_key(), true, &[]),
        ];
        let fresh = select_anchors(&cands, &BTreeSet::new(), 2);
        assert_eq!(
            ids(&fresh),
            [("ma", Reason::Mutual, 0), ("mb", Reason::Mutual, 0)]
        );
        let fresh = select_anchors(&cands, &BTreeSet::new(), 3);
        assert_eq!(fresh[2].peer_id, "pa");

        let previous = BTreeSet::from(["pb".to_string(), "mb".to_string()]);
        let kept = select_anchors(&cands, &previous, 3);
        assert_eq!(
            ids(&kept),
            [
                ("mb", Reason::Mutual, 0),
                ("ma", Reason::Mutual, 0),
                ("pb", Reason::Coverage, 1)
            ]
        );
        let reversed: Vec<Candidate> = cands.iter().rev().cloned().collect();
        assert_eq!(select_anchors(&reversed, &previous, 3), kept);
    }

    fn ad(advertiser: PublicKey, peer: &str, reach: AdReachability, expiration: u64) -> AnchorAd {
        AnchorAd {
            advertiser,
            peer_id: peer.to_string(),
            reachability: reach,
            created_at: 100,
            expiration,
        }
    }

    #[test]
    fn candidates_skip_own_private_and_expired_ads_and_merge_shared_peer_ids() {
        let (a, b, m) = (
            keys().public_key(),
            keys().public_key(),
            keys().public_key(),
        );
        let ads = vec![
            ad(a, "own", AdReachability::Public, 1000),
            ad(a, "private", AdReachability::Private, 1000),
            ad(a, "expired", AdReachability::Public, 200),
            ad(a, "shared", AdReachability::Public, 1000),
            ad(m, "shared", AdReachability::Public, 1000),
            ad(b, "solo", AdReachability::Public, 1000),
        ];
        let mutual = HashSet::from([m]);
        let holds = HashMap::from([
            (m, BTreeSet::from(["s1".to_string()])),
            (b, BTreeSet::from(["s2".to_string()])),
        ]);
        let got = candidates(&usable_ads(ads, "own", 300), &mutual, &holds);
        let twice = candidates(
            &[
                ad(b, "solo", AdReachability::Public, 1000),
                ad(b, "solo2", AdReachability::Public, 1000),
            ],
            &mutual,
            &holds,
        );
        assert!(Arc::ptr_eq(&twice[0].holds, &twice[1].holds));
        assert_eq!(
            got,
            vec![
                cand("shared", m, true, &["s1"]),
                cand("solo", b, false, &["s2"]),
            ]
        );
    }
}
