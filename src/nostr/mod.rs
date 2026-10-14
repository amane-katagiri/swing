use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hash;

use nostr_sdk::prelude::*;

mod anchor;
mod client;
mod follow;
mod report;
mod resend;
mod site;

pub use anchor::{
    ANCHOR_KIND, AdReachability, AnchorAd, build_anchor_ad_builder, latest_anchor_ads,
    parse_anchor_ad, validate_peer_id,
};
pub use client::{
    AnchorRelay, DeletionAnswers, Paged, RelayAnswers, RelayClient, RelaySendResult, ReportRelay,
    SiteAnswer, bounded_client, print_relay_line, print_relay_send_result_lines,
    relay_send_results, with_rejection_hint,
};
pub use follow::{
    FollowSetChoice, choose_follow_set, extract_follow_set_pubkeys, follow_set_pubkeys_capped,
    is_follow_set_of, is_saved_follow_set_of,
};
pub use report::{
    MAX_REPORT_AGE, ReplicaReport, build_replica_report_builder, counted_replica_reports,
    parse_replica_report, site_coordinate,
};
pub use resend::{ResendPlan, Withdrawals, keep_newest_site, plan_site_resends, saved_site_event};
pub use site::{
    SignedSite, SiteEvent, SiteFields, build_site_event_builder, build_site_note_builder,
    canonical_cid, cap_sites_per_author, is_unsafe_char, parse_site_event, select_latest,
    site_note_content, valid_http_url, valid_title, validate_d_tag,
};

pub const SITE_SUBSCRIPTION_ID: &str = "swing-sites";
pub const FOLLOW_SET_KIND: u16 = 30000;

// `created_at` is self-declared, so this tolerance is what separates honest clock skew from a forged timestamp.
pub const MAX_FUTURE_SKEW: u64 = 900;

// str::parse::<u64> also takes a leading '+', which the protocol's decimal integer does not allow.
pub fn parse_decimal(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

pub fn plausible_at(created_at: u64, now: u64) -> bool {
    created_at <= now.saturating_add(MAX_FUTURE_SKEW)
}

// Relay events come from anyone under throwaway keys, so the read-only views get hard ceilings.
pub mod budget {
    pub const MAX_FOLLOW_SET_ENTRIES: usize = 500;
    pub const MAX_SITES_PER_AUTHOR_LISTED: usize = 50;
    pub const MAX_REPORTS_PER_SITE: usize = 200;
    pub const MAX_CRAWL_NODES: usize = 1000;
    pub const MAX_REFERENCING_LISTED: usize = 50;
    pub const MAX_RELAY_FETCH_LIMIT: usize = 10_000;
    // Below the result caps relays commonly impose, so a cut answer reaches our own limit and is noticed.
    pub const MAX_DELETIONS_PER_FILTER: usize = 100;
    pub const AUTHORS_PER_FILTER: usize = 50;
    pub const AUTHORS_PER_SPLIT_REQ: usize = 10;
    pub const COORDINATES_PER_FILTER: usize = 250;
    pub const MAX_TRUSTED_REPORTERS: usize = 1000;
    pub const MAX_RELAY_MESSAGE_BYTES: u32 = 128 * 1024;
    pub const MAX_EVENT_BYTES: u32 = 16 * 1024;
    pub const MAX_FOLLOW_SET_EVENT_BYTES: u32 = 64 * 1024;
    pub const MAX_FETCH_TOTAL_EVENTS: usize = 50_000;
    pub const MAX_FETCH_TOTAL_BYTES: usize = 64 * 1024 * 1024;
    pub const FETCH_CONCURRENCY: usize = 4;
    pub const MAX_EVENT_TAGS: u16 = 600;
    pub const MAX_CONTENT_BYTES: usize = 4096;
}

// NIP-01: for replaceable events the later created_at wins, ties broken by the lowest id.
fn replaceable_is_newer(a: (u64, EventId), b: (u64, EventId)) -> bool {
    (a.0, std::cmp::Reverse(a.1)) > (b.0, std::cmp::Reverse(b.1))
}

pub fn is_newer_replaceable(a: &Event, b: &Event) -> bool {
    replaceable_is_newer(
        (a.created_at.as_secs(), a.id),
        (b.created_at.as_secs(), b.id),
    )
}

fn tag_value<'a>(event: &'a Event, kind: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|t| t.kind() == kind)
        .and_then(|t| t.content())
}

fn newest_per_key<T, K: Eq + Hash>(
    items: impl IntoIterator<Item = T>,
    now: u64,
    key: impl Fn(&T) -> K,
    stamp: impl Fn(&T) -> (u64, EventId),
) -> HashMap<K, T> {
    let mut newest: HashMap<K, T> = HashMap::new();
    for item in items {
        let at = stamp(&item);
        if !plausible_at(at.0, now) {
            continue;
        }
        match newest.entry(key(&item)) {
            Entry::Occupied(mut current) => {
                if replaceable_is_newer(at, stamp(current.get())) {
                    current.insert(item);
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(item);
            }
        }
    }
    newest
}

pub fn newest_by_address(events: impl IntoIterator<Item = Event>, now: u64) -> Vec<Event> {
    newest_per_key(
        events,
        now,
        |e| (e.pubkey, e.kind, e.tags.identifier()),
        |e| (e.created_at.as_secs(), e.id),
    )
    .into_values()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CID_A, CID_B, follow_set, keys, replica_report as report};

    #[test]
    fn newer_replaceable_uses_created_at_then_lowest_id() {
        let k = keys();
        let old = follow_set(&k, "swing", None, "a", &[], 100);
        let new = follow_set(&k, "swing", None, "b", &[], 200);
        assert!(is_newer_replaceable(&new, &old));
        assert!(!is_newer_replaceable(&old, &new));
        assert!(!is_newer_replaceable(&old, &old));

        let x = follow_set(&k, "swing", None, "x", &[], 100);
        let y = follow_set(&k, "swing", None, "y", &[], 100);
        let (low, high) = if x.id < y.id { (x, y) } else { (y, x) };
        assert!(is_newer_replaceable(&low, &high));
        assert!(!is_newer_replaceable(&high, &low));
    }

    #[test]
    fn newest_by_address_keeps_one_event_per_author_kind_and_d() {
        let r1 = keys();
        let r2 = keys();
        let author = keys().public_key();
        let old = report(&r1, &author, "example.com", &[CID_A], 100);
        let new = report(&r1, &author, "example.com", &[CID_B], 200);
        let other_site = report(&r1, &author, "other.example", &[CID_A], 50);
        let other_reporter = report(&r2, &author, "example.com", &[CID_A], 50);

        let mut ids: Vec<EventId> = newest_by_address(
            vec![new.clone(), old, other_site.clone(), other_reporter.clone()],
            1000,
        )
        .into_iter()
        .map(|e| e.id)
        .collect();
        ids.sort();
        let mut expected = vec![new.id, other_site.id, other_reporter.id];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn newest_by_address_ignores_an_implausible_future_created_at() {
        let r = keys();
        let author = keys().public_key();
        let plausible = report(&r, &author, "example.com", &[CID_A], 1000 + MAX_FUTURE_SKEW);
        let forged = report(
            &r,
            &author,
            "example.com",
            &[CID_B],
            1000 + MAX_FUTURE_SKEW + 1,
        );

        let ids: Vec<EventId> = newest_by_address(vec![forged, plausible.clone()], 1000)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec![plausible.id]);
    }
}
