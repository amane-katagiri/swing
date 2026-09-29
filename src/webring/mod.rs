use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::future::Future;

use anyhow::Result;
use nostr_sdk::prelude::*;

use crate::config::Config;
use crate::mirror;
use crate::nostr::{self, RelayClient, SiteEvent};
use crate::signer::Signer;

mod render;

pub use render::{render_dot, render_mermaid, render_text, text_labels};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Text,
    Dot,
    Mermaid,
}

pub trait FollowSetSource {
    fn follow_sets(
        &self,
        authors: &[PublicKey],
    ) -> impl Future<Output = Result<HashMap<PublicKey, Vec<PublicKey>>>> + Send;
    fn referencing(
        &self,
        targets: &[PublicKey],
    ) -> impl Future<Output = Result<HashSet<PublicKey>>> + Send;
}

struct RelaySource<'a> {
    relay: &'a RelayClient,
    mirror_set: &'a str,
}

impl FollowSetSource for RelaySource<'_> {
    async fn follow_sets(
        &self,
        authors: &[PublicKey],
    ) -> Result<HashMap<PublicKey, Vec<PublicKey>>> {
        let sets = self
            .relay
            .fetch_follow_sets(self.mirror_set, authors)
            .await?;
        Ok(sets
            .into_iter()
            .map(|(pk, event)| (pk, nostr::extract_follow_set_pubkeys(&event)))
            .collect())
    }

    async fn referencing(&self, targets: &[PublicKey]) -> Result<HashSet<PublicKey>> {
        self.relay
            .fetch_follow_set_authors_referencing(self.mirror_set, targets)
            .await
    }
}

#[derive(Debug, Default)]
pub struct Crawl {
    pub depths: BTreeMap<PublicKey, usize>,
    pub follows: HashMap<PublicKey, Vec<PublicKey>>,
    pub over_budget: usize,
    pub referencing: Vec<PublicKey>,
    pub referencing_dropped: usize,
}

// `referencing` only proves someone named a root, not that the root reciprocated, so it never admits nodes.
pub async fn crawl<S: FollowSetSource>(
    source: &S,
    roots: &[PublicKey],
    max_depth: usize,
) -> Result<Crawl> {
    let mut out = Crawl::default();
    for root in roots {
        if out.depths.contains_key(root) {
            continue;
        }
        if out.depths.len() >= nostr::budget::MAX_CRAWL_NODES {
            out.over_budget += 1;
            continue;
        }
        out.depths.insert(*root, 0);
    }
    let mut frontier: Vec<PublicKey> = out.depths.keys().copied().collect();
    let mut depth = 0;
    let mut referencing: BTreeSet<PublicKey> = BTreeSet::new();
    while !frontier.is_empty() {
        let sets = source.follow_sets(&frontier).await?;
        if depth == 0 {
            referencing = source.referencing(&frontier).await?.into_iter().collect();
        }
        if depth == max_depth {
            out.follows.extend(sets);
            break;
        }
        let candidates: BTreeSet<PublicKey> = sets
            .values()
            .flatten()
            .filter(|pk| !out.depths.contains_key(pk))
            .copied()
            .collect();
        out.follows.extend(sets);
        depth += 1;
        let mut next = Vec::new();
        for pk in candidates {
            if out.depths.len() >= nostr::budget::MAX_CRAWL_NODES {
                out.over_budget += 1;
                continue;
            }
            out.depths.insert(pk, depth);
            next.push(pk);
        }
        frontier = next;
    }
    let mut referencing: Vec<PublicKey> = referencing
        .into_iter()
        .filter(|pk| !out.depths.contains_key(pk))
        .collect();
    out.referencing_dropped = referencing
        .len()
        .saturating_sub(nostr::budget::MAX_REFERENCING_LISTED);
    referencing.truncate(nostr::budget::MAX_REFERENCING_LISTED);
    out.referencing = referencing;
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub nodes: BTreeMap<PublicKey, usize>,
    pub without_follow_set: BTreeSet<PublicKey>,
    pub edges: BTreeSet<(PublicKey, PublicKey)>,
    pub beyond: usize,
    pub over_budget: usize,
}

pub fn build_graph(crawl: &Crawl) -> Graph {
    let all_edges: BTreeSet<(PublicKey, PublicKey)> = crawl
        .follows
        .iter()
        .flat_map(|(from, targets)| targets.iter().map(move |to| (*from, *to)))
        .filter(|(from, to)| from != to && crawl.depths.contains_key(to))
        .collect();

    let mut adjacent: HashMap<PublicKey, Vec<PublicKey>> = HashMap::new();
    for (from, to) in &all_edges {
        adjacent.entry(*from).or_default().push(*to);
        adjacent.entry(*to).or_default().push(*from);
    }
    let mut reached: HashSet<PublicKey> = crawl
        .depths
        .iter()
        .filter(|(_, depth)| **depth == 0)
        .map(|(pk, _)| *pk)
        .collect();
    let mut queue: VecDeque<PublicKey> = reached.iter().copied().collect();
    while let Some(pk) = queue.pop_front() {
        for next in adjacent.get(&pk).into_iter().flatten() {
            if reached.insert(*next) {
                queue.push_back(*next);
            }
        }
    }

    let nodes: BTreeMap<PublicKey, usize> = crawl
        .depths
        .iter()
        .filter(|(pk, _)| reached.contains(pk))
        .map(|(pk, depth)| (*pk, *depth))
        .collect();
    let beyond = nodes
        .keys()
        .filter_map(|pk| crawl.follows.get(pk))
        .flatten()
        .filter(|pk| !crawl.depths.contains_key(pk))
        .collect::<HashSet<_>>()
        .len();
    Graph {
        without_follow_set: nodes
            .keys()
            .filter(|pk| !crawl.follows.contains_key(pk))
            .copied()
            .collect(),
        edges: all_edges
            .into_iter()
            .filter(|(from, to)| reached.contains(from) && reached.contains(to))
            .collect(),
        nodes,
        beyond,
        over_budget: crawl.over_budget,
    }
}

pub struct Links {
    pub mutual: Vec<(PublicKey, PublicKey)>,
    pub one_way: Vec<(PublicKey, PublicKey)>,
}

pub fn split_links<K: Ord>(graph: &Graph, key: impl Fn(&PublicKey) -> K) -> Links {
    let mut mutual = Vec::new();
    let mut one_way = Vec::new();
    for (from, to) in &graph.edges {
        if !graph.edges.contains(&(*to, *from)) {
            one_way.push((*from, *to));
        } else if key(from) <= key(to) {
            mutual.push((*from, *to));
        }
    }
    mutual.sort_by_key(|(a, b)| (key(a), key(b)));
    one_way.sort_by_key(|(a, b)| (key(a), key(b)));
    Links { mutual, one_way }
}

pub fn short_npub(pk: &PublicKey) -> String {
    let npub = mirror::npub(pk);
    format!("{}…{}", &npub[..12], &npub[npub.len() - 6..])
}

pub fn site_name_lists(
    nodes: &BTreeMap<PublicKey, usize>,
    sites: &[SiteEvent],
) -> HashMap<PublicKey, Vec<String>> {
    let mut by_account: HashMap<PublicKey, BTreeSet<&str>> = HashMap::new();
    for site in sites {
        if nodes.contains_key(&site.pubkey) {
            by_account.entry(site.pubkey).or_default().insert(&site.d);
        }
    }
    by_account
        .into_iter()
        .map(|(pk, ds)| (pk, ds.into_iter().map(str::to_string).collect()))
        .collect()
}

#[derive(Debug, Clone)]
pub struct WebringView {
    pub graph: Graph,
    pub name_lists: HashMap<PublicKey, Vec<String>>,
    pub names: HashMap<PublicKey, String>,
    pub mirror_set: String,
    pub depth: usize,
    pub referencing: Vec<PublicKey>,
    pub referencing_dropped: usize,
}

pub async fn collect(
    relay: &RelayClient,
    config: &Config,
    roots: &[PublicKey],
    depth: usize,
) -> Result<WebringView> {
    let source = RelaySource {
        relay,
        mirror_set: &config.nostr.mirror_set,
    };
    let crawled = crawl(&source, roots, depth).await?;
    let graph = build_graph(&crawled);
    let accounts: Vec<PublicKey> = graph.nodes.keys().copied().collect();
    let parsed: Vec<SiteEvent> = relay
        .fetch_site_events(config.nostr.site_event_kind, &accounts)
        .await?
        .iter()
        .filter_map(|e| nostr::parse_site_event(e, config.nostr.site_event_kind).ok())
        .collect();
    let latest_map = nostr::select_latest(&parsed, Timestamp::now().as_secs());
    let latest: Vec<SiteEvent> = nostr::cap_sites_per_author(
        latest_map.values(),
        nostr::budget::MAX_SITES_PER_AUTHOR_LISTED,
    )
    .into_iter()
    .cloned()
    .collect();
    let name_lists = site_name_lists(&graph.nodes, &latest);
    let names = name_lists
        .iter()
        .map(|(pk, ds)| (*pk, ds.join(", ")))
        .collect();
    Ok(WebringView {
        graph,
        name_lists,
        names,
        mirror_set: config.nostr.mirror_set.clone(),
        depth,
        referencing: crawled.referencing,
        referencing_dropped: crawled.referencing_dropped,
    })
}

pub async fn show(config: &Config, inputs: &[String], depth: usize, format: Format) -> Result<()> {
    let roots = mirror::parse_pubkey_inputs(inputs)?;
    let relay = RelayClient::connect(Signer::require(config)?, &config.nostr.relays).await?;
    let roots = if roots.is_empty() {
        vec![relay.public_key()]
    } else {
        roots
    };
    let result = collect(&relay, config, &roots, depth).await;
    relay.shutdown().await;
    let view = result?;
    let rendered = match format {
        Format::Text => render_text(
            &view.graph,
            &view.names,
            &view.mirror_set,
            view.depth,
            &view.referencing,
            view.referencing_dropped,
        ),
        Format::Dot => render_dot(&view.graph, &view.names),
        Format::Mermaid => render_mermaid(&view.graph, &view.names),
    };
    print!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct FakeSource {
        sets: HashMap<PublicKey, Vec<PublicKey>>,
        calls: Mutex<Vec<(&'static str, Vec<PublicKey>)>>,
    }

    impl FakeSource {
        fn new(edges: &[(PublicKey, &[PublicKey])]) -> Self {
            Self {
                sets: edges.iter().map(|(pk, ps)| (*pk, ps.to_vec())).collect(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl FollowSetSource for FakeSource {
        async fn follow_sets(
            &self,
            authors: &[PublicKey],
        ) -> Result<HashMap<PublicKey, Vec<PublicKey>>> {
            self.calls.lock().unwrap().push(("sets", authors.to_vec()));
            Ok(authors
                .iter()
                .filter_map(|pk| self.sets.get(pk).map(|ps| (*pk, ps.clone())))
                .collect())
        }

        async fn referencing(&self, targets: &[PublicKey]) -> Result<HashSet<PublicKey>> {
            self.calls
                .lock()
                .unwrap()
                .push(("referencing", targets.to_vec()));
            Ok(self
                .sets
                .iter()
                .filter(|(_, ps)| ps.iter().any(|p| targets.contains(p)))
                .map(|(pk, _)| *pk)
                .collect())
        }
    }

    fn keys(n: usize) -> Vec<PublicKey> {
        let mut pks: Vec<PublicKey> = (0..n).map(|_| Keys::generate().public_key()).collect();
        pks.sort();
        pks
    }

    #[tokio::test]
    async fn crawl_expands_only_along_outbound_edges_past_the_roots() {
        let k = keys(5);
        let (me, bob, carol, dave, erin) = (k[0], k[1], k[2], k[3], k[4]);
        let source = FakeSource::new(&[
            (me, &[bob]),
            (bob, &[me, dave]),
            (carol, &[me]),
            (dave, &[erin]),
        ]);

        let result = crawl(&source, &[me], 1).await.unwrap();

        assert_eq!(result.depths, BTreeMap::from([(me, 0), (bob, 1)]));
        assert_eq!(result.referencing, vec![carol]);
        assert_eq!(result.follows.len(), 2);
        let calls = source.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, "sets");
        assert_eq!(calls[1].0, "referencing");
        assert_eq!(calls[2].0, "sets");
        assert_eq!(calls[2].1, vec![bob]);
    }

    #[tokio::test]
    async fn crawl_at_depth_zero_still_queries_referencing_for_the_roots() {
        let k = keys(2);
        let source = FakeSource::new(&[(k[0], &[k[1]])]);
        let result = crawl(&source, &[k[0]], 0).await.unwrap();
        assert_eq!(result.depths, BTreeMap::from([(k[0], 0)]));
        assert!(result.referencing.is_empty());
        let calls = source.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].0, "referencing");
    }

    #[tokio::test]
    async fn crawl_stops_at_the_node_budget_and_counts_the_excess() {
        let root = Keys::generate().public_key();
        let fanout: Vec<PublicKey> = (0..nostr::budget::MAX_CRAWL_NODES + 50)
            .map(|_| Keys::generate().public_key())
            .collect();
        let source = FakeSource::new(&[(root, fanout.as_slice())]);

        let result = crawl(&source, &[root], 1).await.unwrap();

        assert_eq!(result.depths.len(), nostr::budget::MAX_CRAWL_NODES);
        assert_eq!(result.depths.len() + result.over_budget, fanout.len() + 1);
    }

    #[tokio::test]
    async fn crawl_caps_referencing_accounts_and_counts_the_rest() {
        let root = Keys::generate().public_key();
        let referencers: Vec<PublicKey> = (0..nostr::budget::MAX_REFERENCING_LISTED + 5)
            .map(|_| Keys::generate().public_key())
            .collect();
        let root_slice = [root];
        let edges: Vec<(PublicKey, &[PublicKey])> = referencers
            .iter()
            .map(|pk| (*pk, root_slice.as_slice()))
            .collect();
        let source = FakeSource::new(&edges);

        let result = crawl(&source, &[root], 1).await.unwrap();

        assert_eq!(
            result.referencing.len(),
            nostr::budget::MAX_REFERENCING_LISTED
        );
        assert_eq!(result.referencing_dropped, 5);
    }

    #[test]
    fn build_graph_drops_self_links_outside_targets_and_disconnected_accounts() {
        let k = keys(5);
        let (me, bob, carol, stale, outside) = (k[0], k[1], k[2], k[3], k[4]);
        let crawl = Crawl {
            depths: BTreeMap::from([(me, 0), (bob, 1), (carol, 1), (stale, 1)]),
            follows: HashMap::from([
                (me, vec![me, bob, outside]),
                (bob, vec![me]),
                (stale, vec![carol]),
            ]),
            over_budget: 0,
            referencing: Vec::new(),
            referencing_dropped: 0,
        };

        let graph = build_graph(&crawl);

        assert_eq!(graph.nodes, BTreeMap::from([(me, 0), (bob, 1)]));
        assert_eq!(graph.edges, BTreeSet::from([(me, bob), (bob, me)]));
        assert_eq!(graph.without_follow_set, BTreeSet::new());
        assert_eq!(graph.beyond, 1);
    }

    #[test]
    fn build_graph_keeps_roots_without_follow_sets() {
        let k = keys(2);
        let crawl = Crawl {
            depths: BTreeMap::from([(k[0], 0), (k[1], 0)]),
            follows: HashMap::new(),
            over_budget: 0,
            referencing: Vec::new(),
            referencing_dropped: 0,
        };
        let graph = build_graph(&crawl);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.without_follow_set.len(), 2);
        assert!(graph.edges.is_empty());
    }
}
