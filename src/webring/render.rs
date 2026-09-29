use std::collections::{BTreeMap, HashMap};

use nostr_sdk::prelude::*;

use super::{Graph, short_npub, split_links};
use crate::{mirror, nostr};

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::nostr::SiteEvent;
    use crate::webring::site_name_lists;

    fn keys(n: usize) -> Vec<PublicKey> {
        let mut pks: Vec<PublicKey> = (0..n).map(|_| Keys::generate().public_key()).collect();
        pks.sort();
        pks
    }

    fn site(pk: PublicKey, d: &str) -> SiteEvent {
        crate::test_support::site_event_fixture(pk, d, 1)
    }

    fn site_names(
        nodes: &BTreeMap<PublicKey, usize>,
        sites: &[SiteEvent],
    ) -> HashMap<PublicKey, String> {
        site_name_lists(nodes, sites)
            .into_iter()
            .map(|(pk, ds)| (pk, ds.join(", ")))
            .collect()
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
