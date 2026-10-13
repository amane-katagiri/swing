use super::*;
use crate::test_support::{CID_A, CID_B, deletion_event as deletion, keys, make_site_event};

const KIND: u16 = 35980;
const NOW: u64 = 1_000_000;

fn relay(name: &str, sites: Vec<SiteAnswer>) -> RelayAnswers {
    RelayAnswers {
        relay: RelayUrl::parse(&format!("wss://{name}.example")).unwrap(),
        sites,
    }
}

fn names(plan: &ResendPlan) -> Vec<String> {
    let ResendPlan::SendTo(relays) = plan else {
        panic!("expected relays to send to, got {plan:?}");
    };
    relays
        .iter()
        .map(|u| u.as_str().trim_end_matches('/').to_string())
        .collect()
}

fn found(event: &Event) -> SiteAnswer {
    SiteAnswer::Found(event.clone())
}

fn newest_of(events: &[Event], author: &PublicKey, d: &str) -> Option<Event> {
    let mut slot = None;
    for event in events {
        keep_newest_site(&mut slot, event.clone(), author, d, KIND, NOW);
    }
    slot
}

#[test]
fn only_a_valid_plausible_site_event_of_the_site_is_kept_as_newest() {
    let k = keys();
    let author = k.public_key();
    let older = make_site_event(&k, KIND, "a.example", CID_B, 100);
    let newer = make_site_event(&k, KIND, "a.example", CID_B, 300);
    let other_site = make_site_event(&k, KIND, "b.example", CID_A, 400);
    let too_far_ahead = make_site_event(&k, KIND, "a.example", CID_B, NOW + 10_000);
    let by_someone_else = make_site_event(&keys(), KIND, "a.example", CID_B, 400);
    let other_kind = make_site_event(&k, KIND + 1, "a.example", CID_B, 400);
    let mut bad_signature = make_site_event(&k, KIND, "a.example", CID_B, 400);
    bad_signature.sig = older.sig;

    assert_eq!(
        newest_of(&[newer.clone(), older.clone()], &author, "a.example"),
        Some(newer.clone())
    );
    assert_eq!(
        newest_of(&[older.clone(), newer.clone()], &author, "a.example"),
        Some(newer)
    );
    assert_eq!(
        newest_of(
            &[
                other_site,
                too_far_ahead,
                by_someone_else,
                other_kind,
                bad_signature
            ],
            &author,
            "a.example"
        ),
        None
    );
}

#[test]
fn a_held_site_goes_only_to_the_relays_that_answered_without_it() {
    let k = keys();
    let held = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let older = make_site_event(&k, KIND, "a.example", CID_B, 100);

    let answers = vec![
        relay("same", vec![found(&held)]),
        relay("older", vec![found(&older)]),
        relay("none", vec![SiteAnswer::Missing]),
        relay("failed", vec![SiteAnswer::Unanswered]),
        relay("short", Vec::new()),
    ];

    let plan = plan_site_resends(&[&held], &answers);
    assert_eq!(
        names(&plan[0]),
        vec!["wss://older.example", "wss://none.example"]
    );
}

#[test]
fn a_held_site_is_not_sent_anywhere_once_any_relay_holds_a_newer_version() {
    let k = keys();
    let held = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let newer = make_site_event(&k, KIND, "a.example", CID_B, 300);
    let newest = make_site_event(&k, KIND, "a.example", CID_B, 400);

    let answers = vec![
        relay("none", vec![SiteAnswer::Missing]),
        relay("newer", vec![found(&newer)]),
        relay("newest", vec![found(&newest)]),
        relay("same", vec![found(&held)]),
    ];
    assert_eq!(
        plan_site_resends(&[&held], &answers),
        vec![ResendPlan::Superseded(&newest)]
    );
}

#[test]
fn a_rewrite_at_the_same_time_counts_as_newer_only_with_a_lower_id() {
    let k = keys();
    let x = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let y = make_site_event(&k, KIND, "a.example", CID_B, 200);
    let (low, high) = if x.id < y.id { (x, y) } else { (y, x) };

    let has_low = vec![relay("r", vec![found(&low)])];
    assert_eq!(
        plan_site_resends(&[&high], &has_low),
        vec![ResendPlan::Superseded(&low)]
    );
    let has_high = vec![relay("r", vec![found(&high)])];
    assert_eq!(
        names(&plan_site_resends(&[&low], &has_high)[0]),
        vec!["wss://r.example"]
    );
}

#[test]
fn each_held_site_is_planned_from_its_own_answer() {
    let k = keys();
    let a = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let b = make_site_event(&k, KIND, "b.example", CID_A, 200);
    let answers = vec![relay("r", vec![found(&a), SiteAnswer::Missing])];
    let plan = plan_site_resends(&[&a, &b], &answers);
    assert!(names(&plan[0]).is_empty());
    assert_eq!(names(&plan[1]), vec!["wss://r.example"]);
}

fn asked(held: &Event) -> (HashSet<String>, HashSet<String>) {
    let d = held.tags.identifier().unwrap();
    (
        HashSet::from([held.id.to_hex()]),
        HashSet::from([site_coordinate(KIND, &held.pubkey, &d).to_string()]),
    )
}

fn withdrawal_by(held: &Event, deletions: &[Event]) -> Option<Withdrawal> {
    let (ids, coordinates) = asked(held);
    let mut index = Withdrawals::default();
    for deletion in deletions {
        index.note(deletion, &ids, &coordinates, NOW);
    }
    index.of(held, KIND)
}

#[test]
fn deletions_by_the_author_withdraw_the_held_version() {
    let k = keys();
    let held = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let coordinate = format!("{KIND}:{}:a.example", k.public_key().to_hex());
    let id = held.id.to_hex();

    assert_eq!(
        withdrawal_by(&held, &[deletion(&k, &[["e", &id]], 150)]),
        Some(Withdrawal {
            by_id: true,
            through: None
        })
    );
    assert_eq!(
        withdrawal_by(
            &held,
            &[
                deletion(&k, &[["a", &coordinate]], 200),
                deletion(&k, &[["a", &coordinate]], 250),
                deletion(&k, &[["a", &coordinate]], 210),
            ]
        ),
        Some(Withdrawal {
            by_id: false,
            through: Some(250)
        })
    );

    let by_address_earlier = deletion(&k, &[["a", &coordinate]], 199);
    let other_id = deletion(
        &k,
        &[["e", &EventId::from_byte_array([0; 32]).to_hex()]],
        300,
    );
    let other_site = deletion(
        &k,
        &[[
            "a",
            &format!("{KIND}:{}:b.example", k.public_key().to_hex()),
        ]],
        300,
    );
    let other_kind = deletion(
        &k,
        &[[
            "a",
            &format!("{}:{}:a.example", KIND + 1, k.public_key().to_hex()),
        ]],
        300,
    );
    let stranger = keys();
    let by_someone_else = deletion(&stranger, &[["e", &id], ["a", &coordinate]], 300);
    let not_a_deletion = EventBuilder::new(Kind::TextNote, "")
        .tag(Tag::parse(["e", &id]).unwrap())
        .finalize(&k)
        .unwrap();
    assert_eq!(
        withdrawal_by(
            &held,
            &[
                by_address_earlier,
                other_id,
                other_site,
                other_kind,
                by_someone_else,
                not_a_deletion,
            ]
        ),
        None
    );
}

#[test]
fn a_deletion_dated_too_far_ahead_withdraws_by_id_and_by_address_up_to_the_skew() {
    let k = keys();
    let held = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let (_, coordinates) = asked(&held);
    let coordinate = coordinates.iter().next().unwrap();
    let id = held.id.to_hex();

    assert_eq!(
        withdrawal_by(&held, &[deletion(&k, &[["e", &id]], NOW + 10_000)]),
        Some(Withdrawal {
            by_id: true,
            through: None
        })
    );
    assert_eq!(
        withdrawal_by(&held, &[deletion(&k, &[["a", coordinate]], NOW + 10_000)]),
        Some(Withdrawal {
            by_id: false,
            through: Some(NOW + MAX_FUTURE_SKEW)
        })
    );
}

#[test]
fn a_note_tells_which_asked_tags_matched() {
    let k = keys();
    let held = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let (ids, coordinates) = asked(&held);
    let coordinate = coordinates.iter().next().unwrap().clone();
    let mut index = Withdrawals::default();

    let both = deletion(&k, &[["e", &held.id.to_hex()], ["a", &coordinate]], 300);
    assert_eq!(
        index.note(&both, &ids, &coordinates, NOW),
        DeletionMatch {
            by_id: true,
            by_address: true
        }
    );
    let unrelated = deletion(&k, &[["e", &"0".repeat(64)]], 300);
    assert_eq!(
        index.note(&unrelated, &ids, &coordinates, NOW),
        DeletionMatch::default()
    );
}

#[test]
fn merged_indexes_keep_every_id_and_the_latest_address_deletion() {
    let k = keys();
    let held = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let (ids, coordinates) = asked(&held);
    let coordinate = coordinates.iter().next().unwrap().clone();
    let mut first = Withdrawals::default();
    first.note(
        &deletion(&k, &[["a", &coordinate]], 300),
        &ids,
        &coordinates,
        NOW,
    );
    let mut second = Withdrawals::default();
    second.note(
        &deletion(&k, &[["a", &coordinate]], 250),
        &ids,
        &coordinates,
        NOW,
    );
    second.note(
        &deletion(&k, &[["e", &held.id.to_hex()]], 100),
        &ids,
        &coordinates,
        NOW,
    );
    first.merge(second);
    assert_eq!(
        first.of(&held, KIND),
        Some(Withdrawal {
            by_id: true,
            through: Some(300)
        })
    );
}

#[test]
fn only_a_valid_signed_site_event_of_the_key_is_sent_again() {
    let k = keys();
    let held = make_site_event(&k, KIND, "a.example", CID_A, 200);
    let saved = |event: &Event, author: &PublicKey, d: &str, kind: u16| {
        saved_site_event(event, author, d, kind).map(|site| site.id)
    };
    assert_eq!(
        saved(&held, &k.public_key(), "a.example", KIND),
        Some(held.id)
    );
    assert_eq!(saved(&held, &keys().public_key(), "a.example", KIND), None);
    assert_eq!(saved(&held, &k.public_key(), "b.example", KIND), None);
    assert_eq!(saved(&held, &k.public_key(), "a.example", KIND + 1), None);

    let mut forged = held.clone();
    forged.content = "changed".into();
    assert_eq!(saved(&forged, &k.public_key(), "a.example", KIND), None);
    let mut bad_signature = held.clone();
    bad_signature.sig = make_site_event(&k, KIND, "a.example", CID_B, 200).sig;
    assert_eq!(
        saved(&bad_signature, &k.public_key(), "a.example", KIND),
        None
    );
}
