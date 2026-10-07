use nostr_sdk::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use tracing::{debug, info, warn};

use crate::ipfs::KuboStore;
use crate::mfs;
use crate::nip05::Nip05Verify;
use crate::nostr::{self, ReportRelay};
use crate::replicas::{self, Chosen, Tier};
use crate::state::{self, SiteKey};

use super::{Agent, now_secs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SentReport {
    pub(super) cids: BTreeSet<String>,
    pub(super) created_at: u64,
}

#[derive(Default)]
pub(super) struct ReportBook {
    pub(super) loaded: bool,
    pub(super) sent: BTreeMap<SiteKey, SentReport>,
    load_tried: bool,
    own_cids: BTreeMap<SiteKey, BTreeSet<String>>,
}

#[derive(Debug, Default)]
struct Held {
    cids: BTreeMap<SiteKey, BTreeSet<String>>,
    unknown: BTreeSet<SiteKey>,
    unknown_prefix: Option<String>,
}

impl Held {
    fn is_unknown(&self, key: &str) -> bool {
        self.unknown.contains(key)
            || self
                .unknown_prefix
                .as_ref()
                .is_some_and(|prefix| key.starts_with(prefix))
    }
}

fn reports_to_send(
    held: &Held,
    sent: &BTreeMap<SiteKey, SentReport>,
    now: u64,
    refresh_after: u64,
) -> Vec<(SiteKey, BTreeSet<String>)> {
    let mut out = Vec::new();
    for (key, cids) in &held.cids {
        if held.is_unknown(key) {
            continue;
        }
        match sent.get(key) {
            Some(prev)
                if prev.cids == *cids && now < prev.created_at.saturating_add(refresh_after) => {}
            _ => out.push((key.clone(), cids.clone())),
        }
    }
    for (key, prev) in sent {
        if prev.cids.is_empty() || held.cids.contains_key(key) || held.is_unknown(key) {
            continue;
        }
        out.push((key.clone(), BTreeSet::new()));
    }
    out
}

// Relays page by created_at, so one round's reports are spread one per second instead of crowding `now`.
fn report_times(floors: &[Option<u64>], now: u64, loaded: bool) -> Vec<u64> {
    if !loaded {
        // A relay may hold a newer report we have not read yet, so nothing goes below `now`, and bumping duplicates upward could pass the future skew allowance.
        return floors
            .iter()
            .map(|floor| floor.map_or(now, |floor| floor.max(now)))
            .collect();
    }
    let mut used = BTreeSet::new();
    floors
        .iter()
        .enumerate()
        .map(|(i, floor)| {
            let mut at = now.saturating_sub(i as u64);
            if let Some(floor) = floor {
                at = at.max(*floor);
            }
            while !used.insert(at) {
                at += 1;
            }
            at
        })
        .collect()
}

fn own_cids(
    held: &Held,
    previous: &BTreeMap<SiteKey, BTreeSet<String>>,
    own_hex: &str,
) -> BTreeMap<SiteKey, BTreeSet<String>> {
    let canonical = |cid: &String| nostr::canonical_cid(cid).unwrap_or_else(|_| cid.clone());
    let mut out: BTreeMap<SiteKey, BTreeSet<String>> = state::account_entries(&held.cids, own_hex)
        .map(|(key, cids)| (key.clone(), cids.iter().map(canonical).collect()))
        .collect();
    for (key, cids) in previous {
        if held.is_unknown(key) {
            out.entry(key.clone())
                .or_default()
                .extend(cids.iter().cloned());
        }
    }
    out
}

impl<C: KuboStore, N: Nip05Verify, R: ReportRelay> Agent<C, N, R> {
    async fn held(&self) -> Held {
        let mut held = Held::default();
        {
            let state = self.state.lock().await;
            for (key, versions) in &state.sites {
                held.cids
                    .entry(key.clone())
                    .or_default()
                    .extend(versions.iter().map(|v| v.cid.clone()));
            }
        }
        let own_hex = self.own.to_hex();
        let account = self.layout.publish_account(&own_hex);
        let sites = match self.ipfs.mfs_list(&account).await {
            Ok(sites) => sites,
            Err(e) => {
                warn!(path = %account, error = %e, "listing published sites failed; leaving their replica reports as they are");
                held.unknown_prefix = Some(format!("{own_hex}:"));
                return held;
            }
        };
        for site in sites.into_iter().filter(|e| e.is_dir) {
            let Some(d) =
                mfs::site_from_name(&site.name).filter(|d| nostr::validate_d_tag(d).is_ok())
            else {
                continue;
            };
            let key = state::site_key(&own_hex, &d);
            let path = format!("{account}/{}", site.name);
            match self.ipfs.mfs_list(&path).await {
                Ok(versions) => {
                    let mut cids = BTreeSet::new();
                    for v in versions {
                        let Ok(created_at) = v.name.parse::<u64>() else {
                            continue;
                        };
                        match nostr::canonical_cid(&v.cid) {
                            Ok(cid) => {
                                self.activity.record_published(created_at);
                                cids.insert(cid);
                            }
                            Err(e) => {
                                warn!(path = %path, version = %v.name, error = %e, "skipping a published version whose MFS entry is not a valid site CID");
                            }
                        }
                    }
                    if !cids.is_empty() {
                        held.cids.entry(key).or_default().extend(cids);
                    }
                }
                Err(e) => {
                    warn!(path = %path, error = %e, "listing published versions failed; leaving the replica report as it is");
                    held.unknown.insert(key);
                }
            }
        }
        if held.unknown.is_empty() {
            self.activity.mark_published_checked();
        }
        held
    }

    async fn load_sent_reports(&self, book: &mut ReportBook, round: bool) {
        if book.loaded || (book.load_tried && !round) {
            return;
        }
        book.load_tried = true;
        let kind = self.config.nostr.replica_event_kind;
        let paged = match self.reporter.fetch_own_reports(kind).await {
            Ok(paged) => paged,
            Err(e) => {
                warn!(error = %e, "fetching own replica reports failed; fetching them again in the next report round, and stale reports are withdrawn once a fetch succeeds");
                return;
            }
        };
        let own = paged.events.into_iter().filter(|e| e.pubkey == self.own);
        for event in nostr::newest_by_address(own, now_secs()) {
            let report = match nostr::parse_replica_report(
                &event,
                kind,
                self.config.nostr.site_event_kind,
            ) {
                Ok(report) => report,
                Err(e) => {
                    debug!(event_id = %event.id, error = %e, "ignoring own replica report");
                    continue;
                }
            };
            let key = state::site_key(&report.author.to_hex(), &report.d);
            if book
                .sent
                .get(&key)
                .is_none_or(|prev| prev.created_at < report.created_at)
            {
                book.sent.insert(
                    key,
                    SentReport {
                        cids: report.cids,
                        created_at: report.created_at,
                    },
                );
            }
        }
        if !paged.complete {
            warn!(
                "no relay listed every own replica report; fetching them again in the next report round"
            );
            return;
        }
        book.loaded = true;
    }

    pub(super) async fn record_replica_reports(&self) {
        let own_cids = self.reports.lock().await.own_cids.clone();
        let own_hex = self.own.to_hex();
        let kind = self.config.nostr.replica_event_kind;
        let since = self
            .activity
            .latest_replica_report_at()
            .filter(|&at| at > 0);
        let chosen = self.chosen_reporters().await;
        let reporters: Vec<PublicKey> = chosen
            .trusted_reporters(&BTreeSet::from([self.own]))
            .into_iter()
            .filter(|pk| *pk != self.own)
            .collect();
        let events = match self
            .reporter
            .fetch_reports_about(kind, self.own, &reporters, since)
            .await
        {
            Ok(events) => events,
            Err(e) => {
                warn!(
                    error = format!("{e:#}"),
                    "fetching replica reports about own sites failed"
                );
                return;
            }
        };
        self.activity.mark_replica_reports_checked();
        let now = now_secs();
        for event in events {
            if replicas::tier_of(&self.own, &event.pubkey, &chosen) != Tier::Chosen
                || !event.tags.public_keys().any(|pk| pk == self.own)
            {
                continue;
            }
            let Ok(report) =
                nostr::parse_replica_report(&event, kind, self.config.nostr.site_event_kind)
            else {
                continue;
            };
            let names_own_cid = own_cids
                .get(&state::site_key(&own_hex, &report.d))
                .is_some_and(|cids| !cids.is_disjoint(&report.cids));
            // A report dated ahead of now is left for a later poll so the cursor never runs ahead of the clock.
            if report.author == self.own
                && names_own_cid
                && report.created_at <= now
                && report.counts_at(now)
            {
                self.activity.record_replica_report(report.created_at);
            }
        }
    }

    // Our CIDs are public, so only reporters the operator chose may move the replica report time.
    async fn chosen_reporters(&self) -> Chosen {
        let state = self.state.lock().await;
        let own = state
            .follow_set
            .as_ref()
            .filter(|ev| {
                nostr::is_saved_follow_set_of(ev, &self.own, &self.config.nostr.mirror_set)
            })
            .map(|ev| nostr::extract_follow_set_pubkeys(ev).into_iter().collect())
            .unwrap_or_default();
        Chosen::from_own(own)
    }

    pub(super) async fn sync_reports(&self) {
        self.sync_reports_with(false).await;
    }

    // Syncs also follow every stored site and a fetch can take minutes, so only the periodic round retries a failed load.
    pub(super) async fn sync_reports_in_round(&self) {
        self.sync_reports_with(true).await;
    }

    async fn sync_reports_with(&self, round: bool) {
        let mut book = self.reports.lock().await;
        self.load_sent_reports(&mut book, round).await;
        let held = self.held().await;
        book.own_cids = own_cids(&held, &book.own_cids, &self.own.to_hex());
        let now = now_secs();
        let ttl = self.config.agent.report_ttl.as_secs();
        let to_send = reports_to_send(&held, &book.sent, now, ttl / 2);
        let floors: Vec<Option<u64>> = to_send
            .iter()
            .map(|(key, _)| book.sent.get(key).map(|prev| prev.created_at + 1))
            .collect();
        let times = report_times(&floors, now, book.loaded);
        for ((key, cids), created_at) in to_send.into_iter().zip(times) {
            let Some((author_hex, d)) = state::split_site_key(&key) else {
                continue;
            };
            let Ok(author) = PublicKey::from_hex(author_hex) else {
                continue;
            };
            let report = nostr::build_replica_report_builder(
                self.config.nostr.replica_event_kind,
                self.config.nostr.site_event_kind,
                &author,
                d,
                &cids,
                Timestamp::from_secs(created_at.saturating_add(ttl)),
            )
            .custom_created_at(Timestamp::from_secs(created_at));
            match self.reporter.send_report(report).await {
                Ok(true) => {
                    info!(site_key = %key, cids = cids.len(), "sent replica report");
                    book.sent.insert(key, SentReport { cids, created_at });
                }
                Ok(false) => {
                    warn!(site_key = %key, "no relay accepted the replica report; will retry")
                }
                Err(e) => {
                    warn!(site_key = %key, error = format!("{e:#}"), "sending the replica report failed; retrying the rest on the next poll");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
