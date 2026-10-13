use std::collections::{BTreeSet, HashMap, HashSet};

use nostr_sdk::prelude::*;

use crate::state::Withdrawn;

use super::{
    MAX_FUTURE_SKEW, RelayAnswers, SiteAnswer, SiteEvent, is_newer_replaceable, parse_site_event,
    plausible_at, site_coordinate,
};

fn site_of(event: &Event, author: &PublicKey, d: &str, site_event_kind: u16) -> Option<SiteEvent> {
    if event.pubkey != *author {
        return None;
    }
    parse_site_event(event, site_event_kind)
        .ok()
        .filter(|site| site.d == d)
}

pub fn saved_site_event(
    event: &Event,
    author: &PublicKey,
    d: &str,
    site_event_kind: u16,
) -> Option<SiteEvent> {
    site_of(event, author, d, site_event_kind).filter(|_| event.verify().is_ok())
}

pub fn keep_newest_site(
    slot: &mut Option<Event>,
    event: Event,
    author: &PublicKey,
    d: &str,
    site_event_kind: u16,
    now: u64,
) {
    if !plausible_at(event.created_at.as_secs(), now)
        || slot
            .as_ref()
            .is_some_and(|current| !is_newer_replaceable(&event, current))
        || saved_site_event(&event, author, d, site_event_kind).is_none()
    {
        return;
    }
    *slot = Some(event);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResendPlan<'a> {
    Superseded(&'a Event),
    SendTo(Vec<RelayUrl>),
}

pub fn plan_site_resends<'a>(held: &[&Event], answers: &'a [RelayAnswers]) -> Vec<ResendPlan<'a>> {
    held.iter()
        .enumerate()
        .map(|(i, event)| {
            let found = || {
                answers.iter().filter_map(move |a| match a.sites.get(i) {
                    Some(SiteAnswer::Found(found) | SiteAnswer::Unfinished(found)) => Some(found),
                    _ => None,
                })
            };
            let newest = found()
                .filter(|found| is_newer_replaceable(found, event))
                .reduce(|a, b| if is_newer_replaceable(b, a) { b } else { a });
            if let Some(newer) = newest {
                return ResendPlan::Superseded(newer);
            }
            ResendPlan::SendTo(
                answers
                    .iter()
                    .filter(|a| match a.sites.get(i) {
                        Some(SiteAnswer::Missing) => true,
                        Some(SiteAnswer::Found(found)) => found.id != event.id,
                        Some(SiteAnswer::Unanswered | SiteAnswer::Unfinished(_)) | None => false,
                    })
                    .map(|a| a.relay.clone())
                    .collect(),
            )
        })
        .collect()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeletionMatch {
    pub(super) by_id: bool,
    pub(super) by_address: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Withdrawals {
    ids: HashSet<(PublicKey, String)>,
    addresses: HashMap<(PublicKey, String), u64>,
}

impl Withdrawals {
    pub(super) fn note(
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
        let through = deletion
            .created_at
            .as_secs()
            .min(now.saturating_add(MAX_FUTURE_SKEW));
        for tag in deletion.tags.iter() {
            let Some(value) = tag.content() else {
                continue;
            };
            let kind = tag.kind();
            if kind == "e" && ids.contains(value) {
                found.by_id = true;
                self.ids.insert((deletion.pubkey, value.to_string()));
            } else if kind == "a" && coordinates.contains(value) {
                found.by_address = true;
                let at = self
                    .addresses
                    .entry((deletion.pubkey, value.to_string()))
                    .or_insert(0);
                *at = (*at).max(through);
            }
        }
        found
    }

    pub(super) fn merge(&mut self, other: Withdrawals) {
        self.ids.extend(other.ids);
        for (key, created_at) in other.addresses {
            let at = self.addresses.entry(key).or_insert(created_at);
            *at = (*at).max(created_at);
        }
    }

    pub fn of(&self, held: &Event, site_event_kind: u16) -> Option<Withdrawn> {
        let id = held.id.to_hex();
        let events: BTreeSet<String> = self
            .ids
            .contains(&(held.pubkey, id.clone()))
            .then_some(id)
            .into_iter()
            .collect();
        let through = held
            .tags
            .identifier()
            .and_then(|d| {
                let coordinate = site_coordinate(site_event_kind, &held.pubkey, &d).to_string();
                self.addresses.get(&(held.pubkey, coordinate)).copied()
            })
            .filter(|at| held.created_at.as_secs() <= *at);
        (!events.is_empty() || through.is_some()).then_some(Withdrawn { events, through })
    }
}

#[cfg(test)]
mod tests;
