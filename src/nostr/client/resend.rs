use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use super::RelayClient;
use super::fetch::{capped_limit, fold_per_relay};
use crate::nostr::{Withdrawals, budget, keep_newest_site, site_coordinate};

pub struct RelayAnswers {
    pub relay: RelayUrl,
    pub sites: Vec<Option<Option<Event>>>,
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
}

impl DeletionUnit {
    pub(super) fn filters(&self) -> [Filter; 2] {
        let coordinates: Vec<Coordinate> = self
            .coordinates
            .iter()
            .filter_map(|c| Coordinate::parse(c).ok())
            .collect();
        let ids = self.ids.iter().filter_map(|id| EventId::from_hex(id).ok());
        [
            Filter::new()
                .kind(Kind::EventDeletion)
                .author(self.author)
                .coordinates(&coordinates)
                .limit(self.limit),
            Filter::new()
                .kind(Kind::EventDeletion)
                .author(self.author)
                .events(ids)
                .limit(self.limit),
        ]
    }
}

pub(super) const UNITS_PER_REQ: usize = budget::AUTHORS_PER_SPLIT_REQ / 2;

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
                .map(|chunk| DeletionUnit {
                    author,
                    sites: chunk.to_vec(),
                    ids: chunk.iter().map(|&i| held[i].id.to_hex()).collect(),
                    coordinates: chunk
                        .iter()
                        .filter_map(|&i| {
                            held[i]
                                .tags
                                .identifier()
                                .map(|d| site_coordinate(site_event_kind, &author, &d).to_string())
                        })
                        .collect(),
                    limit: capped_limit(chunk.len(), 2),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

struct DeletionReq {
    first: usize,
    counts: Vec<(usize, usize)>,
    seen: HashSet<EventId>,
    withdrawals: Withdrawals,
}

impl RelayClient {
    async fn read_relays(&self) -> Vec<(RelayUrl, Relay)> {
        self.client
            .relays()
            .with_capabilities(RelayCapabilities::READ)
            .await
            .into_iter()
            .collect()
    }

    // Kept per relay, unlike the other fetches, because a held site is sent again only to the relays that lack it.
    pub async fn fetch_sites_per_relay(
        &self,
        site_event_kind: u16,
        sites: &[(PublicKey, String)],
    ) -> Vec<RelayAnswers> {
        let kind = Kind::Custom(site_event_kind);
        let batches: Vec<&[(PublicKey, String)]> =
            sites.chunks(budget::AUTHORS_PER_SPLIT_REQ).collect();
        let requests: Vec<Vec<Filter>> = batches
            .iter()
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
            |i| (i, vec![None; batches[i].len()]),
            |(i, slots): &mut (usize, Vec<Option<Event>>), event| {
                let batch = batches[*i];
                let Some(j) = batch.iter().position(|(author, d)| {
                    *author == event.pubkey && event.tags.identifier().as_deref() == Some(d)
                }) else {
                    return;
                };
                let (author, d) = &batch[j];
                keep_newest_site(&mut slots[j], event, author, d, site_event_kind, now);
            },
        )
        .await;
        folded
            .into_iter()
            .map(|(relay, requests)| {
                let mut answers = vec![None; sites.len()];
                for (i, request) in requests.into_iter().enumerate() {
                    if !request.complete {
                        continue;
                    }
                    let start = i * budget::AUTHORS_PER_SPLIT_REQ;
                    for (j, newest) in request.value.1.into_iter().enumerate() {
                        answers[start + j] = Some(newest);
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
            .map(|chunk| chunk.iter().flat_map(DeletionUnit::filters).collect())
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
                    counts: vec![(0, 0); len],
                    seen: HashSet::new(),
                    withdrawals: Withdrawals::default(),
                }
            },
            |req: &mut DeletionReq, event| {
                if event.kind != Kind::EventDeletion || !req.seen.insert(event.id) {
                    return;
                }
                for (k, count) in req.counts.iter_mut().enumerate() {
                    let unit = &units[req.first + k];
                    if unit.author != event.pubkey {
                        continue;
                    }
                    let found = req
                        .withdrawals
                        .note(&event, &unit.ids, &unit.coordinates, now);
                    count.0 += usize::from(found.by_address);
                    count.1 += usize::from(found.by_id);
                }
            },
        )
        .await;
        let mut withdrawals = Withdrawals::default();
        let relays = folded
            .into_iter()
            .map(|(relay, requests)| {
                let mut answered = vec![false; held.len()];
                for request in requests {
                    let req = request.value;
                    for (k, (by_address, by_id)) in req.counts.iter().enumerate() {
                        let unit = &units[req.first + k];
                        let full = *by_address >= unit.limit || *by_id >= unit.limit;
                        if full {
                            tracing::warn!(relay = %relay, pubkey = %unit.author, "deletions by the author reach the request limit; not sending the author's held site events to this relay this time");
                        }
                        if request.complete && !full {
                            for &i in &unit.sites {
                                answered[i] = true;
                            }
                        }
                    }
                    withdrawals.merge(req.withdrawals);
                }
                (relay, answered)
            })
            .collect();
        DeletionAnswers {
            withdrawals,
            relays,
        }
    }

    pub async fn publish_to(
        &self,
        event: &Event,
        relays: &[RelayUrl],
    ) -> Result<Output<EventId, EventSendStatus, String>> {
        self.client
            .send_event(event)
            .to(relays.iter().map(|u| u.as_str()))
            .await
            .context("sending event to relays")
    }
}
