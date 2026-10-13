use std::collections::{HashMap, HashSet};

use nostr_sdk::prelude::*;

use super::{RelayAnswers, is_newer_replaceable, parse_site_event, plausible_at, site_coordinate};

pub fn is_saved_site_event(
    event: &Event,
    author: &PublicKey,
    d: &str,
    site_event_kind: u16,
) -> bool {
    event.pubkey == *author
        && parse_site_event(event, site_event_kind).is_ok_and(|site| site.d == d)
        && event.verify().is_ok()
}

pub fn keep_newest_site(
    slot: &mut Option<Event>,
    event: Event,
    author: &PublicKey,
    d: &str,
    site_event_kind: u16,
    now: u64,
) {
    if event.pubkey != *author
        || !plausible_at(event.created_at.as_secs(), now)
        || !parse_site_event(&event, site_event_kind).is_ok_and(|site| site.d == d)
    {
        return;
    }
    if slot
        .as_ref()
        .is_none_or(|current| is_newer_replaceable(&event, current))
    {
        *slot = Some(event);
    }
}

fn relay_lacks(held: &Event, newest: Option<&Event>) -> bool {
    newest.is_none_or(|found| found.id != held.id && is_newer_replaceable(held, found))
}

pub fn plan_site_resends(held: &[Event], answers: &[RelayAnswers]) -> Vec<Vec<RelayUrl>> {
    held.iter()
        .enumerate()
        .map(|(i, event)| {
            answers
                .iter()
                .filter(|a| {
                    a.sites
                        .get(i)
                        .and_then(Option::as_ref)
                        .is_some_and(|newest| relay_lacks(event, newest.as_ref()))
                })
                .map(|a| a.relay.clone())
                .collect()
        })
        .collect()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeletionMatch {
    pub by_id: bool,
    pub by_address: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Withdrawals {
    ids: HashSet<(PublicKey, String)>,
    addresses: HashMap<(PublicKey, String), u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Withdrawal {
    pub by_id: bool,
    pub through: Option<u64>,
}

impl Withdrawals {
    // Only the tags naming what was asked about are indexed, so a deletion with hundreds of tags costs no more than one with a single tag.
    pub fn note(
        &mut self,
        deletion: &Event,
        ids: &HashSet<String>,
        coordinates: &HashSet<String>,
        now: u64,
    ) -> DeletionMatch {
        let mut found = DeletionMatch::default();
        if deletion.kind != Kind::EventDeletion {
            return found;
        }
        let plausible = plausible_at(deletion.created_at.as_secs(), now);
        for tag in deletion.tags.iter() {
            let Some(value) = tag.content() else {
                continue;
            };
            let kind = tag.kind();
            if kind == "e" && ids.contains(value) {
                found.by_id = true;
                if plausible {
                    self.ids.insert((deletion.pubkey, value.to_string()));
                }
            } else if kind == "a" && coordinates.contains(value) {
                found.by_address = true;
                if plausible {
                    let at = self
                        .addresses
                        .entry((deletion.pubkey, value.to_string()))
                        .or_insert(0);
                    *at = (*at).max(deletion.created_at.as_secs());
                }
            }
        }
        found
    }

    pub fn merge(&mut self, other: Withdrawals) {
        self.ids.extend(other.ids);
        for (key, created_at) in other.addresses {
            let at = self.addresses.entry(key).or_insert(created_at);
            *at = (*at).max(created_at);
        }
    }

    pub fn of(&self, held: &Event, site_event_kind: u16) -> Option<Withdrawal> {
        let by_id = self.ids.contains(&(held.pubkey, held.id.to_hex()));
        let through = held
            .tags
            .identifier()
            .and_then(|d| {
                let coordinate = site_coordinate(site_event_kind, &held.pubkey, &d).to_string();
                self.addresses.get(&(held.pubkey, coordinate)).copied()
            })
            .filter(|at| held.created_at.as_secs() <= *at);
        (by_id || through.is_some()).then_some(Withdrawal { by_id, through })
    }
}

#[cfg(test)]
mod tests;
