use std::collections::{BTreeMap, BTreeSet};

use nostr_sdk::prelude::*;
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
                        if !v.cid.is_empty() {
                            self.activity.record_published(created_at);
                            cids.insert(v.cid);
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

    async fn load_sent_reports(&self, book: &mut ReportBook) {
        if book.loaded {
            return;
        }
        let kind = self.config.nostr.replica_event_kind;
        let events = match self.reporter.fetch_own_reports(kind).await {
            Ok(events) => events,
            Err(e) => {
                warn!(error = %e, "fetching own replica reports failed; stale reports are withdrawn after a later fetch succeeds");
                return;
            }
        };
        let own = events.into_iter().filter(|e| e.pubkey == self.own);
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
            .filter(|ev| nostr::is_follow_set_of(ev, &self.own, &self.config.nostr.mirror_set))
            .map(|ev| nostr::extract_follow_set_pubkeys(ev).into_iter().collect())
            .unwrap_or_default();
        Chosen::from_own(own)
    }

    pub(super) async fn sync_reports(&self) {
        let mut book = self.reports.lock().await;
        self.load_sent_reports(&mut book).await;
        let held = self.held().await;
        book.own_cids = own_cids(&held, &book.own_cids, &self.own.to_hex());
        let now = now_secs();
        let ttl = self.config.agent.report_ttl.as_secs();
        for (key, cids) in reports_to_send(&held, &book.sent, now, ttl / 2) {
            let Some((author_hex, d)) = state::split_site_key(&key) else {
                continue;
            };
            let Ok(author) = PublicKey::from_hex(author_hex) else {
                continue;
            };
            let created_at = book
                .sent
                .get(&key)
                .map_or(now, |prev| now.max(prev.created_at + 1));
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
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::state::VersionRecord;
    use std::collections::HashSet;
    use tokio::task::JoinSet;

    fn cids(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    #[tokio::test]
    async fn stored_versions_are_reported_after_each_store() {
        let mut policy = default_policy();
        policy.keep_versions = 1;
        let fx = Fixture::new(policy, FakeKubo::default());
        let mut tasks = JoinSet::new();

        fx.agent
            .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
        while tasks.join_next().await.is_some() {}
        assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-1"]))]);

        fx.agent
            .submit(fx.event(D, "bafy-1", None, 100), &mut tasks);
        while tasks.join_next().await.is_some() {}
        assert!(fx.take_reports().is_empty());

        fx.agent
            .submit(fx.event(D, "bafy-2", None, 200), &mut tasks);
        while tasks.join_next().await.is_some() {}
        assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-2"]))]);
    }

    #[tokio::test]
    async fn reports_are_refreshed_before_they_expire() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        fx.seed(D, "bafy-b", 1, 200).await;

        fx.agent.sync_reports().await;
        assert_eq!(
            fx.take_reports(),
            vec![(fx.key(D), cids(&["bafy-a", "bafy-b"]))]
        );
        fx.agent.sync_reports().await;
        assert!(fx.take_reports().is_empty());

        let first = fx.sent_created_at(&fx.key(D)).await;
        fx.agent
            .reports
            .lock()
            .await
            .sent
            .get_mut(&fx.key(D))
            .unwrap()
            .created_at -= REPORT_TTL / 2;
        fx.agent.sync_reports().await;
        assert_eq!(
            fx.take_reports(),
            vec![(fx.key(D), cids(&["bafy-a", "bafy-b"]))]
        );
        assert!(fx.sent_created_at(&fx.key(D)).await >= first);
    }

    #[tokio::test]
    async fn a_changed_report_is_newer_than_the_previous_one_within_a_second() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        fx.agent.sync_reports().await;
        let first = fx.sent_created_at(&fx.key(D)).await;

        fx.seed(D, "bafy-b", 1, 200).await;
        fx.agent.sync_reports().await;

        assert!(fx.sent_created_at(&fx.key(D)).await > first);
        assert_eq!(fx.take_reports().len(), 2);
    }

    #[tokio::test]
    async fn unfollowed_sites_are_withdrawn_once() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        fx.agent.sync_reports().await;
        fx.take_reports();

        fx.agent.replace_targets(HashSet::new());
        fx.agent.remove_unfollowed().await;
        fx.agent.sync_reports().await;
        assert_eq!(fx.take_reports(), vec![(fx.key(D), vec![])]);

        fx.agent.sync_reports().await;
        assert!(fx.take_reports().is_empty());
    }

    const CID_A: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
    const CID_B: &str = "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR";

    #[tokio::test]
    async fn reports_left_on_relays_are_withdrawn_or_kept_on_startup() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, CID_A, 1, 100).await;
        let author = fx.pubkey;
        let now = now_secs();
        let old = |d: &str, cids: &[&str], created_at: u64| {
            nostr::build_replica_report_builder(
                35981,
                35980,
                &author,
                d,
                &cids.iter().map(|c| c.to_string()).collect(),
                Timestamp::from_secs(created_at + REPORT_TTL),
            )
            .custom_created_at(Timestamp::from_secs(created_at))
            .finalize(&fx.agent.reporter.keys)
            .unwrap()
        };
        let foreign = nostr::build_replica_report_builder(
            35981,
            35980,
            &author,
            "foreign.example",
            &BTreeSet::from([CID_A.to_string()]),
            Timestamp::from_secs(now + REPORT_TTL),
        )
        .finalize(&Keys::generate())
        .unwrap();
        fx.relay().stored = vec![
            old(D, &[CID_A], now - 10),
            old("gone.example", &[CID_A], now - 20),
            old("gone.example", &[CID_B], now - 10),
            old("withdrawn.example", &[], now - 10),
            foreign,
        ];

        fx.agent.sync_reports().await;

        assert_eq!(fx.take_reports(), vec![(fx.key("gone.example"), vec![])]);
        assert!(fx.sent_created_at(&fx.key("gone.example")).await >= now);
    }

    #[tokio::test]
    async fn stale_reports_are_withdrawn_once_the_relays_answer() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        let stale = nostr::build_replica_report_builder(
            35981,
            35980,
            &fx.pubkey,
            "gone.example",
            &BTreeSet::from([CID_B.to_string()]),
            Timestamp::from_secs(now_secs() + REPORT_TTL),
        )
        .custom_created_at(Timestamp::from_secs(now_secs() - 10))
        .finalize(&fx.agent.reporter.keys)
        .unwrap();
        {
            let mut relay = fx.relay();
            relay.stored = vec![stale];
            relay.fail_fetch = true;
        }

        fx.agent.sync_reports().await;
        assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-a"]))]);

        fx.relay().fail_fetch = false;
        fx.agent.sync_reports().await;
        assert_eq!(fx.take_reports(), vec![(fx.key("gone.example"), vec![])]);
    }

    #[tokio::test]
    async fn rejected_reports_are_retried() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed(D, "bafy-a", 1, 100).await;
        fx.relay().reject = true;
        fx.agent.sync_reports().await;
        assert!(fx.take_reports().is_empty());

        fx.relay().reject = false;
        fx.agent.sync_reports().await;
        assert_eq!(fx.take_reports(), vec![(fx.key(D), cids(&["bafy-a"]))]);
    }

    #[tokio::test]
    async fn a_signing_failure_stops_the_round_and_the_rest_are_retried() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.seed("a.example", "bafy-a", 1, 100).await;
        fx.seed("b.example", "bafy-b", 1, 100).await;
        fx.relay().fail_sign = true;
        fx.agent.sync_reports().await;
        assert_eq!(fx.relay().send_attempts, 1);
        assert!(fx.take_reports().is_empty());

        fx.relay().fail_sign = false;
        fx.agent.sync_reports().await;
        assert_eq!(fx.take_reports().len(), 2);
    }

    #[tokio::test]
    async fn published_versions_are_reported_together_with_mirrored_ones() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.agent.state.lock().await.apply_store(
            &fx.own_key(D),
            VersionRecord {
                cid: "bafy-mirrored".into(),
                size: 1,
                created_at: 100,
                stored_at: 100,
            },
        );
        {
            let mut kubo = fx.kubo();
            kubo.mfs
                .insert(fx.publish_path(D, "100"), "bafy-mirrored".into());
            kubo.mfs
                .insert(fx.publish_path(D, "200"), "bafy-published".into());
            kubo.mfs
                .insert(fx.publish_path(D, "notes.txt"), "bafy-ignored".into());
            kubo.mfs
                .insert(fx.publish_path("a/b.example", "300"), "bafy-ab".into());
            let other = fx.agent.layout.publish_version(&fx.pubkey.to_hex(), D, 1);
            kubo.mfs.insert(other, "bafy-not-mine".into());
        }

        fx.agent.sync_reports().await;

        let mut expected = vec![
            (fx.own_key(D), cids(&["bafy-mirrored", "bafy-published"])),
            (fx.own_key("a/b.example"), cids(&["bafy-ab"])),
        ];
        expected.sort();
        assert_eq!(fx.take_reports(), expected);

        fx.kubo().mfs.retain(|p, _| !p.contains("a%2Fb.example"));
        fx.agent.sync_reports().await;
        assert_eq!(fx.take_reports(), vec![(fx.own_key("a/b.example"), vec![])]);
    }

    #[tokio::test]
    async fn syncing_records_the_newest_published_version_and_never_lowers_it() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        fx.agent.sync_reports().await;
        assert_eq!(fx.agent.activity.latest_published_at(), Some(0));

        {
            let mut kubo = fx.kubo();
            kubo.mfs.insert(fx.publish_path(D, "100"), "bafy-a".into());
            kubo.mfs
                .insert(fx.publish_path("a/b.example", "300"), "bafy-b".into());
            kubo.mfs.insert(fx.publish_path(D, "999999"), String::new());
            kubo.mfs
                .insert(fx.publish_path(D, "notes.txt"), "bafy-n".into());
        }
        fx.agent.sync_reports().await;
        assert_eq!(fx.agent.activity.latest_published_at(), Some(300));

        fx.agent.activity.record_published(400);
        fx.kubo().mfs.retain(|p, _| !p.contains("a%2Fb.example"));
        fx.agent.sync_reports().await;
        assert_eq!(fx.agent.activity.latest_published_at(), Some(400));
    }

    fn report_about_own(fx: &Fixture, reporter: &Keys, d: &str, created_at: u64) -> Event {
        report_about_own_cid(fx, reporter, d, CID_A, created_at)
    }

    fn report_about_own_cid(
        fx: &Fixture,
        reporter: &Keys,
        d: &str,
        cid: &str,
        created_at: u64,
    ) -> Event {
        nostr::build_replica_report_builder(
            35981,
            35980,
            &fx.agent.own,
            d,
            &BTreeSet::from([cid.to_string()]),
            Timestamp::from_secs(created_at + REPORT_TTL),
        )
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(reporter)
        .unwrap()
    }

    async fn publish_own(fx: &Fixture, ds: &[&str]) {
        {
            let mut kubo = fx.kubo();
            for d in ds {
                kubo.mfs.insert(fx.publish_path(d, "100"), CID_A.into());
            }
        }
        fx.agent.sync_reports().await;
        fx.take_reports();
    }

    #[tokio::test]
    async fn only_reports_naming_a_published_cid_up_to_now_move_the_replica_report_time() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        publish_own(&fx, &[D]).await;
        let now = now_secs();
        let other = Keys::generate();
        let future = Keys::generate();
        fx.choose_reporters(&[other.public_key(), future.public_key()])
            .await;
        fx.relay().stored = vec![
            report_about_own_cid(&fx, &other, D, CID_B, now - 10),
            report_about_own(&fx, &other, "unpublished.example", now - 10),
            report_about_own(&fx, &future, D, now + 600),
        ];

        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(0));

        fx.relay()
            .stored
            .push(report_about_own(&fx, &other, D, now - 5));
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 5));
    }

    #[tokio::test]
    async fn reports_by_others_about_own_sites_advance_the_replica_report_time() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        publish_own(&fx, &[D, "b.example"]).await;
        let now = now_secs();
        let other = Keys::generate();
        let second = Keys::generate();
        let own_keys = fx.agent.reporter.keys.clone();
        fx.choose_reporters(&[
            other.public_key(),
            second.public_key(),
            own_keys.public_key(),
        ])
        .await;
        let about_someone_else = nostr::build_replica_report_builder(
            35981,
            35980,
            &fx.pubkey,
            D,
            &BTreeSet::from([CID_A.to_string()]),
            Timestamp::from_secs(now + REPORT_TTL),
        )
        .custom_created_at(Timestamp::from_secs(now - 1))
        .tag(Tag::public_key(fx.agent.own))
        .finalize(&other)
        .unwrap();
        fx.relay().stored = vec![
            report_about_own(&fx, &own_keys, D, now - 1),
            about_someone_else,
            report_about_own(&fx, &other, D, now + nostr::MAX_FUTURE_SKEW + 60),
        ];

        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(0));

        fx.relay()
            .stored
            .push(report_about_own(&fx, &other, D, now - 100));
        fx.relay()
            .stored
            .push(report_about_own(&fx, &second, "b.example", now - 50));
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 50));

        fx.relay()
            .stored
            .push(report_about_own(&fx, &own_keys, "b.example", now));
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 50));
        assert_eq!(fx.relay().about_since, vec![None, None, Some(now - 50)]);
    }

    #[tokio::test]
    async fn only_chosen_reporters_move_the_replica_report_time() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        publish_own(&fx, &[D]).await;
        let now = now_secs();
        let chosen = Keys::generate();
        let stranger = Keys::generate();
        fx.choose_reporters(&[chosen.public_key(), fx.agent.own])
            .await;
        fx.relay().stored = vec![report_about_own(&fx, &stranger, D, now - 5)];

        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(0));
        assert_eq!(fx.relay().about_reporters, vec![vec![chosen.public_key()]]);

        fx.relay()
            .stored
            .push(report_about_own(&fx, &chosen, D, now - 20));
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 20));
    }

    #[tokio::test]
    async fn a_failed_fetch_keeps_the_replica_report_time() {
        let fx = Fixture::new(default_policy(), FakeKubo::default());
        publish_own(&fx, &[D]).await;
        let now = now_secs();
        let other = Keys::generate();
        fx.choose_reporters(&[other.public_key()]).await;
        fx.relay().fail_fetch = true;
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), None);

        {
            let mut relay = fx.relay();
            relay.stored = vec![report_about_own(&fx, &other, D, now - 10)];
            relay.fail_fetch = false;
        }
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 10));

        {
            let mut relay = fx.relay();
            relay.stored.push(report_about_own(&fx, &other, D, now));
            relay.fail_fetch = true;
        }
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now - 10));

        fx.relay().fail_fetch = false;
        fx.agent.record_replica_reports().await;
        assert_eq!(fx.agent.activity.latest_replica_report_at(), Some(now));
    }

    #[test]
    fn reports_are_not_touched_for_sites_whose_listing_failed() {
        let sent = BTreeMap::from([
            (
                "me:a".to_string(),
                SentReport {
                    cids: BTreeSet::from(["x".to_string()]),
                    created_at: 0,
                },
            ),
            (
                "me:b".to_string(),
                SentReport {
                    cids: BTreeSet::from(["x".to_string()]),
                    created_at: 0,
                },
            ),
            (
                "other:c".to_string(),
                SentReport {
                    cids: BTreeSet::from(["x".to_string()]),
                    created_at: 0,
                },
            ),
        ]);
        let held = Held {
            cids: BTreeMap::from([("me:b".to_string(), BTreeSet::from(["y".to_string()]))]),
            unknown: BTreeSet::from(["me:a".to_string(), "me:b".to_string()]),
            unknown_prefix: None,
        };
        assert_eq!(
            reports_to_send(&held, &sent, 1, 100),
            vec![("other:c".to_string(), BTreeSet::new())]
        );

        let held = Held {
            unknown: BTreeSet::new(),
            unknown_prefix: Some("me:".to_string()),
            ..held
        };
        assert_eq!(
            reports_to_send(&held, &sent, 1, 100),
            vec![("other:c".to_string(), BTreeSet::new())]
        );
    }
}
