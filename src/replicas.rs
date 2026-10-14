use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::Result;
use nostr_sdk::prelude::*;
use tracing::warn;

use crate::config::Config;
use crate::mirror;
use crate::nostr::{self, RelayClient, ReplicaReport, SiteEvent};
use crate::signer::Signer;

pub type SiteAddress = (PublicKey, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Author,
    Chosen,
    Other,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Author => "author",
            Tier::Chosen => "chosen",
            Tier::Other => "other",
        }
    }
}

// A reporter's own follow set is self-asserted, so only the author's and the operator's choices count.
#[derive(Debug, Default)]
pub struct Chosen {
    author: HashMap<PublicKey, HashSet<PublicKey>>,
    own: HashSet<PublicKey>,
}

impl Chosen {
    pub fn from_own(own: HashSet<PublicKey>) -> Self {
        Self {
            author: HashMap::new(),
            own,
        }
    }

    fn contains(&self, author: &PublicKey, reporter: &PublicKey) -> bool {
        self.author
            .get(author)
            .is_some_and(|s| s.contains(reporter))
            || self.own.contains(reporter)
    }

    pub fn trusted_reporters(&self, authors: &BTreeSet<PublicKey>) -> Vec<PublicKey> {
        let own: BTreeSet<PublicKey> = self.own.iter().copied().collect();
        let chosen: BTreeSet<PublicKey> = authors
            .iter()
            .filter_map(|a| self.author.get(a))
            .flatten()
            .copied()
            .collect();
        let mut seen = HashSet::new();
        authors
            .iter()
            .chain(&own)
            .chain(&chosen)
            .copied()
            .filter(|pk| seen.insert(*pk))
            .take(nostr::budget::MAX_TRUSTED_REPORTERS)
            .collect()
    }
}

async fn fetch_author_chosen(
    relay: &RelayClient,
    mirror_set: &str,
    authors: &[PublicKey],
) -> Result<HashMap<PublicKey, HashSet<PublicKey>>> {
    let sets = relay.fetch_follow_sets(mirror_set, authors).await?;
    Ok(sets
        .into_iter()
        .map(|(pk, ev)| {
            (
                pk,
                nostr::extract_follow_set_pubkeys(&ev).into_iter().collect(),
            )
        })
        .collect())
}

pub async fn fetch_chosen(
    relay: &RelayClient,
    config: &Config,
    authors: &[PublicKey],
) -> Result<Chosen> {
    let own = relay
        .fetch_follow_set(&config.nostr.mirror_set)
        .await?
        .map(|ev| nostr::extract_follow_set_pubkeys(&ev).into_iter().collect())
        .unwrap_or_default();
    fetch_chosen_with_own(relay, config, authors, own).await
}

pub async fn fetch_chosen_with_own(
    relay: &RelayClient,
    config: &Config,
    authors: &[PublicKey],
    own: HashSet<PublicKey>,
) -> Result<Chosen> {
    let author = fetch_author_chosen(relay, &config.nostr.mirror_set, authors).await?;
    Ok(Chosen { author, own })
}

pub fn tier_of(author: &PublicKey, reporter: &PublicKey, chosen: &Chosen) -> Tier {
    if reporter == author {
        Tier::Author
    } else if chosen.contains(author, reporter) {
        Tier::Chosen
    } else {
        Tier::Other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replica {
    pub reporter: PublicKey,
    pub latest: bool,
    pub tier: Tier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplicaCounts {
    pub trusted: usize,
    pub unverified: usize,
}

pub fn count_replicas(replicas: &[Replica]) -> ReplicaCounts {
    let mut counts = ReplicaCounts::default();
    for r in replicas {
        if !r.latest {
            continue;
        }
        match r.tier {
            Tier::Other => counts.unverified += 1,
            Tier::Author | Tier::Chosen => counts.trusted += 1,
        }
    }
    counts
}

pub fn format_replica_counts(counts: ReplicaCounts) -> String {
    if counts.unverified > 0 {
        format!("{} (+{} unverified)", counts.trusted, counts.unverified)
    } else {
        counts.trusted.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SiteReportSet {
    pub reports: Vec<ReplicaReport>,
    pub dropped: usize,
}

fn collect_reports(
    events: Vec<Event>,
    report_kind: u16,
    site_event_kind: u16,
    now: u64,
    chosen: &Chosen,
) -> HashMap<SiteAddress, SiteReportSet> {
    let mut out: HashMap<SiteAddress, Vec<ReplicaReport>> = HashMap::new();
    for report in nostr::counted_replica_reports(events, report_kind, site_event_kind, now) {
        out.entry((report.author, report.d.clone()))
            .or_default()
            .push(report);
    }
    out.into_iter()
        .map(|((author, d), mut reports)| {
            reports.sort_by_cached_key(|r| {
                (
                    tier_of(&author, &r.reporter, chosen),
                    std::cmp::Reverse(r.created_at),
                    r.reporter,
                )
            });
            let dropped = reports
                .len()
                .saturating_sub(nostr::budget::MAX_REPORTS_PER_SITE);
            reports.truncate(nostr::budget::MAX_REPORTS_PER_SITE);
            ((author, d), SiteReportSet { reports, dropped })
        })
        .collect()
}

pub fn replicas_of(
    reports: &[ReplicaReport],
    latest_cid: &str,
    author: &PublicKey,
    chosen: &Chosen,
) -> Vec<Replica> {
    let mut replicas: Vec<Replica> = reports
        .iter()
        .map(|r| Replica {
            reporter: r.reporter,
            latest: r.cids.contains(latest_cid),
            tier: tier_of(author, &r.reporter, chosen),
        })
        .collect();
    replicas.sort_by_key(|r| (r.tier, !r.latest, r.reporter));
    replicas
}

pub async fn fetch_for_sites(
    relay: &RelayClient,
    config: &Config,
    sites: &[&SiteEvent],
    chosen: &Chosen,
) -> Result<HashMap<SiteAddress, SiteReportSet>> {
    let coordinates: Vec<Coordinate> = sites
        .iter()
        .map(|ev| nostr::site_coordinate(config.nostr.site_event_kind, &ev.pubkey, &ev.d))
        .collect();
    let kind = config.nostr.replica_event_kind;
    let authors: BTreeSet<PublicKey> = sites.iter().map(|ev| ev.pubkey).collect();
    let trusted_reporters = chosen.trusted_reporters(&authors);
    let (anyone, trusted) = tokio::join!(
        relay.fetch_replica_reports(kind, &coordinates),
        relay.fetch_replica_reports_by(kind, &coordinates, &trusted_reporters),
    );
    let events = merge_report_fetches(anyone, trusted)?;
    Ok(collect_reports(
        events,
        config.nostr.replica_event_kind,
        config.nostr.site_event_kind,
        Timestamp::now().as_secs(),
        chosen,
    ))
}

fn merge_report_fetches(
    anyone: Result<Vec<Event>>,
    trusted: Result<Vec<Event>>,
) -> Result<Vec<Event>> {
    match (anyone, trusted) {
        (Ok(mut anyone), Ok(trusted)) => {
            anyone.extend(trusted);
            Ok(anyone)
        }
        (Ok(events), Err(e)) => {
            warn!(
                error = format!("{e:#}"),
                "fetching replica reports by trusted reporters failed; counting the rest"
            );
            Ok(events)
        }
        (Err(e), Ok(events)) => {
            warn!(
                error = format!("{e:#}"),
                "fetching replica reports from anyone failed; counting trusted reporters only"
            );
            Ok(events)
        }
        (Err(e), Err(_)) => Err(e),
    }
}

fn tier_mark(tier: Tier) -> &'static str {
    match tier {
        Tier::Author => "  [author]",
        Tier::Chosen => "  [chosen]",
        Tier::Other => "  [unverified]",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reporter {
    pub pubkey: PublicKey,
    pub latest: bool,
    pub tier: Tier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteReplicas {
    pub d: String,
    pub cid: String,
    pub replicas: usize,
    pub unverified: usize,
    pub reports: usize,
    pub dropped: usize,
    pub reporters: Vec<Reporter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorReplicas {
    pub pubkey: PublicKey,
    pub sites: Vec<SiteReplicas>,
}

pub async fn collect(
    relay: &RelayClient,
    config: &Config,
    authors: &[PublicKey],
) -> Result<Vec<AuthorReplicas>> {
    let kind = config.nostr.site_event_kind;
    let held = mirror::load_state(config)
        .await?
        .listed_site_events(kind, authors);
    let latest = relay.fetch_latest_sites(kind, authors, held).await?;
    let sites =
        nostr::cap_sites_per_author(latest.values(), nostr::budget::MAX_SITES_PER_AUTHOR_LISTED);

    let chosen = fetch_chosen(relay, config, authors).await?;
    let reports = fetch_for_sites(relay, config, &sites, &chosen).await?;

    let mut by_author: BTreeMap<PublicKey, Vec<&SiteEvent>> =
        authors.iter().map(|pk| (*pk, Vec::new())).collect();
    for ev in sites {
        by_author.entry(ev.pubkey).or_default().push(ev);
    }

    let mut out = Vec::with_capacity(by_author.len());
    for (author, mut evs) in by_author {
        evs.sort_by(|a, b| a.d.cmp(&b.d));
        let sites = evs
            .into_iter()
            .map(|ev| {
                let report_set = reports.get(&(ev.pubkey, ev.d.clone()));
                let replicas = report_set
                    .map(|r| replicas_of(&r.reports, &ev.cid, &author, &chosen))
                    .unwrap_or_default();
                let counts = count_replicas(&replicas);
                let reporters = replicas
                    .iter()
                    .map(|r| Reporter {
                        pubkey: r.reporter,
                        latest: r.latest,
                        tier: r.tier,
                    })
                    .collect();
                SiteReplicas {
                    d: ev.d.clone(),
                    cid: ev.cid.clone(),
                    replicas: counts.trusted,
                    unverified: counts.unverified,
                    reports: replicas.len(),
                    dropped: report_set.map_or(0, |r| r.dropped),
                    reporters,
                }
            })
            .collect();
        out.push(AuthorReplicas {
            pubkey: author,
            sites,
        });
    }
    Ok(out)
}

fn print_replicas(authors: &[AuthorReplicas]) {
    for author in authors {
        println!(
            "{} ({})",
            mirror::npub(&author.pubkey),
            author.pubkey.to_hex()
        );
        if author.sites.is_empty() {
            println!("  (no site events)");
            continue;
        }
        for site in &author.sites {
            println!(
                "  d={} cid={} replicas={} (reports={})",
                site.d,
                site.cid,
                format_replica_counts(ReplicaCounts {
                    trusted: site.replicas,
                    unverified: site.unverified,
                }),
                site.reports
            );
            for r in &site.reporters {
                let version = if r.latest { "latest" } else { "older version" };
                println!(
                    "    {}  [{version}]{}",
                    mirror::npub(&r.pubkey),
                    tier_mark(r.tier)
                );
            }
            if site.dropped > 0 {
                println!("    … and {} more report(s) not shown", site.dropped);
            }
        }
    }
}

pub async fn show(config: &Config, inputs: &[String]) -> Result<()> {
    let authors = mirror::parse_pubkey_inputs(inputs)?;
    let relay = RelayClient::connect(Signer::require(config)?, &config.nostr.relays).await?;
    let authors = if authors.is_empty() {
        vec![relay.public_key()]
    } else {
        authors
    };
    let result = collect(&relay, config, &authors).await;
    relay.shutdown().await;
    print_replicas(&result?);
    Ok(())
}

#[cfg(test)]
mod tests;
