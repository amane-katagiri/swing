use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use nostr_sdk::prelude::*;

use crate::config::Config;
use crate::mirror;
use crate::nostr::{self, RelayClient, ReplicaReport, SiteEvent};

pub type SiteAddress = (PublicKey, String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replica {
    pub reporter: PublicKey,
    pub latest: bool,
}

fn collect_reports(
    events: Vec<Event>,
    report_kind: u16,
    site_event_kind: u16,
    now: u64,
) -> HashMap<SiteAddress, Vec<ReplicaReport>> {
    let mut out: HashMap<SiteAddress, Vec<ReplicaReport>> = HashMap::new();
    for event in nostr::newest_by_address(events) {
        let Ok(report) = nostr::parse_replica_report(&event, report_kind, site_event_kind) else {
            continue;
        };
        if report.cids.is_empty() || report.is_expired_at(now) {
            continue;
        }
        out.entry((report.author, report.d.clone()))
            .or_default()
            .push(report);
    }
    out
}

pub fn replicas_of(reports: &[ReplicaReport], latest_cid: &str) -> Vec<Replica> {
    let mut replicas: Vec<Replica> = reports
        .iter()
        .map(|r| Replica {
            reporter: r.reporter,
            latest: r.cids.contains(latest_cid),
        })
        .collect();
    replicas.sort_by_key(|r| (!r.latest, r.reporter.to_hex()));
    replicas
}

pub fn latest_count(replicas: &[Replica]) -> usize {
    replicas.iter().filter(|r| r.latest).count()
}

pub async fn fetch_for_sites(
    relay: &RelayClient,
    config: &Config,
    sites: &[&SiteEvent],
) -> Result<HashMap<SiteAddress, Vec<ReplicaReport>>> {
    let coordinates: Vec<Coordinate> = sites
        .iter()
        .map(|ev| nostr::site_coordinate(config.nostr.site_event_kind, &ev.pubkey, &ev.d))
        .collect();
    let events = relay
        .fetch_replica_reports(config.nostr.replica_event_kind, &coordinates)
        .await?;
    Ok(collect_reports(
        events,
        config.nostr.replica_event_kind,
        config.nostr.site_event_kind,
        Timestamp::now().as_secs(),
    ))
}

fn follow_mark(
    author: &PublicKey,
    reporter: &PublicKey,
    follows: &HashSet<PublicKey>,
) -> &'static str {
    if reporter == author {
        "  [author]"
    } else if follows.contains(reporter) {
        ""
    } else {
        "  [not following]"
    }
}

pub async fn show(config: &Config, inputs: &[String]) -> Result<()> {
    let authors = mirror::parse_pubkey_inputs(inputs)?;
    let relay = RelayClient::connect(&config.nostr.secret_key, &config.nostr.relays).await?;
    let authors = if authors.is_empty() {
        vec![relay.keys.public_key()]
    } else {
        authors
    };
    let result = show_with(&relay, config, &authors).await;
    relay.client.shutdown().await;
    result
}

async fn show_with(relay: &RelayClient, config: &Config, authors: &[PublicKey]) -> Result<()> {
    let raw_events = relay
        .fetch_site_events(config.nostr.site_event_kind, authors)
        .await?;
    let parsed: Vec<SiteEvent> = raw_events
        .iter()
        .filter_map(|e| nostr::parse_site_event(e, config.nostr.site_event_kind).ok())
        .collect();
    let latest = nostr::select_latest(&parsed);
    let sites: Vec<&SiteEvent> = latest.values().collect();
    let reports = fetch_for_sites(relay, config, &sites).await?;

    let reporters: Vec<PublicKey> = reports
        .values()
        .flatten()
        .map(|r| r.reporter)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let follow_sets = relay
        .fetch_follow_sets(&config.nostr.mirror_set, &reporters)
        .await?;

    let mut by_author: BTreeMap<String, Vec<&SiteEvent>> =
        authors.iter().map(|pk| (pk.to_hex(), Vec::new())).collect();
    for ev in sites {
        by_author.entry(ev.pubkey.to_hex()).or_default().push(ev);
    }
    for (author_hex, mut evs) in by_author {
        let author = mirror::print_account_header(&author_hex, "")?;
        if evs.is_empty() {
            println!("  (no site events)");
            continue;
        }
        evs.sort_by(|a, b| a.d.cmp(&b.d));
        let followers: HashSet<PublicKey> = follow_sets
            .iter()
            .filter(|(_, fs)| nostr::extract_follow_set_pubkeys(fs).contains(&author))
            .map(|(pk, _)| *pk)
            .collect();
        for ev in evs {
            let replicas = reports
                .get(&(ev.pubkey, ev.d.clone()))
                .map(|r| replicas_of(r, &ev.cid))
                .unwrap_or_default();
            println!(
                "  d={} cid={} replicas={} (reports={})",
                ev.d,
                ev.cid,
                latest_count(&replicas),
                replicas.len()
            );
            for r in &replicas {
                let version = if r.latest { "latest" } else { "older version" };
                println!(
                    "    {}  [{version}]{}",
                    mirror::npub(&r.reporter),
                    follow_mark(&author, &r.reporter, &followers)
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const CID_A: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
    const CID_B: &str = "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR";

    fn report(
        reporter: &Keys,
        author: &PublicKey,
        d: &str,
        cids: &[&str],
        created_at: u64,
        expiration: u64,
    ) -> Event {
        nostr::build_replica_report_builder(
            35981,
            35980,
            author,
            d,
            &cids.iter().map(|c| c.to_string()).collect::<BTreeSet<_>>(),
            Timestamp::from_secs(expiration),
        )
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(reporter)
        .unwrap()
    }

    #[test]
    fn collect_reports_keeps_the_newest_live_report_per_reporter() {
        let author = Keys::generate().public_key();
        let (r1, r2, r3, r4) = (
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
        );
        let events = vec![
            report(&r1, &author, "example.com", &[CID_A], 100, 1000),
            report(&r1, &author, "example.com", &[CID_B], 200, 1000),
            report(&r2, &author, "example.com", &[CID_A], 100, 1000),
            report(&r2, &author, "example.com", &[], 200, 1000),
            report(&r3, &author, "example.com", &[CID_A], 100, 500),
            report(&r4, &author, "other.example", &[CID_A], 100, 1000),
            report(&r4, &author, "example.com", &["not-a-cid"], 100, 1000),
        ];

        let collected = collect_reports(events, 35981, 35980, 500);

        assert_eq!(collected.len(), 2);
        let site = &collected[&(author, "example.com".to_string())];
        assert_eq!(site.len(), 1);
        assert_eq!(site[0].reporter, r1.public_key());
        assert_eq!(site[0].cids, BTreeSet::from([CID_B.to_string()]));
        let other = &collected[&(author, "other.example".to_string())];
        assert_eq!(other[0].reporter, r4.public_key());
    }

    #[test]
    fn replicas_are_counted_by_the_latest_cid() {
        let author = Keys::generate().public_key();
        let reporters: Vec<Keys> = (0..3).map(|_| Keys::generate()).collect();
        let events = vec![
            report(
                &reporters[0],
                &author,
                "example.com",
                &[CID_A, CID_B],
                1,
                1000,
            ),
            report(&reporters[1], &author, "example.com", &[CID_B], 1, 1000),
            report(&reporters[2], &author, "example.com", &[CID_A], 1, 1000),
        ];
        let collected = collect_reports(events, 35981, 35980, 0);
        let replicas = replicas_of(&collected[&(author, "example.com".to_string())], CID_A);

        assert_eq!(latest_count(&replicas), 2);
        assert_eq!(replicas.len(), 3);
        assert!(replicas[0].latest && replicas[1].latest && !replicas[2].latest);
        assert_eq!(replicas[2].reporter, reporters[1].public_key());
    }

    #[test]
    fn follow_mark_distinguishes_the_author_and_non_followers() {
        let author = Keys::generate().public_key();
        let follower = Keys::generate().public_key();
        let stranger = Keys::generate().public_key();
        let follows = HashSet::from([follower]);
        assert_eq!(follow_mark(&author, &author, &follows), "  [author]");
        assert_eq!(follow_mark(&author, &follower, &follows), "");
        assert_eq!(
            follow_mark(&author, &stranger, &follows),
            "  [not following]"
        );
    }
}
