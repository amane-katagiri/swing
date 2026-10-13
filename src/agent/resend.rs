use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use nostr_sdk::prelude::*;
use tracing::{debug, info, warn};

use crate::config::CheckMode;
use crate::ipfs::KuboStore;
use crate::nip05::{self, Nip05Verify};
use crate::nostr::{self, RelayClient, ReportRelay, ResendPlan, budget};
use crate::state::{self, SiteKey, State, Withdrawn};

use super::{Agent, now_secs};

pub(super) const RESEND_CHECK_INTERVAL: Duration = Duration::from_secs(3600);
const SEND_DEADLINE: Duration = Duration::from_secs(120);
const BACKOFF_BASE_SECS: u64 = RESEND_CHECK_INTERVAL.as_secs();
const BACKOFF_CAP_SECS: u64 = 86_400;

#[derive(Default)]
pub(super) struct Schedule {
    checked_at: Option<tokio::time::Instant>,
    backoff: Backoff,
}

impl Schedule {
    pub(super) fn take_due(&mut self, now: tokio::time::Instant) -> bool {
        if self
            .checked_at
            .is_some_and(|at| now.duration_since(at) < RESEND_CHECK_INTERVAL)
        {
            return false;
        }
        self.checked_at = Some(now);
        true
    }
}

#[derive(Default)]
pub(super) struct Backoff {
    entries: HashMap<(RelayUrl, EventId), Retry>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Retry {
    failures: u32,
    until: u64,
}

fn backoff_secs(failures: u32) -> u64 {
    let doublings = failures.saturating_sub(1).min(32);
    BACKOFF_BASE_SECS
        .saturating_mul(1u64 << doublings)
        .min(BACKOFF_CAP_SECS)
}

impl Backoff {
    fn ready(&self, relay: &RelayUrl, id: EventId, now: u64) -> bool {
        self.entries
            .get(&(relay.clone(), id))
            .is_none_or(|retry| now >= retry.until)
    }

    fn failed(&mut self, relay: &RelayUrl, id: EventId, now: u64) {
        let retry = self.entries.entry((relay.clone(), id)).or_default();
        retry.failures = retry.failures.saturating_add(1);
        retry.until = now.saturating_add(backoff_secs(retry.failures));
    }

    fn accepted(&mut self, relay: &RelayUrl, id: EventId) {
        self.entries.remove(&(relay.clone(), id));
    }

    fn retain(&mut self, ids: &HashSet<EventId>) {
        self.entries.retain(|(_, id), _| ids.contains(id));
    }
}

struct Held {
    key: SiteKey,
    pubkey: PublicKey,
    d: String,
    cid: String,
    version_at: u64,
    event: Event,
    withdrawn: Option<Withdrawn>,
}

fn still_held(state: &State, key: &str, cid: &str, id: EventId) -> bool {
    state.latest_version(key).is_some_and(|latest| {
        latest.cid == cid && latest.event.as_ref().is_some_and(|e| e.id == id)
    })
}

fn nip05_verified(state: &State, key: &str) -> bool {
    state
        .verifications
        .get(key)
        .is_some_and(|v| v.status == nip05::STATE_VERIFIED)
}

fn held_site_events(state: &State, targets: &HashSet<PublicKey>, require_nip05: bool) -> Vec<Held> {
    let targets: HashSet<String> = targets.iter().map(|pk| pk.to_hex()).collect();
    state
        .sites
        .keys()
        .filter_map(|key| {
            let (pubkey_hex, d) = state::split_site_key(key)?;
            if !targets.contains(pubkey_hex) {
                return None;
            }
            let record = state.latest_version(key)?;
            let event = record.event.as_ref()?;
            if require_nip05 && !nip05_verified(state, key) {
                debug!(site_key = %key, "the site is not verified by NIP-05; not sending its held site event");
                return None;
            }
            Some(Held {
                key: key.clone(),
                pubkey: PublicKey::from_hex(pubkey_hex).ok()?,
                d: d.to_string(),
                cid: record.cid.clone(),
                version_at: record.created_at,
                event: event.clone(),
                withdrawn: state.withdrawn.get(key).cloned(),
            })
        })
        .collect()
}

fn resend_candidates(held: Vec<Held>, site_event_kind: u16) -> Vec<Held> {
    held.into_iter()
        .filter(|h| {
            let Some(site) = nostr::saved_site_event(&h.event, &h.pubkey, &h.d, site_event_kind)
            else {
                debug!(site_key = %h.key, "the held site event is not a valid site event of the site; not sending it");
                return false;
            };
            if site.cid != h.cid || site.created_at < h.version_at {
                warn!(site_key = %h.key, event_id = %h.event.id, "the held site event does not match the stored version; not sending it");
                return false;
            }
            if h.withdrawn
                .as_ref()
                .is_some_and(|w| w.covers(&h.event.id.to_hex(), h.event.created_at.as_secs()))
            {
                debug!(site_key = %h.key, event_id = %h.event.id, "the author withdrew the held site event before; not sending it");
                return false;
            }
            true
        })
        .collect()
}

// A newer version in the same second only outranks the held one by id, so covering that second would also cover the newer version.
fn superseded(held: &Event, newer: &Event) -> Withdrawn {
    let newer_at = newer.created_at.as_secs();
    if newer_at > held.created_at.as_secs() {
        Withdrawn {
            events: BTreeSet::new(),
            through: Some(newer_at - 1),
        }
    } else {
        Withdrawn {
            events: BTreeSet::from([held.id.to_hex()]),
            through: None,
        }
    }
}

type Classified = (
    Vec<(Held, Withdrawn)>,
    Vec<(Held, Event)>,
    Vec<(Held, Vec<RelayUrl>)>,
);

fn classify(
    candidates: Vec<Held>,
    plans: Vec<ResendPlan<'_>>,
    kind: u16,
    backoff: &Backoff,
    now: u64,
) -> Classified {
    let mut withdrawn = Vec::new();
    let mut resigned = Vec::new();
    let mut wanted = Vec::new();
    for (h, plan) in candidates.into_iter().zip(plans) {
        match plan {
            ResendPlan::Superseded(newer)
                if nostr::saved_site_event(newer, &h.pubkey, &h.d, kind)
                    .is_some_and(|site| site.cid == h.cid) =>
            {
                info!(site = %h.d, pubkey = %h.pubkey, event_id = %h.event.id, newer_id = %newer.id, "a relay holds a newer signing of the held site event's CID; keeping that one instead");
                resigned.push((h, newer.clone()));
            }
            ResendPlan::Superseded(newer) => {
                info!(site = %h.d, pubkey = %h.pubkey, event_id = %h.event.id, newer_id = %newer.id, "a relay holds a newer version of the held site event; not sending it again");
                let w = superseded(&h.event, newer);
                withdrawn.push((h, w));
            }
            ResendPlan::SendTo(relays) => {
                let relays: Vec<RelayUrl> = relays
                    .into_iter()
                    .filter(|r| backoff.ready(r, h.event.id, now))
                    .collect();
                if !relays.is_empty() {
                    wanted.push((h, relays));
                }
            }
        }
    }
    (withdrawn, resigned, wanted)
}

pub(super) async fn resend_held_sites<C, N, R>(
    relay: &RelayClient,
    agent: &Arc<Agent<C, N, R>>,
    targets: &[PublicKey],
) where
    C: KuboStore + Send + Sync + 'static,
    N: Nip05Verify + Send + Sync + 'static,
    R: ReportRelay + Send + Sync + 'static,
{
    let kind = agent.config.nostr.site_event_kind;
    let require_nip05 = agent.config.policy.nip05 == CheckMode::Require;
    let targets: HashSet<PublicKey> = targets
        .iter()
        .copied()
        .filter(|pk| agent.is_target(pk))
        .collect();
    let held = held_site_events(&*agent.state.lock().await, &targets, require_nip05);
    let candidates = resend_candidates(held, kind);
    let ids: HashSet<EventId> = candidates.iter().map(|h| h.event.id).collect();
    agent.resend.lock().unwrap().backoff.retain(&ids);
    if candidates.is_empty() {
        return;
    }
    let sites: Vec<(PublicKey, String)> =
        candidates.iter().map(|h| (h.pubkey, h.d.clone())).collect();
    let answers = relay.fetch_sites_per_relay(kind, &sites).await;
    let events: Vec<&Event> = candidates.iter().map(|h| &h.event).collect();
    let plans = nostr::plan_site_resends(&events, &answers);
    let (withdrawn, resigned, wanted) = {
        let resend = agent.resend.lock().unwrap();
        classify(candidates, plans, kind, &resend.backoff, now_secs())
    };
    apply_if_still_held(
        agent,
        withdrawn,
        "recording withdrawn site events",
        "recording the withdrawal of the held site event",
        |state, h, w| state.record_withdrawal(&h.key, w),
    )
    .await;
    apply_if_still_held(
        agent,
        resigned,
        "keeping site events",
        "keeping the newer signing of the held site event",
        |state, h, newer| state.fill_latest_event(&h.key, &h.cid, &newer),
    )
    .await;
    if wanted.is_empty() {
        return;
    }
    let wanted_events: Vec<&Event> = wanted.iter().map(|(h, _)| &h.event).collect();
    let deletions = relay.fetch_site_deletions(kind, &wanted_events).await;
    let mut withdrawn = Vec::new();
    let mut sends = Vec::new();
    for (i, (h, relays)) in wanted.into_iter().enumerate() {
        if let Some(w) = deletions.withdrawals.of(&h.event, kind) {
            info!(site = %h.d, pubkey = %h.pubkey, event_id = %h.event.id, "the author deleted the held site event; not sending it again");
            withdrawn.push((h, w));
            continue;
        }
        let relays: Vec<RelayUrl> = relays
            .into_iter()
            .filter(|r| deletions.answered(r, i))
            .collect();
        if relays.is_empty() {
            debug!(site = %h.d, event_id = %h.event.id, "no relay lacking the held site event answered the deletion check; not sending it this time");
            continue;
        }
        sends.push((h, relays));
    }
    apply_if_still_held(
        agent,
        withdrawn,
        "recording withdrawn site events",
        "recording the withdrawal of the held site event",
        |state, h, w| state.record_withdrawal(&h.key, w),
    )
    .await;
    send_held(relay, agent, sends).await;
}

async fn apply_if_still_held<C, N, R, T>(
    agent: &Agent<C, N, R>,
    items: Vec<(Held, T)>,
    save_label: &str,
    skipped: &str,
    apply: impl Fn(&mut State, &Held, T) -> bool,
) where
    C: KuboStore,
    N: Nip05Verify,
    R: ReportRelay,
{
    if items.is_empty() {
        return;
    }
    let mut state = agent.state.lock().await;
    let mut changed = false;
    for (h, value) in items {
        if !still_held(&state, &h.key, &h.cid, h.event.id) {
            debug!(site_key = %h.key, event_id = %h.event.id, "the stored version changed during the check; not {skipped}");
            continue;
        }
        changed |= apply(&mut state, &h, value);
    }
    if changed {
        agent.save(&state, save_label).await;
    }
}

async fn send_held<C, N, R>(
    relay: &RelayClient,
    agent: &Agent<C, N, R>,
    sends: Vec<(Held, Vec<RelayUrl>)>,
) where
    C: KuboStore,
    N: Nip05Verify,
    R: ReportRelay,
{
    let require_nip05 = agent.config.policy.nip05 == CheckMode::Require;
    let sends: Vec<(Held, Vec<RelayUrl>)> = {
        let state = agent.state.lock().await;
        sends
            .into_iter()
            .filter(|(h, _)| {
                if !agent.is_target(&h.pubkey) {
                    debug!(site = %h.d, pubkey = %h.pubkey, "the author is no longer followed; not sending the held site event");
                    return false;
                }
                if require_nip05 && !nip05_verified(&state, &h.key) {
                    debug!(site = %h.d, "the site is no longer verified by NIP-05; not sending the held site event");
                    return false;
                }
                if !still_held(&state, &h.key, &h.cid, h.event.id) {
                    debug!(site = %h.d, event_id = %h.event.id, "the stored version changed during the check; not sending the held site event");
                    return false;
                }
                if state
                    .withdrawn
                    .get(&h.key)
                    .is_some_and(|w| w.covers(&h.event.id.to_hex(), h.event.created_at.as_secs())) {
                    debug!(site = %h.d, event_id = %h.event.id, "the author withdrew the held site event during the check; not sending it");
                    return false;
                }
                true
            })
            .collect()
    };
    if sends.is_empty() {
        return;
    }
    let results = futures_util::stream::iter(sends)
        .map(|(h, relays)| async move {
            let names: Vec<&str> = relays.iter().map(|u| u.as_str()).collect();
            info!(site = %h.d, pubkey = %h.pubkey, event_id = %h.event.id, relays = ?names, "relays lack the held site event; sending it again");
            let output = relay.publish_to(&h.event, &relays).await;
            (h, relays, output)
        })
        .buffer_unordered(budget::FETCH_CONCURRENCY);
    let record = async {
        let mut results = std::pin::pin!(results);
        while let Some((h, relays, output)) = results.next().await {
            let accepted: HashSet<RelayUrl> = match output {
                Ok(output) => {
                    if output.success.is_empty() {
                        warn!(site = %h.d, event_id = %h.event.id, "no relay accepted the held site event");
                    }
                    output.success.into_keys().collect()
                }
                Err(e) => {
                    warn!(site = %h.d, event_id = %h.event.id, error = %e, "sending the held site event again failed");
                    HashSet::new()
                }
            };
            let now = now_secs();
            let mut resend = agent.resend.lock().unwrap();
            for r in &relays {
                if accepted.contains(r) {
                    resend.backoff.accepted(r, h.event.id);
                } else {
                    resend.backoff.failed(r, h.event.id, now);
                }
            }
        }
    };
    if tokio::time::timeout(SEND_DEADLINE, record).await.is_err() {
        warn!(
            "sending held site events again did not finish within the deadline; the rest wait for the next check"
        );
    }
}

#[cfg(test)]
mod tests;
