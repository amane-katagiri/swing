use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashSet};

use nostr_sdk::prelude::*;
use tracing::{debug, warn};

use super::RelayClient;
use super::fetch::{Folded, capped_limit, fold_per_relay};
use crate::nostr::{Withdrawals, budget, keep_newest_site, site_coordinate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteAnswer {
    Unanswered,
    Missing,
    Found(Event),
}

pub struct RelayAnswers {
    pub relay: RelayUrl,
    pub sites: Vec<SiteAnswer>,
}

pub struct DeletionAnswers {
    pub withdrawals: Withdrawals,
    pub relays: Vec<(RelayUrl, Vec<bool>)>,
}

impl DeletionAnswers {
    pub fn answered(&self, relay: &RelayUrl, site: usize) -> bool {
        self.relays
            .iter()
            .find(|(url, _)| url == relay)
            .and_then(|(_, sites)| sites.get(site).copied())
            .unwrap_or(false)
    }
}

pub(super) struct DeletionUnit {
    pub(super) author: PublicKey,
    pub(super) sites: Vec<usize>,
    pub(super) ids: HashSet<String>,
    pub(super) coordinates: HashSet<String>,
    pub(super) limit: usize,
    pub(super) filters: [Filter; 2],
}

pub(super) const UNITS_PER_REQ: usize = budget::AUTHORS_PER_SPLIT_REQ / 2;

fn deletion_unit(
    site_event_kind: u16,
    author: PublicKey,
    held: &[&Event],
    sites: &[usize],
) -> DeletionUnit {
    let limit = capped_limit(sites.len(), 2).min(budget::MAX_DELETIONS_PER_FILTER);
    let coordinates: Vec<Coordinate> = sites
        .iter()
        .filter_map(|&i| held[i].tags.identifier())
        .map(|d| site_coordinate(site_event_kind, &author, &d))
        .collect();
    let ids: Vec<EventId> = sites.iter().map(|&i| held[i].id).collect();
    let deletions = Filter::new()
        .kind(Kind::EventDeletion)
        .author(author)
        .limit(limit);
    DeletionUnit {
        author,
        sites: sites.to_vec(),
        ids: ids.iter().map(EventId::to_hex).collect(),
        coordinates: coordinates.iter().map(Coordinate::to_string).collect(),
        limit,
        filters: [
            deletions.clone().coordinates(&coordinates),
            deletions.events(ids),
        ],
    }
}

// A limit shared by several authors would let one author's deletions naming another's site crowd out that author's own.
pub(super) fn deletion_units(site_event_kind: u16, held: &[&Event]) -> Vec<DeletionUnit> {
    let mut by_author: BTreeMap<PublicKey, Vec<usize>> = BTreeMap::new();
    for (i, event) in held.iter().enumerate() {
        by_author.entry(event.pubkey).or_default().push(i);
    }
    by_author
        .into_iter()
        .flat_map(|(author, sites)| {
            sites
                .chunks(budget::COORDINATES_PER_FILTER)
                .map(|chunk| deletion_unit(site_event_kind, author, held, chunk))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[derive(Default)]
pub(super) struct Returned {
    count: usize,
    newest: BinaryHeap<Reverse<u64>>,
}

impl Returned {
    pub(super) fn add(&mut self, created_at: u64, limit: usize) {
        self.count += 1;
        self.newest.push(Reverse(created_at));
        if self.newest.len() > limit {
            self.newest.pop();
        }
    }

    pub(super) fn cutoff(&self, limit: usize) -> Option<u64> {
        (self.count >= limit).then(|| self.newest.peek().map_or(0, |Reverse(at)| *at))
    }
}

pub(super) fn answered_past(cutoffs: [Option<u64>; 2], created_at: u64) -> bool {
    cutoffs
        .iter()
        .all(|cutoff| cutoff.is_none_or(|cutoff| created_at > cutoff))
}

struct SitesReq {
    first: usize,
    newest: Vec<Option<Event>>,
}

struct DeletionReq {
    first: usize,
    returned: Vec<[Returned; 2]>,
    seen: HashSet<EventId>,
    withdrawals: Withdrawals,
}

impl DeletionReq {
    fn note(&mut self, units: &[DeletionUnit], event: Event, now: u64) {
        if event.kind != Kind::EventDeletion || !self.seen.insert(event.id) {
            return;
        }
        let created_at = event.created_at.as_secs();
        for (k, [by_address, by_id]) in self.returned.iter_mut().enumerate() {
            let unit = &units[self.first + k];
            if unit.author != event.pubkey {
                continue;
            }
            let found = self
                .withdrawals
                .note(&event, &unit.ids, &unit.coordinates, now);
            if found.by_address {
                by_address.add(created_at, unit.limit);
            }
            if found.by_id {
                by_id.add(created_at, unit.limit);
            }
        }
    }
}

fn answered_sites(
    relay: &RelayUrl,
    units: &[DeletionUnit],
    held: &[&Event],
    requests: &[Folded<DeletionReq>],
) -> Vec<bool> {
    let mut answered = vec![false; held.len()];
    for request in requests {
        let req = &request.value;
        for (k, returned) in req.returned.iter().enumerate() {
            let unit = &units[req.first + k];
            let cutoffs = returned.each_ref().map(|r| r.cutoff(unit.limit));
            if let Some(cutoff) = cutoffs.iter().flatten().max() {
                warn!(relay = %relay, pubkey = %unit.author, cutoff, "deletions by the author reach the request limit; the author's held site events at or before the cutoff are not sent to this relay this time");
            }
            if !request.complete {
                continue;
            }
            for &i in &unit.sites {
                let created_at = held[i].created_at.as_secs();
                if answered_past(cutoffs, created_at) {
                    answered[i] = true;
                } else {
                    debug!(relay = %relay, event_id = %held[i].id, created_at, "the held site event is not newer than the cutoff of the deletions returned; whether the author deleted it is unknown on this relay");
                }
            }
        }
    }
    answered
}

impl RelayClient {
    // Kept per relay, unlike the other fetches, because a held site is sent again only to the relays that lack it.
    pub async fn fetch_sites_per_relay(
        &self,
        site_event_kind: u16,
        sites: &[(PublicKey, String)],
    ) -> Vec<RelayAnswers> {
        let kind = Kind::Custom(site_event_kind);
        let requests: Vec<Vec<Filter>> = sites
            .chunks(budget::AUTHORS_PER_SPLIT_REQ)
            .map(|batch| {
                batch
                    .iter()
                    .map(|(author, d)| {
                        Filter::new()
                            .kind(kind)
                            .author(*author)
                            .identifier(d)
                            .limit(capped_limit(1, 2))
                    })
                    .collect()
            })
            .collect();
        let now = Timestamp::now().as_secs();
        let folded = fold_per_relay(
            self.read_relays().await,
            &requests,
            |i| {
                let first = i * budget::AUTHORS_PER_SPLIT_REQ;
                let len = sites[first..].len().min(budget::AUTHORS_PER_SPLIT_REQ);
                SitesReq {
                    first,
                    newest: vec![None; len],
                }
            },
            |req: &mut SitesReq, event| {
                let batch = &sites[req.first..req.first + req.newest.len()];
                let Some(j) = batch.iter().position(|(author, d)| {
                    *author == event.pubkey && event.tags.identifier().as_deref() == Some(d)
                }) else {
                    return;
                };
                let (author, d) = &batch[j];
                keep_newest_site(&mut req.newest[j], event, author, d, site_event_kind, now);
            },
        )
        .await;
        folded
            .into_iter()
            .map(|(relay, requests)| {
                let mut answers = vec![SiteAnswer::Unanswered; sites.len()];
                for request in requests.into_iter().filter(|r| r.complete) {
                    let req = request.value;
                    for (j, newest) in req.newest.into_iter().enumerate() {
                        answers[req.first + j] =
                            newest.map_or(SiteAnswer::Missing, SiteAnswer::Found);
                    }
                }
                RelayAnswers {
                    relay,
                    sites: answers,
                }
            })
            .collect()
    }

    pub async fn fetch_site_deletions(
        &self,
        site_event_kind: u16,
        held: &[&Event],
    ) -> DeletionAnswers {
        let units = deletion_units(site_event_kind, held);
        let requests: Vec<Vec<Filter>> = units
            .chunks(UNITS_PER_REQ)
            .map(|chunk| chunk.iter().flat_map(|u| u.filters.clone()).collect())
            .collect();
        let now = Timestamp::now().as_secs();
        let units = &units;
        let folded = fold_per_relay(
            self.read_relays().await,
            &requests,
            |i| {
                let first = i * UNITS_PER_REQ;
                let len = units[first..].len().min(UNITS_PER_REQ);
                DeletionReq {
                    first,
                    returned: (0..len).map(|_| Default::default()).collect(),
                    seen: HashSet::new(),
                    withdrawals: Withdrawals::default(),
                }
            },
            |req: &mut DeletionReq, event| req.note(units, event, now),
        )
        .await;
        let mut withdrawals = Withdrawals::default();
        let relays = folded
            .into_iter()
            .map(|(relay, requests)| {
                let answered = answered_sites(&relay, units, held, &requests);
                for request in requests {
                    withdrawals.merge(request.value.withdrawals);
                }
                (relay, answered)
            })
            .collect();
        DeletionAnswers {
            withdrawals,
            relays,
        }
    }
}
