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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        let retry = self.entries.entry((relay.clone(), id)).or_insert(Retry {
            failures: 0,
            until: 0,
        });
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
    created_at: u64,
    event: Event,
    withdrawn: Option<Withdrawn>,
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
                created_at: record.created_at,
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
            if site.cid != h.cid || site.created_at != h.created_at {
                warn!(site_key = %h.key, event_id = %h.event.id, "the held site event does not match the stored version; not sending it");
                return false;
            }
            if h.withdrawn
                .as_ref()
                .is_some_and(|w| w.covers(&h.event.id.to_hex(), h.created_at))
            {
                debug!(site_key = %h.key, event_id = %h.event.id, "the author withdrew the held site event before; not sending it");
                return false;
            }
            true
        })
        .collect()
}

// A newer version in the same second only outranks the held one by id, so covering that second would also cover the newer version.
fn superseded(held: Held, newer: &Event) -> (Held, Withdrawn) {
    let newer_at = newer.created_at.as_secs();
    let withdrawn = if newer_at > held.created_at {
        Withdrawn {
            events: BTreeSet::new(),
            through: Some(newer_at - 1),
        }
    } else {
        Withdrawn {
            events: BTreeSet::from([held.event.id.to_hex()]),
            through: None,
        }
    };
    (held, withdrawn)
}

struct Outgoing {
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
    let mut wanted: Vec<(Held, Vec<RelayUrl>)> = Vec::new();
    {
        let resend = agent.resend.lock().unwrap();
        for (h, plan) in candidates.into_iter().zip(plans) {
            match plan {
                ResendPlan::Superseded(newer) => {
                    info!(site = %h.d, pubkey = %h.pubkey, event_id = %h.event.id, newer_id = %newer.id, "a relay holds a newer version of the held site event; not sending it again");
                    withdrawn.push(superseded(h, newer));
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
        if state
            .latest_version(&h.key)
            .is_none_or(|latest| latest.cid != h.cid || latest.created_at != h.created_at)
        {
            debug!(site_key = %h.key, event_id = %h.event.id, "the stored version changed during the check; not recording the withdrawal of the held site event");
            continue;
        }
        changed |= state.record_withdrawal(&h.key, w);
    }
    if changed {
        agent.save(&state, "recording withdrawn site events").await;
    }
}

async fn send_held<C, N, R>(relay: &RelayClient, agent: &Agent<C, N, R>, sends: Vec<Outgoing>)
where
    C: KuboStore,
    N: Nip05Verify,
    R: ReportRelay,
{
    let sends: Vec<Outgoing> = sends
        .into_iter()
        .filter(|send| {
            let target = agent.is_target(&send.event.pubkey);
            if !target {
                debug!(site = %send.d, pubkey = %send.event.pubkey, "the author is no longer followed; not sending the held site event");
            }
            target
        })
        .collect();
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
                Ok(output) => output.success.into_keys().collect(),
                Err(e) => {
                    warn!(site = %send.d, error = %e, "sending the held site event again failed");
                    HashSet::new()
                }
            };
            if accepted.is_empty() {
                warn!(site = %send.d, event_id = %send.event.id, "no relay accepted the held site event");
            }
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
