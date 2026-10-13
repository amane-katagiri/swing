use std::collections::BTreeSet;

use super::super::tests::ServesForgeries;
use super::*;
use crate::nostr::client::FetchCap;
use crate::signer::Signer;
use crate::test_support::{
    CID_A, CID_B, deletion_event, keys, make_site_event, numbered_deletion_event, refusing_relay,
    relay_holding,
};

#[tokio::test]
async fn sites_per_relay_tell_a_relay_without_the_site_from_one_that_did_not_answer() {
    let k = keys();
    let now = Timestamp::now().as_secs();
    let held = make_site_event(&k, 35980, "a.example", CID_A, now);
    let other = make_site_event(&k, 35980, "b.example", CID_A, now);
    let (_holding, holding) = relay_holding(&[held.clone(), other]).await;
    let (_empty, empty) = relay_holding(&[]).await;
    let (_refusing, refusing) = refusing_relay(None).await;
    let client = RelayClient::connect(
        Signer::Local(keys()),
        &[holding.clone(), empty.clone(), refusing.clone()],
    )
    .await
    .unwrap();

    let sites = vec![(k.public_key(), "a.example".to_string())];
    let answers = client.fetch_sites_per_relay(35980, &sites).await;
    client.shutdown().await;

    let by_url = |url: &str| {
        let url = RelayUrl::parse(url).unwrap();
        answers
            .iter()
            .find(|a| a.relay == url)
            .map(|a| a.sites.clone())
            .unwrap()
    };
    assert_eq!(by_url(&holding), vec![SiteAnswer::Found(held)]);
    assert_eq!(by_url(&empty), vec![SiteAnswer::Missing]);
    assert_eq!(by_url(&refusing), vec![SiteAnswer::Unanswered]);
}

#[tokio::test]
async fn a_newer_version_from_a_request_that_did_not_finish_still_supersedes_the_held_one() {
    let k = keys();
    let now = Timestamp::now().as_secs();
    let held = make_site_event(&k, 35980, "a.example", CID_A, now - 100);
    let newer = make_site_event(&k, 35980, "a.example", CID_B, now - 50);
    let filler = "x".repeat(budget::MAX_EVENT_BYTES as usize - 1024);
    let mut served: Vec<Event> = (0..FetchCap::PER_REQ.bytes / filler.len() + 100)
        .map(|i| {
            EventBuilder::new(Kind::Custom(35980), filler.as_str())
                .tag(Tag::identifier("b.example"))
                .custom_created_at(Timestamp::from_secs(now - 1000 - i as u64))
                .finalize(&k)
                .unwrap()
        })
        .collect();
    served.push(newer.clone());
    let cut_off = LocalRelayBuilder::default()
        .database(ServesForgeries(served))
        .build();
    cut_off.run().await.unwrap();
    let cut_off_url = cut_off.url().await;
    let (_empty, empty) = relay_holding(&[]).await;
    let client = RelayClient::connect(
        Signer::Local(keys()),
        &[cut_off_url.to_string(), empty.clone()],
    )
    .await
    .unwrap();

    let sites = vec![
        (k.public_key(), "a.example".to_string()),
        (k.public_key(), "b.example".to_string()),
    ];
    let answers = client.fetch_sites_per_relay(35980, &sites).await;
    client.shutdown().await;

    let cut = answers.iter().find(|a| a.relay == cut_off_url).unwrap();
    assert_eq!(cut.sites[0], SiteAnswer::Unfinished(newer.clone()));
    assert_eq!(
        crate::nostr::plan_site_resends(&[&held], &answers),
        vec![crate::nostr::ResendPlan::Superseded(&newer)]
    );
}

fn coordinate_of(event: &Event) -> String {
    site_coordinate(35980, &event.pubkey, &event.tags.identifier().unwrap()).to_string()
}

#[test]
fn deletion_requests_give_each_author_its_own_filters_and_limit() {
    let authors: Vec<Keys> = (0..12).map(|_| keys()).collect();
    let mut held: Vec<Event> = authors
        .iter()
        .map(|k| make_site_event(k, 35980, "a.example", CID_A, 100))
        .collect();
    held.push(make_site_event(&authors[0], 35980, "b.example", CID_A, 100));
    let refs: Vec<&Event> = held.iter().collect();

    let units = deletion_units(35980, &refs);
    assert_eq!(units.len(), 12);
    let first = units
        .iter()
        .find(|u| u.author == authors[0].public_key())
        .unwrap();
    assert_eq!(first.sites.len(), 2);
    assert_eq!(first.limit, 4);
    for unit in &units {
        let [by_address, by_id] = &unit.filters;
        for filter in [by_address, by_id] {
            assert_eq!(filter.authors, Some(BTreeSet::from([unit.author])));
            assert_eq!(filter.kinds, Some(BTreeSet::from([Kind::EventDeletion])));
            assert_eq!(filter.limit, Some(unit.limit));
        }
        let addresses = by_address
            .generic_tags
            .get(&SingleLetterTag::from_char('a').unwrap())
            .unwrap();
        assert_eq!(addresses.len(), unit.sites.len());
        assert!(addresses.iter().all(|a| a.contains(&unit.author.to_hex())));
        assert!(
            !by_address
                .generic_tags
                .contains_key(&SingleLetterTag::from_char('e').unwrap())
        );
        let ids = by_id
            .generic_tags
            .get(&SingleLetterTag::from_char('e').unwrap())
            .unwrap();
        assert_eq!(ids.len(), unit.sites.len());
    }
    let requests: Vec<usize> = units
        .chunks(UNITS_PER_REQ)
        .map(|chunk| chunk.iter().flat_map(|u| u.filters.iter()).count())
        .collect();
    assert_eq!(requests, vec![10, 10, 4]);
}

#[tokio::test]
async fn deletions_naming_another_authors_site_do_not_crowd_out_that_authors_own() {
    let flooder = keys();
    let author = keys();
    let now = Timestamp::now().as_secs();
    let flooders_site = make_site_event(&flooder, 35980, "f.example", CID_A, now - 100);
    let held = make_site_event(&author, 35980, "a.example", CID_A, now - 100);
    let target = coordinate_of(&held);
    let mut stored: Vec<Event> = (0..20)
        .map(|i| {
            deletion_event(
                &flooder,
                &[["a", &target], ["e", &held.id.to_hex()]],
                now - i,
            )
        })
        .collect();
    stored.push(deletion_event(&author, &[["a", &target]], now - 50));
    let (_relay, url) = relay_holding(&stored).await;
    let client = RelayClient::connect(Signer::Local(keys()), std::slice::from_ref(&url))
        .await
        .unwrap();

    let answers = client
        .fetch_site_deletions(35980, &[&flooders_site, &held])
        .await;
    client.shutdown().await;

    assert!(answers.withdrawals.of(&held, 35980).is_some());
    assert!(answers.withdrawals.of(&flooders_site, 35980).is_none());
    let relay = RelayUrl::parse(&url).unwrap();
    assert!(answers.answered(&relay, 0));
    assert!(answers.answered(&relay, 1));
}

#[tokio::test]
async fn when_deletions_reach_the_limit_only_sites_newer_than_the_cutoff_are_answered() {
    let author = keys();
    let quiet = keys();
    let now = Timestamp::now().as_secs();
    let newer = make_site_event(&author, 35980, "n.example", CID_A, now - 50);
    let older = make_site_event(&author, 35980, "o.example", CID_A, now - 200);
    let withdrawn = make_site_event(&author, 35980, "w.example", CID_A, now - 400);
    let quiet_site = make_site_event(&quiet, 35980, "a.example", CID_A, now - 300);
    let target = coordinate_of(&withdrawn);
    let stored: Vec<Event> = (0..10)
        .map(|i| deletion_event(&author, &[["a", &target]], now - 100 - i))
        .collect();
    let (_relay, url) = relay_holding(&stored).await;
    let client = RelayClient::connect(Signer::Local(keys()), std::slice::from_ref(&url))
        .await
        .unwrap();

    let answers = client
        .fetch_site_deletions(35980, &[&newer, &older, &withdrawn, &quiet_site])
        .await;
    client.shutdown().await;

    let relay = RelayUrl::parse(&url).unwrap();
    assert!(answers.answered(&relay, 0));
    assert!(!answers.answered(&relay, 1));
    assert!(answers.answered(&relay, 3));
    assert!(answers.withdrawals.of(&newer, 35980).is_none());
    assert!(answers.withdrawals.of(&older, 35980).is_none());
    assert_eq!(
        answers.withdrawals.of(&withdrawn, 35980).unwrap().through,
        Some(now - 100)
    );
}

#[tokio::test]
async fn a_site_created_in_the_same_second_as_the_cutoff_is_unknown() {
    let author = keys();
    let now = Timestamp::now().as_secs();
    let tied = make_site_event(&author, 35980, "t.example", CID_A, now - 100);
    let withdrawn = make_site_event(&author, 35980, "w.example", CID_A, now - 400);
    let target = coordinate_of(&withdrawn);
    let mut stored: Vec<Event> = (0..4)
        .map(|n| numbered_deletion_event(&author, &[["a", &target]], now - 100, &n.to_string()))
        .collect();
    stored.push(deletion_event(&author, &[["a", &target]], now - 300));
    let (_relay, url) = relay_holding(&stored).await;
    let client = RelayClient::connect(Signer::Local(keys()), std::slice::from_ref(&url))
        .await
        .unwrap();

    let answers = client
        .fetch_site_deletions(35980, &[&tied, &withdrawn])
        .await;
    client.shutdown().await;

    let relay = RelayUrl::parse(&url).unwrap();
    assert!(!answers.answered(&relay, 0));
    assert!(answers.withdrawals.of(&tied, 35980).is_none());
    assert_eq!(
        answers.withdrawals.of(&withdrawn, 35980).unwrap().through,
        Some(now - 100)
    );
}

#[tokio::test]
async fn when_deletions_by_id_reach_the_limit_no_site_of_the_author_is_answered() {
    let author = keys();
    let now = Timestamp::now().as_secs();
    let newer = make_site_event(&author, 35980, "n.example", CID_A, now - 50);
    let withdrawn = make_site_event(&author, 35980, "w.example", CID_A, now - 400);
    let id = withdrawn.id.to_hex();
    let mut stored: Vec<Event> = (0..4)
        .map(|i| deletion_event(&author, &[["e", &id]], now - 100 - i))
        .collect();
    stored.push(deletion_event(
        &author,
        &[["e", &newer.id.to_hex()]],
        now - 500,
    ));
    let (_relay, url) = relay_holding(&stored).await;
    let client = RelayClient::connect(Signer::Local(keys()), std::slice::from_ref(&url))
        .await
        .unwrap();

    let answers = client
        .fetch_site_deletions(35980, &[&newer, &withdrawn])
        .await;
    client.shutdown().await;

    let relay = RelayUrl::parse(&url).unwrap();
    assert!(answers.withdrawals.of(&newer, 35980).is_none());
    assert!(!answers.answered(&relay, 0));
    assert!(answers.withdrawals.of(&withdrawn, 35980).is_some());
}

#[test]
fn the_cutoff_is_the_limit_th_newest_of_the_counted_deletions() {
    let mut returned = Returned::default();
    for at in [90, 100, 80] {
        returned.add(at, 4);
    }
    assert_eq!(returned.cutoff(4), None);
    for at in [70, 10] {
        returned.add(at, 4);
    }
    assert_eq!(returned.cutoff(4), Some(70));
}

#[test]
fn a_site_is_answered_only_when_newer_than_the_address_cutoff_and_ids_are_not_cut() {
    assert!(answered_past([None, None], 5));
    assert!(answered_past([Some(4), None], 5));
    assert!(!answered_past([Some(5), None], 5));
    assert!(!answered_past([None, Some(4)], 5));
    assert!(!answered_past([None, Some(6)], 5));
}

#[test]
fn the_deletion_limit_per_filter_is_capped() {
    let author = keys();
    let held: Vec<Event> = (0..60)
        .map(|i| make_site_event(&author, 35980, &format!("s{i}.example"), CID_A, 100))
        .collect();
    let refs: Vec<&Event> = held.iter().collect();

    let units = deletion_units(35980, &refs);
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].limit, budget::MAX_DELETIONS_PER_FILTER);
    for filter in &units[0].filters {
        assert_eq!(filter.limit, Some(budget::MAX_DELETIONS_PER_FILTER));
    }
}

#[tokio::test]
async fn a_relay_that_refuses_the_deletion_request_has_not_answered_it() {
    let author = keys();
    let now = Timestamp::now().as_secs();
    let held = make_site_event(&author, 35980, "a.example", CID_A, now - 100);
    let (_healthy, healthy) = relay_holding(&[]).await;
    let (_refusing, refusing) = refusing_relay(None).await;
    let client = RelayClient::connect(Signer::Local(keys()), &[healthy.clone(), refusing.clone()])
        .await
        .unwrap();

    let answers = client.fetch_site_deletions(35980, &[&held]).await;
    client.shutdown().await;

    assert!(answers.answered(&RelayUrl::parse(&healthy).unwrap(), 0));
    assert!(!answers.answered(&RelayUrl::parse(&refusing).unwrap(), 0));
}
