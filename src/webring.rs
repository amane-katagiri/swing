use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::future::Future;

use anyhow::Result;
use nostr_sdk::prelude::*;

use crate::config::Config;
use crate::mirror;
use crate::nostr::{self, RelayClient, SiteEvent};
use crate::signer::Signer;

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

#[cfg(test)]
fn site_names(
    nodes: &BTreeMap<PublicKey, usize>,
    sites: &[SiteEvent],
) -> HashMap<PublicKey, String> {
    site_name_lists(nodes, sites)
        .into_iter()
        .map(|(pk, ds)| (pk, ds.join(", ")))
        .collect()
}

pub fn text_labels(
    nodes: &BTreeMap<PublicKey, usize>,
    names: &HashMap<PublicKey, String>,
) -> HashMap<PublicKey, String> {
    let base: HashMap<PublicKey, String> = nodes
        .keys()
        .map(|pk| {
            (
                *pk,
                names.get(pk).cloned().unwrap_or_else(|| short_npub(pk)),
            )
        })
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for label in base.values() {
        *counts.entry(label.as_str()).or_default() += 1;
    }
    base.iter()
        .map(|(pk, label)| {
            if counts[label.as_str()] > 1 {
                (*pk, format!("{label} ({})", short_npub(pk)))
            } else {
                (*pk, label.clone())
            }
        })
        .collect()
}

pub fn render_text(
    graph: &Graph,
    names: &HashMap<PublicKey, String>,
    mirror_set: &str,
    max_depth: usize,
    referencing: &[PublicKey],
    referencing_dropped: usize,
) -> String {
    let labels = text_labels(&graph.nodes, names);
    let key = |pk: &PublicKey| (graph.nodes[pk], labels[pk].clone());
    let links = split_links(graph, key);
    let mut accounts: Vec<&PublicKey> = graph.nodes.keys().collect();
    accounts.sort_by_key(|pk| key(pk));
    let width = labels
        .values()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let mut out = format!(
        "Webring of mirror set \"{mirror_set}\" (depth {max_depth}): {} accounts, {} mutual, {} one-way\n",
        graph.nodes.len(),
        links.mutual.len(),
        links.one_way.len()
    );
    out.push_str("\nAccounts\n");
    for pk in accounts {
        let label = &labels[pk];
        let pad = width - label.chars().count();
        let depth = graph.nodes[pk];
        let mut marks = String::new();
        if depth == 0 {
            marks.push_str("  [root]");
        }
        if graph.without_follow_set.contains(pk) {
            marks.push_str("  [no follow set]");
        }
        out.push_str(&format!(
            "  {label}{}  {}  depth={depth}{marks}\n",
            " ".repeat(pad),
            mirror::npub(pk)
        ));
    }
    out.push_str("\nMutual\n");
    if links.mutual.is_empty() {
        out.push_str("  (none)\n");
    }
    for (a, b) in &links.mutual {
        out.push_str(&format!("  {} ↔ {}\n", labels[a], labels[b]));
    }
    out.push_str("\nOne-way (A → B: A mirrors B)\n");
    if links.one_way.is_empty() {
        out.push_str("  (none)\n");
    }
    for (a, b) in &links.one_way {
        out.push_str(&format!("  {} → {}\n", labels[a], labels[b]));
    }
    out.push_str("\nReferencing the root (unverified)\n");
    if referencing.is_empty() {
        out.push_str("  (none)\n");
    }
    for pk in referencing {
        out.push_str(&format!("  {}\n", mirror::npub(pk)));
    }
    if referencing_dropped > 0 {
        out.push_str(&format!("  … and {referencing_dropped} more\n"));
    }
    if graph.beyond > 0 {
        out.push_str(&format!(
            "\n(accounts beyond depth {max_depth}, not shown: {})\n",
            graph.beyond
        ));
    }
    if graph.over_budget > 0 {
        out.push_str(&format!(
            "\n(crawl stopped at the {}-account budget; not reached: {})\n",
            nostr::budget::MAX_CRAWL_NODES,
            graph.over_budget
        ));
    }
    out
}

fn graph_label(pk: &PublicKey, names: &HashMap<PublicKey, String>) -> (Option<String>, String) {
    (names.get(pk).cloned(), short_npub(pk))
}

pub fn render_dot(graph: &Graph, names: &HashMap<PublicKey, String>) -> String {
    let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let links = split_links(graph, |pk| *pk);
    let mut out = String::from("digraph swing {\n  rankdir=LR;\n  node [shape=box];\n");
    for (pk, depth) in &graph.nodes {
        let label = match graph_label(pk, names) {
            (Some(sites), npub) => format!("{}\\n{}", escape(&sites), escape(&npub)),
            (None, npub) => escape(&npub),
        };
        let root = if *depth == 0 { ", penwidth=2" } else { "" };
        out.push_str(&format!(
            "  \"{}\" [label=\"{label}\"{root}];\n",
            pk.to_hex()
        ));
    }
    for (a, b) in &links.mutual {
        out.push_str(&format!(
            "  \"{}\" -> \"{}\" [dir=both];\n",
            a.to_hex(),
            b.to_hex()
        ));
    }
    for (a, b) in &links.one_way {
        out.push_str(&format!("  \"{}\" -> \"{}\";\n", a.to_hex(), b.to_hex()));
    }
    out.push_str("}\n");
    out
}

pub fn render_mermaid(graph: &Graph, names: &HashMap<PublicKey, String>) -> String {
    let escape = |s: &str| {
        s.replace('#', "#35;")
            .replace('&', "#amp;")
            .replace('"', "#quot;")
            .replace('<', "#lt;")
            .replace('>', "#gt;")
    };
    let ids: HashMap<PublicKey, String> = graph
        .nodes
        .keys()
        .enumerate()
        .map(|(i, pk)| (*pk, format!("n{i}")))
        .collect();
    let links = split_links(graph, |pk| *pk);
    let mut out = String::from("graph LR\n  classDef root stroke-width:3px\n");
    for (pk, depth) in &graph.nodes {
        let label = match graph_label(pk, names) {
            (Some(sites), npub) => format!("{}<br/>{}", escape(&sites), escape(&npub)),
            (None, npub) => escape(&npub),
        };
        let class = if *depth == 0 { ":::root" } else { "" };
        out.push_str(&format!("  {}[\"{label}\"]{class}\n", ids[pk]));
    }
    for (a, b) in &links.mutual {
        out.push_str(&format!("  {} <--> {}\n", ids[a], ids[b]));
    }
    for (a, b) in &links.one_way {
        out.push_str(&format!("  {} --> {}\n", ids[a], ids[b]));
    }
    out
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

    fn site(pk: PublicKey, d: &str) -> SiteEvent {
        SiteEvent {
            pubkey: pk,
            d: d.to_string(),
            cid: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".to_string(),
            url: None,
            size: None,
            title: None,
            message: None,
            created_at: 1,
        }
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

    fn sample() -> (Vec<PublicKey>, Graph, HashMap<PublicKey, String>) {
        let k = keys(4);
        let (me, bob, carol, dave) = (k[0], k[1], k[2], k[3]);
        let graph = Graph {
            nodes: BTreeMap::from([(me, 0), (bob, 1), (carol, 1), (dave, 2)]),
            without_follow_set: BTreeSet::from([dave]),
            edges: BTreeSet::from([(me, bob), (bob, me), (carol, me), (bob, dave)]),
            beyond: 2,
            over_budget: 0,
        };
        let names = site_names(
            &graph.nodes,
            &[
                site(me, "me.example"),
                site(me, "blog.me.example"),
                site(bob, "bob.example"),
                site(carol, "\"x\" <y> #z"),
            ],
        );
        (k, graph, names)
    }

    #[test]
    fn text_lists_accounts_and_links_by_depth() {
        let (k, graph, names) = sample();
        let text = render_text(&graph, &names, "swing", 2, &[], 0);
        let dave = short_npub(&k[3]);

        assert!(text.starts_with(
            "Webring of mirror set \"swing\" (depth 2): 4 accounts, 1 mutual, 2 one-way\n"
        ));
        assert!(text.contains(&format!(
            "  blog.me.example, me.example  {}  depth=0  [root]\n",
            mirror::npub(&k[0])
        )));
        assert!(text.contains("depth=2  [no follow set]\n"));
        assert!(text.contains("Mutual\n  blog.me.example, me.example ↔ bob.example\n"));
        assert!(text.contains(&format!(
            "  \"x\" <y> #z → blog.me.example, me.example\n  bob.example → {dave}\n"
        )));
        assert!(text.contains("Referencing the root (unverified)\n  (none)\n"));
        assert!(text.ends_with("\n(accounts beyond depth 2, not shown: 2)\n"));
    }

    #[test]
    fn text_lists_referencing_accounts_and_the_remainder() {
        let (_, graph, names) = sample();
        let extra = Keys::generate().public_key();
        let text = render_text(&graph, &names, "swing", 2, &[extra], 4);
        assert!(text.contains(&format!(
            "Referencing the root (unverified)\n  {}\n  … and 4 more\n",
            mirror::npub(&extra)
        )));
    }

    #[test]
    fn text_reports_accounts_dropped_by_the_crawl_budget() {
        let (_, mut graph, names) = sample();
        graph.over_budget = 3;
        let text = render_text(&graph, &names, "swing", 2, &[], 0);
        assert!(text.ends_with(&format!(
            "\n(crawl stopped at the {}-account budget; not reached: 3)\n",
            nostr::budget::MAX_CRAWL_NODES
        )));
    }

    #[test]
    fn text_labels_disambiguate_duplicate_site_names() {
        let k = keys(2);
        let nodes = BTreeMap::from([(k[0], 0), (k[1], 1)]);
        let names = site_names(
            &nodes,
            &[site(k[0], "same.example"), site(k[1], "same.example")],
        );
        let labels = text_labels(&nodes, &names);
        assert_eq!(
            labels[&k[0]],
            format!("same.example ({})", short_npub(&k[0]))
        );
        assert_ne!(labels[&k[0]], labels[&k[1]]);
    }

    #[test]
    fn dot_marks_mutual_links_and_roots() {
        let (k, graph, names) = sample();
        let dot = render_dot(&graph, &names);
        let (me, bob, carol) = (k[0].to_hex(), k[1].to_hex(), k[2].to_hex());

        assert!(dot.starts_with("digraph swing {\n"));
        assert!(dot.contains(&format!(
            "  \"{me}\" [label=\"blog.me.example, me.example\\n{}\", penwidth=2];\n",
            short_npub(&k[0])
        )));
        assert!(dot.contains("label=\"\\\"x\\\" <y> #z\\n"));
        assert!(dot.contains(&format!("  \"{me}\" -> \"{bob}\" [dir=both];\n")));
        assert!(!dot.contains(&format!("  \"{bob}\" -> \"{me}\"")));
        assert!(dot.contains(&format!("  \"{carol}\" -> \"{me}\";\n")));
        assert!(dot.ends_with("}\n"));
    }

    #[test]
    fn mermaid_escapes_labels_and_uses_short_ids() {
        let (k, graph, names) = sample();
        let mermaid = render_mermaid(&graph, &names);

        assert!(mermaid.starts_with("graph LR\n"));
        assert!(mermaid.contains(&format!(
            "  n0[\"blog.me.example, me.example<br/>{}\"]:::root\n",
            short_npub(&k[0])
        )));
        assert!(mermaid.contains("  n2[\"#quot;x#quot; #lt;y#gt; #35;z<br/>"));
        assert!(mermaid.contains(&format!("  n3[\"{}\"]\n", short_npub(&k[3]))));
        assert!(mermaid.contains("  n0 <--> n1\n"));
        assert!(mermaid.contains("  n2 --> n0\n"));
        assert!(mermaid.contains("  n1 --> n3\n"));
        assert!(!mermaid.contains(&k[0].to_hex()));
    }
}
