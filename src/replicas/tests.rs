use std::collections::BTreeSet;

use super::*;
use crate::test_support::{CID_A, CID_B, raw_event, replica_report_event as report};

#[test]
fn a_failed_report_fetch_keeps_the_other_ones_results() {
    let author = Keys::generate().public_key();
    let a = report(&Keys::generate(), &author, "a.example", &[CID_A], 1, 10);
    let b = report(&Keys::generate(), &author, "b.example", &[CID_A], 1, 10);
    let ids =
        |r: Result<Vec<Event>>| -> Vec<EventId> { r.unwrap().into_iter().map(|e| e.id).collect() };
    assert_eq!(
        ids(merge_report_fetches(
            Ok(vec![a.clone()]),
            Ok(vec![b.clone()])
        )),
        vec![a.id, b.id]
    );
    assert_eq!(
        ids(merge_report_fetches(
            Err(anyhow::anyhow!("too many")),
            Ok(vec![b.clone()])
        )),
        vec![b.id]
    );
    assert_eq!(
        ids(merge_report_fetches(
            Ok(vec![a.clone()]),
            Err(anyhow::anyhow!("down"))
        )),
        vec![a.id]
    );
    assert!(merge_report_fetches(Err(anyhow::anyhow!("x")), Err(anyhow::anyhow!("y"))).is_err());
}

#[test]
fn collect_reports_keeps_the_newest_live_report_per_reporter() {
    let base = 2 * nostr::MAX_REPORT_AGE;
    let now = base + 500;
    let author = Keys::generate().public_key();
    let (r1, r2, r3, r4) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let garbage_expiration = Keys::generate();
    let too_old = Keys::generate();
    let d = format!("{}:example.com", author.to_hex());
    let a = nostr::site_coordinate(35980, &author, "example.com").to_string();
    let events = vec![
        report(
            &r1,
            &author,
            "example.com",
            &[CID_A],
            base + 100,
            base + 1000,
        ),
        report(
            &r1,
            &author,
            "example.com",
            &[CID_B],
            base + 200,
            base + 1000,
        ),
        report(
            &r2,
            &author,
            "example.com",
            &[CID_A],
            base + 100,
            base + 1000,
        ),
        report(&r2, &author, "example.com", &[], base + 200, base + 1000),
        report(&r3, &author, "example.com", &[CID_A], base + 100, now),
        report(
            &r4,
            &author,
            "other.example",
            &[CID_A],
            base + 100,
            base + 1000,
        ),
        report(
            &r4,
            &author,
            "example.com",
            &["not-a-cid"],
            base + 100,
            base + 1000,
        ),
        raw_event(
            &garbage_expiration,
            35981,
            &[
                &["d", &d],
                &["a", &a],
                &["cid", CID_A],
                &["expiration", "not-a-number"],
            ],
            base + 100,
        ),
        report(
            &too_old,
            &author,
            "example.com",
            &[CID_A],
            now - nostr::MAX_REPORT_AGE - 1,
            now + nostr::MAX_REPORT_AGE * 10,
        ),
    ];

    let collected = collect_reports(events, 35981, 35980, now, &Chosen::default());

    assert_eq!(collected.len(), 2);
    let site = &collected[&(author, "example.com".to_string())];
    assert_eq!(site.reports.len(), 1);
    assert_eq!(site.dropped, 0);
    assert_eq!(site.reports[0].reporter, r1.public_key());
    assert_eq!(
        site.reports[0].cids,
        BTreeSet::from([nostr::canonical_cid(CID_B).unwrap()])
    );
    let other = &collected[&(author, "other.example".to_string())];
    assert_eq!(other.reports[0].reporter, r4.public_key());
}

#[test]
fn collect_reports_keeps_the_newest_and_flags_truncation_past_the_budget() {
    let author = Keys::generate().public_key();
    let reporters: Vec<Keys> = (0..nostr::budget::MAX_REPORTS_PER_SITE + 1)
        .map(|_| Keys::generate())
        .collect();
    let events: Vec<Event> = reporters
        .iter()
        .enumerate()
        .map(|(i, r)| {
            report(
                r,
                &author,
                "example.com",
                &[CID_A],
                100 + i as u64,
                1_000_000,
            )
        })
        .collect();
    let newest = reporters.last().unwrap();

    let collected = collect_reports(events, 35981, 35980, 200_000, &Chosen::default());
    let site = &collected[&(author, "example.com".to_string())];

    assert_eq!(site.reports.len(), nostr::budget::MAX_REPORTS_PER_SITE);
    assert_eq!(site.dropped, 1);
    assert_eq!(site.reports[0].reporter, newest.public_key());
}

#[test]
fn collect_reports_keeps_trusted_reporters_before_older_others_when_truncating() {
    let author = Keys::generate().public_key();
    let trusted = Keys::generate();
    let others: Vec<Keys> = (0..nostr::budget::MAX_REPORTS_PER_SITE)
        .map(|_| Keys::generate())
        .collect();
    let mut events: Vec<Event> = others
        .iter()
        .enumerate()
        .map(|(i, r)| {
            report(
                r,
                &author,
                "example.com",
                &[CID_A],
                1000 + i as u64,
                1_000_000,
            )
        })
        .collect();
    events.push(report(
        &trusted,
        &author,
        "example.com",
        &[CID_A],
        1,
        1_000_000,
    ));
    let chosen = Chosen {
        author: HashMap::new(),
        own: HashSet::from([trusted.public_key()]),
    };

    let collected = collect_reports(events, 35981, 35980, 500_000, &chosen);
    let site = &collected[&(author, "example.com".to_string())];

    assert_eq!(site.reports.len(), nostr::budget::MAX_REPORTS_PER_SITE);
    assert_eq!(site.dropped, 1);
    assert_eq!(site.reports[0].reporter, trusted.public_key());
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
    let chosen = Chosen {
        author: HashMap::new(),
        own: reporters.iter().map(|k| k.public_key()).collect(),
    };
    let collected = collect_reports(events, 35981, 35980, 0, &chosen);
    let replicas = replicas_of(
        &collected[&(author, "example.com".to_string())].reports,
        CID_A,
        &author,
        &chosen,
    );

    assert_eq!(count_replicas(&replicas).trusted, 2);
    assert_eq!(replicas.len(), 3);
    assert!(replicas[0].latest && replicas[1].latest && !replicas[2].latest);
    assert_eq!(replicas[2].reporter, reporters[1].public_key());
}

#[test]
fn tier_of_classifies_author_chosen_and_other() {
    let author = Keys::generate().public_key();
    let via_author = Keys::generate().public_key();
    let via_own = Keys::generate().public_key();
    let stranger = Keys::generate().public_key();
    let chosen = Chosen {
        author: HashMap::from([(author, HashSet::from([via_author]))]),
        own: HashSet::from([via_own]),
    };

    assert_eq!(tier_of(&author, &author, &chosen), Tier::Author);
    assert_eq!(tier_of(&author, &via_author, &chosen), Tier::Chosen);
    assert_eq!(tier_of(&author, &via_own, &chosen), Tier::Chosen);
    assert_eq!(tier_of(&author, &stranger, &chosen), Tier::Other);
}

#[test]
fn tier_mark_labels_each_tier() {
    assert_eq!(tier_mark(Tier::Author), "  [author]");
    assert_eq!(tier_mark(Tier::Chosen), "  [chosen]");
    assert_eq!(tier_mark(Tier::Other), "  [unverified]");
}

#[test]
fn format_replica_counts_omits_the_parenthesis_when_there_is_nothing_unverified() {
    assert_eq!(
        format_replica_counts(ReplicaCounts {
            trusted: 3,
            unverified: 0
        }),
        "3"
    );
    assert_eq!(
        format_replica_counts(ReplicaCounts {
            trusted: 3,
            unverified: 12
        }),
        "3 (+12 unverified)"
    );
}
