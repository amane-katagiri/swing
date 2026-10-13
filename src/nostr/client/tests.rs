use std::collections::BTreeSet;
use std::time::Duration;

use super::super::fixtures::{follow_set, report, site_event_with};
use super::fetch::{Measured, Streamed, event_bytes};
use super::*;
use crate::nostr::site_coordinate;
use crate::test_support::{
    CID_A, CID_B, deletion_event, keys, make_site_event, numbered_deletion_event, refusing_relay,
    relay_holding, serve_relay,
};

#[test]
fn relay_send_results_reports_success_and_failure_per_relay() {
    let id = EventBuilder::new(Kind::TextNote, "")
        .finalize(&keys())
        .unwrap()
        .id;
    let mut output: Output<EventId, EventSendStatus, String> = Output::new(id);
    let ok_url = RelayUrl::parse("wss://ok.example").unwrap();
    let failed_url = RelayUrl::parse("wss://failed.example").unwrap();
    output.success.insert(ok_url, EventSendStatus::Sent);
    output
        .failed
        .insert(failed_url, "connection refused".to_string());

    let relays = vec![
        "wss://ok.example".to_string(),
        "wss://failed.example".to_string(),
        "wss://unknown.example".to_string(),
    ];
    let results = relay_send_results(&relays, &output);

    assert_eq!(
        results,
        vec![
            RelaySendResult {
                relay: "wss://ok.example".to_string(),
                ok: true,
                error: None,
            },
            RelaySendResult {
                relay: "wss://failed.example".to_string(),
                ok: false,
                error: Some("connection refused".to_string()),
            },
            RelaySendResult {
                relay: "wss://unknown.example".to_string(),
                ok: false,
                error: None,
            },
        ]
    );
}

#[tokio::test]
async fn the_relay_client_drops_events_over_the_size_or_tag_limits() {
    let local = LocalRelayBuilder::default()
        .max_event_size(1024 * 1024)
        .build();
    local.run().await.unwrap();
    let url = local.url().await.to_string();
    let k = keys();
    let now = Timestamp::now().as_secs();
    let small = make_site_event(&k, 35980, "small.example", CID_A, now);
    let big = site_event_with(
        &k,
        "big.example",
        "t",
        &"a".repeat(budget::MAX_EVENT_BYTES as usize),
    );
    let tagged = EventBuilder::new(Kind::Custom(35980), "")
        .tag(Tag::identifier("tagged.example"))
        .tag(Tag::custom("cid", [CID_A.to_string()]))
        .tags((0..budget::MAX_EVENT_TAGS).map(|i| Tag::custom("x", [i.to_string()])))
        .finalize(&k)
        .unwrap();
    let seeder = Client::default();
    seeder.add_relay(url.as_str()).await.unwrap();
    seeder.connect().await;
    for event in [&small, &big, &tagged] {
        seeder.send_event(event).await.unwrap();
    }
    let stored = seeder
        .fetch_events(Filter::new().kind(Kind::Custom(35980)))
        .timeout(Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(stored.len(), 3);
    seeder.shutdown().await;

    let client = RelayClient::connect(Signer::Local(keys()), &[url])
        .await
        .unwrap();
    let ids: Vec<EventId> = client
        .fetch_site_events(35980, &[k.public_key()])
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, vec![small.id]);
    client.shutdown().await;
}

#[test]
fn reports_for_sites_keeps_only_the_requested_coordinates() {
    let author = keys().public_key();
    let reporter = keys();
    let wanted = report(&reporter, &author, "wanted.example", &[CID_A], 1);
    let other = report(&reporter, &author, "other.example", &[CID_A], 1);
    let sites = [site_coordinate(35980, &author, "wanted.example")];
    let kept: Vec<EventId> =
        reports_for_sites(vec![wanted.clone(), other], Kind::Custom(35981), &sites)
            .into_iter()
            .map(|e| e.id)
            .collect();
    assert_eq!(kept, vec![wanted.id]);
}

fn streamed(url: &RelayUrl, events: &[Event]) -> Vec<(RelayUrl, Streamed)> {
    events
        .iter()
        .cloned()
        .map(|e| (url.clone(), Streamed::Event(e)))
        .collect()
}

async fn newest_created_at(items: Vec<(RelayUrl, Streamed)>, cap: FetchCap) -> Vec<u64> {
    collect_newest(futures_util::stream::iter(items), cap)
        .await
        .events
        .into_iter()
        .map(|m| {
            assert_eq!(m.bytes, event_bytes(&m.event));
            m.event.created_at.as_secs()
        })
        .collect()
}

fn measured(events: &[Event]) -> Vec<Measured> {
    events
        .iter()
        .map(|e| Measured {
            bytes: event_bytes(e),
            event: e.clone(),
        })
        .collect()
}

#[tokio::test]
async fn collect_newest_truncates_to_the_newest_instead_of_failing() {
    let k = keys();
    let url = RelayUrl::parse("wss://a.example").unwrap();
    let events: Vec<Event> = (1..=5)
        .map(|at| make_site_event(&k, 35980, &format!("s{at}.example"), CID_A, at))
        .collect();
    let mut items = streamed(&url, &events);
    items.extend(streamed(&url, &events));
    let cap = FetchCap {
        events: 3,
        bytes: usize::MAX,
    };
    assert_eq!(newest_created_at(items, cap).await, vec![5, 4, 3]);

    let two = event_bytes(&events[4]) + event_bytes(&events[3]);
    let cap = FetchCap {
        events: 100,
        bytes: two,
    };
    assert_eq!(
        newest_created_at(streamed(&url, &events), cap).await,
        vec![5, 4]
    );

    let far = Timestamp::now().as_secs() + super::super::MAX_FUTURE_SKEW + 3600;
    let future = make_site_event(&k, 35980, "future.example", CID_A, far);
    let cap = FetchCap {
        events: 1,
        bytes: usize::MAX,
    };
    assert_eq!(
        newest_created_at(streamed(&url, &[future, events[4].clone()]), cap).await,
        vec![5]
    );
}

#[tokio::test]
async fn collect_newest_counts_only_relays_that_finished_without_failing() {
    let a = RelayUrl::parse("wss://a.example").unwrap();
    let b = RelayUrl::parse("wss://b.example").unwrap();
    let completed = |items: Vec<(RelayUrl, Streamed)>| async move {
        collect_newest(futures_util::stream::iter(items), FetchCap::PER_REQ)
            .await
            .completed
    };
    assert_eq!(completed(vec![]).await, 0);
    assert_eq!(
        completed(vec![
            (a.clone(), Streamed::Failed("closed".into())),
            (a.clone(), Streamed::Finished),
            (b.clone(), Streamed::Failed("refused".into())),
        ])
        .await,
        0
    );
    assert_eq!(
        completed(vec![
            (a.clone(), Streamed::Failed("refused".into())),
            (b.clone(), Streamed::Finished),
        ])
        .await,
        1
    );
}

async fn fetch_follow_set_from(relays: &[String]) -> Result<Option<Event>> {
    let client = RelayClient::connect(Signer::Local(keys()), relays)
        .await
        .unwrap();
    let result = client.fetch_follow_set("swing").await;
    client.shutdown().await;
    result
}

#[tokio::test]
async fn a_fetch_fails_when_no_relay_answers_but_not_when_one_does() {
    let (_refusing, refusing) = refusing_relay(None).await;
    let healthy = LocalRelay::new();
    healthy.run().await.unwrap();
    let healthy = healthy.url().await.to_string();

    let err = fetch_follow_set_from(std::slice::from_ref(&refusing))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("no relay answered"), "{err:#}");
    assert!(fetch_follow_set_from(&[]).await.is_err());
    assert!(
        fetch_follow_set_from(&[refusing, healthy])
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_fetch_fails_when_the_only_relay_refuses_the_connection() {
    let err = fetch_follow_set_from(&["ws://127.0.0.1:1".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("no relay answered"), "{err:#}");
}

#[tokio::test]
async fn gather_stops_at_the_total_budget_and_keeps_what_arrived_by_the_deadline() {
    let k = keys();
    let batch: Vec<Event> = (1..=3)
        .map(|at| make_site_event(&k, 35980, &format!("s{at}.example"), CID_A, at))
        .collect();
    let batches = futures_util::stream::iter([Ok(measured(&batch)), Ok(measured(&batch))]);
    let cap = FetchCap {
        events: 4,
        bytes: usize::MAX,
    };
    let out = gather(batches, cap, Duration::from_secs(5), "test")
        .await
        .unwrap();
    assert_eq!(out.len(), 4);

    let slow =
        futures_util::stream::iter([Ok(measured(&batch))]).chain(futures_util::stream::pending());
    let out = gather(slow, FetchCap::TOTAL, Duration::from_millis(100), "test")
        .await
        .unwrap();
    assert_eq!(out.len(), 3);

    let bytes = measured(&batch[..1])[0].bytes;
    let cap = FetchCap {
        events: 100,
        bytes: bytes * 2,
    };
    let out = gather(
        futures_util::stream::iter([Ok(measured(&batch))]),
        cap,
        Duration::from_secs(5),
        "test",
    )
    .await
    .unwrap();
    assert_eq!(out.len(), 2);
}

#[tokio::test]
async fn gather_keeps_the_answered_requests_and_fails_only_when_none_answered() {
    let k = keys();
    let batch: Vec<Event> = (1..=3)
        .map(|at| make_site_event(&k, 35980, &format!("s{at}.example"), CID_A, at))
        .collect();
    let partly = futures_util::stream::iter([
        Err(anyhow::anyhow!("relay refused")),
        Ok(measured(&batch)),
        Err(anyhow::anyhow!("relay refused")),
    ]);
    let out = gather(partly, FetchCap::TOTAL, Duration::from_secs(5), "test")
        .await
        .unwrap();
    assert_eq!(out.len(), 3);

    let none = futures_util::stream::iter([
        Err(anyhow::anyhow!("first refused")),
        Err(anyhow::anyhow!("second refused")),
    ]);
    let err = gather(none, FetchCap::TOTAL, Duration::from_secs(5), "test")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("refused"), "{err}");

    let empty = futures_util::stream::iter(Vec::<Result<Vec<Measured>>>::new());
    assert!(
        gather(empty, FetchCap::TOTAL, Duration::from_secs(5), "test")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn follow_sets_get_more_room_than_site_events() {
    let local = LocalRelayBuilder::default()
        .max_event_size(1024 * 1024)
        .build();
    local.run().await.unwrap();
    let url = local.url().await.to_string();
    let k = keys();
    let mut tags = vec![Tag::identifier("swing")];
    tags.extend((0..budget::MAX_FOLLOW_SET_ENTRIES).map(|_| Tag::public_key(keys().public_key())));
    let full = EventBuilder::new(Kind::Custom(FOLLOW_SET_KIND), "")
        .tags(tags)
        .finalize(&k)
        .unwrap();
    assert!(event_bytes(&full) > budget::MAX_EVENT_BYTES as usize);
    let seeder = Client::default();
    seeder.add_relay(url.as_str()).await.unwrap();
    seeder.connect().await;
    seeder.send_event(&full).await.unwrap();
    seeder.shutdown().await;

    let client = RelayClient::connect(Signer::Local(keys()), &[url])
        .await
        .unwrap();
    let sets = client
        .fetch_follow_sets("swing", &[k.public_key()])
        .await
        .unwrap();
    assert_eq!(sets[&k.public_key()].id, full.id);
    client.shutdown().await;
}

#[derive(Debug)]
struct IgnoresFilter;

impl QueryPolicy for IgnoresFilter {
    fn admit_query<'a>(
        &'a self,
        query: &'a mut Filter,
        _addr: &'a std::net::SocketAddr,
    ) -> std::pin::Pin<Box<dyn Future<Output = QueryPolicyResult> + Send + 'a>> {
        Box::pin(async move {
            *query = Filter::new();
            QueryPolicyResult::Accept
        })
    }
}

async fn relay_that_ignores_filters(events: &[Event]) -> (LocalRelay, RelayClient) {
    let (local, url) = serve_relay(
        LocalRelayBuilder::default()
            .query_policy(IgnoresFilter)
            .build(),
        events,
    )
    .await;
    let client = RelayClient::connect(Signer::Local(keys()), &[url])
        .await
        .unwrap();
    (local, client)
}

#[tokio::test]
async fn fetches_never_return_events_the_relay_sends_for_unrequested_authors() {
    let now = Timestamp::now().as_secs();
    let wanted = keys();
    let stranger = keys();
    let wanted_site = make_site_event(&wanted, 35980, "wanted.example", CID_A, now);
    let stranger_site = make_site_event(&stranger, 35980, "stranger.example", CID_B, now);
    let wanted_set = follow_set(&wanted, "swing", now, "wanted");
    let stranger_set = follow_set(&stranger, "swing", now, "stranger");
    let wanted_report = report(
        &stranger,
        &wanted.public_key(),
        "wanted.example",
        &[CID_A],
        now,
    );
    let stranger_report = report(
        &wanted,
        &stranger.public_key(),
        "stranger.example",
        &[CID_B],
        now,
    );
    let all = [
        wanted_site.clone(),
        stranger_site.clone(),
        wanted_set.clone(),
        stranger_set.clone(),
        wanted_report.clone(),
        stranger_report.clone(),
    ];
    let (_local, client) = relay_that_ignores_filters(&all).await;

    let unfiltered = client
        .fetch_one(
            Filter::new()
                .kind(Kind::Custom(35980))
                .author(wanted.public_key()),
            "probe",
        )
        .await
        .unwrap();
    assert!(!unfiltered.iter().any(|e| e.id == stranger_site.id));

    let sites = client
        .fetch_site_events(35980, &[wanted.public_key()])
        .await
        .unwrap();
    assert!(sites.iter().all(|e| e.id == wanted_site.id));

    let sets = client
        .fetch_follow_sets("swing", &[wanted.public_key()])
        .await
        .unwrap();
    assert!(sets.values().all(|e| e.id == wanted_set.id));

    let reports = client
        .fetch_replica_reports(
            35981,
            &[site_coordinate(
                35980,
                &wanted.public_key(),
                "wanted.example",
            )],
        )
        .await
        .unwrap();
    assert!(reports.iter().all(|e| e.id == wanted_report.id));

    let wanted_coordinate = [site_coordinate(
        35980,
        &wanted.public_key(),
        "wanted.example",
    )];
    let by_stranger = client
        .fetch_replica_reports_by(35981, &wanted_coordinate, &[stranger.public_key()])
        .await
        .unwrap();
    assert!(by_stranger.iter().all(|e| e.id == wanted_report.id));
    let by_wanted = client
        .fetch_replica_reports_by(35981, &wanted_coordinate, &[wanted.public_key()])
        .await
        .unwrap();
    assert!(by_wanted.is_empty());

    let referencing = client
        .fetch_follow_set_authors_referencing("swing", &[wanted.public_key()])
        .await
        .unwrap();
    assert!(referencing.is_empty());

    client.shutdown().await;
}

type DbFuture<'a, T> =
    std::pin::Pin<Box<dyn Future<Output = Result<T, nostr_database::error::Error>> + Send + 'a>>;

#[derive(Debug)]
struct ServesForgeries(Vec<Event>);

impl NostrDatabase for ServesForgeries {
    fn backend(&self) -> &'static str {
        "forgeries"
    }

    fn features(&self) -> Features {
        Features {
            persistent: false,
            event_expiration: false,
            full_text_search: false,
            request_to_vanish: false,
        }
    }

    fn save_event<'a>(&'a self, _event: &'a Event) -> DbFuture<'a, SaveEventStatus> {
        Box::pin(async { Ok(SaveEventStatus::Success) })
    }

    fn check_id<'a>(&'a self, _event_id: &'a EventId) -> DbFuture<'a, DatabaseEventStatus> {
        Box::pin(async { Ok(DatabaseEventStatus::NotExistent) })
    }

    fn event_by_id<'a>(&'a self, _event_id: &'a EventId) -> DbFuture<'a, Option<Event>> {
        Box::pin(async { Ok(None) })
    }

    fn count(&self, _filter: Filter) -> DbFuture<'_, usize> {
        Box::pin(async { Ok(self.0.len()) })
    }

    fn query(&self, _filter: Filter) -> DbFuture<'_, BTreeSet<Event>> {
        Box::pin(async { Ok(self.0.iter().cloned().collect()) })
    }

    fn delete(&self, _filter: Filter) -> DbFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn wipe(&self) -> DbFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn a_flood_of_fresh_unrequested_events_does_not_push_out_the_real_answer() {
    let k = keys();
    let now = Timestamp::now().as_secs();
    let genuine = make_site_event(&k, 35980, "a.example", CID_A, now - 3600);

    let honest = LocalRelay::new();
    honest.run().await.unwrap();
    let honest_url = honest.url().await;
    let seeder = Client::default();
    seeder.add_relay(honest_url.clone()).await.unwrap();
    seeder.connect().await;
    seeder.send_event(&genuine).await.unwrap();
    seeder.shutdown().await;

    let stranger = keys();
    let filler = "x".repeat(budget::MAX_EVENT_BYTES as usize - 1024);
    let flood: Vec<Event> = (0..FetchCap::PER_REQ.bytes / filler.len() + 100)
        .map(|i| site_event_with(&stranger, &format!("s{i}.example"), "t", &filler))
        .collect();
    let hostile = LocalRelayBuilder::default()
        .database(ServesForgeries(flood))
        .build();
    hostile.run().await.unwrap();
    let hostile_url = hostile.url().await;

    let client = RelayClient::connect(
        Signer::Local(keys()),
        &[honest_url.to_string(), hostile_url.to_string()],
    )
    .await
    .unwrap();
    let sites = client
        .fetch_site_events(35980, &[k.public_key()])
        .await
        .unwrap();
    assert_eq!(
        sites.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![genuine.id]
    );
    client.shutdown().await;
}

#[tokio::test]
async fn a_forged_copy_of_a_verified_event_does_not_hide_the_real_one() {
    let k = keys();
    let now = Timestamp::now().as_secs();
    let genuine = make_site_event(&k, 35980, "a.example", CID_A, now);
    let mut forged = genuine.clone();
    forged.content = "forged".to_string();

    let honest = LocalRelay::new();
    honest.run().await.unwrap();
    let honest_url = honest.url().await;
    let seeder = Client::default();
    seeder.add_relay(honest_url.clone()).await.unwrap();
    seeder.connect().await;
    seeder.send_event(&genuine).await.unwrap();
    seeder.shutdown().await;
    let hostile = LocalRelayBuilder::default()
        .database(ServesForgeries(vec![forged]))
        .build();
    hostile.run().await.unwrap();
    let hostile_url = hostile.url().await;

    let client = RelayClient::connect(Signer::Local(keys()), &[honest_url.to_string()])
        .await
        .unwrap();
    let first = client
        .fetch_site_events(35980, &[k.public_key()])
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].content, genuine.content);

    client.client.add_relay(hostile_url.clone()).await.unwrap();
    client
        .client
        .connect_relay(hostile_url.clone())
        .await
        .unwrap();
    let filter = Filter::new()
        .kind(Kind::Custom(35980))
        .author(k.public_key());
    let from_hostile = client
        .client
        .fetch_events(ReqTarget::single(hostile_url, [filter]))
        .timeout(Duration::from_secs(5))
        .await
        .unwrap();
    assert!(from_hostile.is_empty());

    let both = client
        .fetch_site_events(35980, &[k.public_key()])
        .await
        .unwrap();
    assert_eq!(both.len(), 1);
    assert_eq!(both[0].content, genuine.content);
    client.shutdown().await;
}

#[tokio::test]
async fn paged_fetches_walk_each_relay_past_its_page_limit() {
    let reporter = keys();
    let author = keys().public_key();
    let now = Timestamp::now().as_secs();
    let reports: Vec<Event> = (0..7)
        .map(|i| {
            report(
                &reporter,
                &author,
                &format!("s{i}.example"),
                &[CID_A],
                now - 60 + i,
            )
        })
        .collect();
    let (_a, a) = relay_holding(&reports[..5]).await;
    let (_b, b) = relay_holding(&[&reports[..2], &reports[5..]].concat()).await;

    let client = RelayClient::connect(Signer::Local(reporter.clone()), &[a, b])
        .await
        .unwrap();
    let filter = Filter::new()
        .kind(Kind::Custom(35981))
        .author(reporter.public_key());
    let one_page = client
        .fetch_one(filter.clone().limit(2), "test")
        .await
        .unwrap();
    assert!(one_page.len() < reports.len());
    let paged = client.fetch_pages(filter, 2, "test").await.unwrap();
    assert!(paged.complete);
    let mut ids: Vec<EventId> = paged.events.into_iter().map(|e| e.id).collect();
    ids.sort();
    let mut expected: Vec<EventId> = reports.iter().map(|e| e.id).collect();
    expected.sort();
    assert_eq!(ids, expected);

    let own = ReportRelay::fetch_own_reports(&client, 35981)
        .await
        .unwrap();
    assert!(own.complete);
    assert_eq!(own.events.len(), reports.len());
    client.shutdown().await;
}

#[tokio::test]
async fn paged_fetches_step_past_a_second_holding_more_than_a_page() {
    let reporter = keys();
    let author = keys().public_key();
    let now = Timestamp::now().as_secs();
    let crowded: Vec<Event> = (0..5)
        .map(|i| {
            report(
                &reporter,
                &author,
                &format!("c{i}.example"),
                &[CID_A],
                now - 10,
            )
        })
        .collect();
    let older: Vec<Event> = (0..3)
        .map(|i| {
            report(
                &reporter,
                &author,
                &format!("o{i}.example"),
                &[CID_A],
                now - 60 + i,
            )
        })
        .collect();
    let (_relay, url) = relay_holding(&[&crowded[..], &older[..]].concat()).await;

    let client = RelayClient::connect(Signer::Local(reporter.clone()), &[url])
        .await
        .unwrap();
    let filter = Filter::new()
        .kind(Kind::Custom(35981))
        .author(reporter.public_key());
    let paged = client.fetch_pages(filter, 2, "test").await.unwrap();
    assert!(paged.complete);
    let ids: HashSet<EventId> = paged.events.iter().map(|e| e.id).collect();
    assert!(older.iter().all(|e| ids.contains(&e.id)));
    assert!(crowded.iter().filter(|e| ids.contains(&e.id)).count() >= 2);
    client.shutdown().await;
}

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

    let units = resend::deletion_units(35980, &refs);
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
        .chunks(resend::UNITS_PER_REQ)
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
    let mut returned = resend::Returned::default();
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
    assert!(resend::answered_past([None, None], 5));
    assert!(resend::answered_past([Some(4), None], 5));
    assert!(!resend::answered_past([Some(5), None], 5));
    assert!(!resend::answered_past([None, Some(4)], 5));
    assert!(!resend::answered_past([None, Some(6)], 5));
}

#[test]
fn the_deletion_limit_per_filter_is_capped() {
    let author = keys();
    let held: Vec<Event> = (0..60)
        .map(|i| make_site_event(&author, 35980, &format!("s{i}.example"), CID_A, 100))
        .collect();
    let refs: Vec<&Event> = held.iter().collect();

    let units = resend::deletion_units(35980, &refs);
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
