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
            if require_nip05
                && state
                    .verifications
                    .get(key)
                    .is_none_or(|v| v.status != nip05::STATE_VERIFIED)
            {
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

struct Outgoing {
    key: SiteKey,
    cid: String,
    event: Event,
    d: String,
    relays: Vec<RelayUrl>,
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
    let now = now_secs();
    let mut withdrawn = Vec::new();
    let mut resigned = Vec::new();
    let mut wanted: Vec<(Held, Vec<RelayUrl>)> = Vec::new();
    {
        let resend = agent.resend.lock().unwrap();
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
                        .filter(|r| resend.backoff.ready(r, h.event.id, now))
                        .collect();
                    if !relays.is_empty() {
                        wanted.push((h, relays));
                    }
                }
            }
        }
    }
    record_withdrawals(agent, withdrawn).await;
    adopt_resigned(agent, resigned).await;
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
        sends.push(Outgoing {
            key: h.key,
            cid: h.cid,
            event: h.event,
            d: h.d,
            relays,
        });
    }
    record_withdrawals(agent, withdrawn).await;
    send_held(relay, agent, sends).await;
}

async fn record_withdrawals<C, N, R>(agent: &Agent<C, N, R>, withdrawn: Vec<(Held, Withdrawn)>)
where
    C: KuboStore,
    N: Nip05Verify,
    R: ReportRelay,
{
    if withdrawn.is_empty() {
        return;
    }
    let mut state = agent.state.lock().await;
    let mut changed = false;
    for (h, w) in withdrawn {
        if !still_held(&state, &h.key, &h.cid, h.event.id) {
            debug!(site_key = %h.key, event_id = %h.event.id, "the stored version changed during the check; not recording the withdrawal of the held site event");
            continue;
        }
        changed |= state.record_withdrawal(&h.key, w);
    }
    if changed {
        agent.save(&state, "recording withdrawn site events").await;
    }
}

async fn adopt_resigned<C, N, R>(agent: &Agent<C, N, R>, resigned: Vec<(Held, Event)>)
where
    C: KuboStore,
    N: Nip05Verify,
    R: ReportRelay,
{
    if resigned.is_empty() {
        return;
    }
    let mut state = agent.state.lock().await;
    let mut changed = false;
    for (h, newer) in resigned {
        if !still_held(&state, &h.key, &h.cid, h.event.id) {
            debug!(site_key = %h.key, event_id = %h.event.id, "the stored version changed during the check; not keeping the newer signing of the held site event");
            continue;
        }
        changed |= state.fill_latest_event(&h.key, &h.cid, &newer);
    }
    if changed {
        agent.save(&state, "keeping site events").await;
    }
}

async fn send_held<C, N, R>(relay: &RelayClient, agent: &Agent<C, N, R>, sends: Vec<Outgoing>)
where
    C: KuboStore,
    N: Nip05Verify,
    R: ReportRelay,
{
    let sends: Vec<Outgoing> = {
        let state = agent.state.lock().await;
        sends
            .into_iter()
            .filter(|send| {
                if !agent.is_target(&send.event.pubkey) {
                    debug!(site = %send.d, pubkey = %send.event.pubkey, "the author is no longer followed; not sending the held site event");
                    return false;
                }
                if !still_held(&state, &send.key, &send.cid, send.event.id) {
                    debug!(site = %send.d, event_id = %send.event.id, "the stored version changed during the check; not sending the held site event");
                    return false;
                }
                if state.withdrawn.get(&send.key).is_some_and(|w| {
                    w.covers(&send.event.id.to_hex(), send.event.created_at.as_secs())
                }) {
                    debug!(site = %send.d, event_id = %send.event.id, "the author withdrew the held site event during the check; not sending it");
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
        .map(|send| async move {
            let names: Vec<&str> = send.relays.iter().map(|u| u.as_str()).collect();
            info!(site = %send.d, pubkey = %send.event.pubkey, event_id = %send.event.id, relays = ?names, "relays lack the held site event; sending it again");
            let output = relay.publish_to(&send.event, &send.relays).await;
            (send, output)
        })
        .buffer_unordered(budget::FETCH_CONCURRENCY);
    let record = async {
        let mut results = std::pin::pin!(results);
        while let Some((send, output)) = results.next().await {
            let accepted: HashSet<RelayUrl> = match output {
                Ok(output) => {
                    if output.success.is_empty() {
                        warn!(site = %send.d, event_id = %send.event.id, "no relay accepted the held site event");
                    }
                    output.success.into_keys().collect()
                }
                Err(e) => {
                    warn!(site = %send.d, event_id = %send.event.id, error = %e, "sending the held site event again failed");
                    HashSet::new()
                }
            };
            let now = now_secs();
            let mut resend = agent.resend.lock().unwrap();
            for r in &send.relays {
                if accepted.contains(r) {
                    resend.backoff.accepted(r, send.event.id);
                } else {
                    resend.backoff.failed(r, send.event.id, now);
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
