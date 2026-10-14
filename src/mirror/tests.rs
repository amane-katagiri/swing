use super::*;
use crate::test_support::{follow_set, keys, version};

#[test]
fn ensure_within_follow_set_cap_allows_exactly_the_budget() {
    assert!(ensure_within_follow_set_cap(nostr::budget::MAX_FOLLOW_SET_ENTRIES).is_ok());
    let err = ensure_within_follow_set_cap(nostr::budget::MAX_FOLLOW_SET_ENTRIES + 1).unwrap_err();
    assert!(err.to_string().contains("over the"), "{err}");
}

#[test]
fn parse_pubkey_input_reads_hex_npub_and_nprofile() {
    let k = keys();
    let nprofile = Nip19Profile::new(k.public_key(), Vec::<RelayUrl>::new())
        .to_bech32()
        .unwrap();
    for (case, input) in [
        ("hex", k.public_key().to_hex()),
        ("npub", k.public_key().to_bech32().unwrap()),
        ("nprofile", nprofile),
    ] {
        assert_eq!(
            parse_pubkey_input(&input).unwrap(),
            k.public_key(),
            "{case}"
        );
    }
    for bad in ["not-a-key", ""] {
        assert!(parse_pubkey_input(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn parse_pubkey_inputs_dedupes_preserving_order() {
    let k1 = keys().public_key();
    let k2 = keys().public_key();
    let inputs = vec![k1.to_hex(), k2.to_hex(), k1.to_hex()];
    let parsed = parse_pubkey_inputs(&inputs).unwrap();
    assert_eq!(parsed, vec![k1, k2]);
}

#[test]
fn unfollowed_sites_lists_the_latest_version_of_accounts_outside_the_follow_set() {
    let mut state = State::default();
    let record = |cid: &str, created_at| version(cid, 1, created_at);
    state.apply_store(&state::site_key("aa", "x.example"), record("old", 1));
    state.apply_store(&state::site_key("aa", "x.example"), record("new", 2));
    state.apply_store(&state::site_key("aa", "y:z"), record("yz", 1));
    state.apply_store(&state::site_key("bb", "x.example"), record("kept", 1));
    let targets = BTreeSet::from(["bb".to_string()]);

    let unfollowed = unfollowed_sites(&state, &targets);

    assert_eq!(unfollowed.keys().collect::<Vec<_>>(), vec!["aa"]);
    let sites: Vec<(&str, &str)> = unfollowed["aa"]
        .iter()
        .map(|(d, v)| (d.as_str(), v.cid.as_str()))
        .collect();
    assert_eq!(sites, vec![("x.example", "new"), ("y:z", "yz")]);
}

#[test]
fn unfollowed_accounts_take_the_site_fields_from_the_held_event() {
    use crate::test_support::{CID_A, CID_B, make_site_event};
    let author = keys();
    let hex = author.public_key().to_hex();
    let record = |cid: &str, event: Option<Event>| VersionRecord {
        event,
        ..version(cid, 1, 10)
    };
    let mut state = State::default();
    state.apply_store(
        &state::site_key(&hex, "held.example"),
        record(
            CID_A,
            Some(make_site_event(&author, 35980, "held.example", CID_A, 10)),
        ),
    );
    state.apply_store(
        &state::site_key(&hex, "mismatch.example"),
        record(
            CID_B,
            Some(make_site_event(
                &author,
                35980,
                "mismatch.example",
                CID_A,
                10,
            )),
        ),
    );
    state.apply_store(&state::site_key(&hex, "none.example"), record(CID_A, None));

    let accounts = unfollowed_accounts(&state, &[], 35980).unwrap();

    let urls: Vec<(&str, Option<&str>)> = accounts[0]
        .sites
        .iter()
        .map(|s| (s.d.as_str(), s.url.as_deref()))
        .collect();
    assert_eq!(
        urls,
        vec![
            ("held.example", Some("https://example.com/")),
            ("mismatch.example", None),
            ("none.example", None),
        ]
    );
}

#[test]
fn followed_accounts_reports_the_latest_stored_version_while_the_event_is_not_stored() {
    let pk = keys().public_key();
    let hex = pk.to_hex();
    let record = |cid: &str, created_at| version(cid, created_at, created_at);
    let mut state = State::default();
    state.apply_store(&state::site_key(&hex, "pending.example"), record("old", 1));
    state.apply_store(
        &state::site_key(&hex, "pending.example"),
        record("older", 0),
    );
    state.apply_store(&state::site_key(&hex, "stored.example"), record("old", 1));
    state.apply_store(
        &state::site_key(&hex, "stored.example"),
        record(crate::test_support::CID_A, 2),
    );
    let events = [
        crate::test_support::site_event_fixture(
            pk,
            "pending.example",
            crate::test_support::CID_A,
            3,
        ),
        crate::test_support::site_event_fixture(
            pk,
            "stored.example",
            crate::test_support::CID_A,
            2,
        ),
        crate::test_support::site_event_fixture(pk, "new.example", crate::test_support::CID_A, 3),
    ];
    let latest: Vec<&nostr::SiteEvent> = events.iter().collect();

    let accounts = followed_accounts(&[pk], &latest, &state, &None).unwrap();

    let rows: Vec<(&str, bool, Option<&str>)> = accounts[0]
        .sites
        .iter()
        .map(|s| {
            (
                s.d.as_str(),
                s.stored,
                s.previous.as_ref().map(|v| v.cid.as_str()),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("new.example", false, None),
            ("pending.example", false, Some("old")),
            ("stored.example", true, None),
        ]
    );
}

#[test]
fn newest_follow_set_prefers_the_newest_plausible_copy() {
    let k = keys();
    let old = follow_set(&k, "swing", None, "", &[], 100);
    let new = follow_set(&k, "swing", None, "", &[], 200);
    let poisoned = follow_set(
        &k,
        "swing",
        None,
        "",
        &[],
        1000 + nostr::MAX_FUTURE_SKEW + 1,
    );
    let cases = [
        ("saved newer", Some(&old), Some(&new), Some(&new), true),
        ("fetched newer", Some(&new), Some(&old), Some(&new), false),
        ("saved only", None, Some(&old), Some(&old), true),
        ("fetched only", Some(&old), None, Some(&old), false),
        ("neither", None, None, None, false),
        (
            "saved from the future",
            Some(&old),
            Some(&poisoned),
            Some(&old),
            false,
        ),
        (
            "only saved from the future",
            None,
            Some(&poisoned),
            None,
            false,
        ),
    ];
    for (case, fetched, saved, want, noted) in cases {
        let (ev, note) = newest_follow_set(fetched.cloned(), saved.cloned(), 1000);
        assert_eq!(ev.map(|e| e.id), want.map(|e| e.id), "{case}");
        assert_eq!(note.is_some(), noted, "{case}");
    }
}

struct MirrorRig {
    _relay: LocalRelay,
    client: RelayClient,
    config: Config,
}

async fn mirror_rig() -> MirrorRig {
    let (relay, url) = crate::test_support::relay_holding(&[]).await;
    let toml = format!("[nostr]\nrelays = [\"{url}\"]\n");
    let mut config = crate::config::build_config_from_str(&toml, |_| None).unwrap();
    config.agent.state_dir = crate::test_support::unused_dir();
    let client = RelayClient::connect(Signer::Local(keys()), &[url])
        .await
        .unwrap();
    MirrorRig {
        _relay: relay,
        client,
        config,
    }
}

impl MirrorRig {
    async fn on_relay(&self) -> Option<Event> {
        self.client
            .fetch_follow_set(&self.config.nostr.mirror_set)
            .await
            .unwrap()
    }
}

fn hexes(keys: &[PublicKey]) -> Vec<String> {
    keys.iter().map(PublicKey::to_hex).collect()
}

fn sorted(mut keys: Vec<PublicKey>) -> Vec<PublicKey> {
    keys.sort();
    keys
}

#[tokio::test]
async fn mirror_changes_publish_only_when_the_set_changes() {
    let rig = mirror_rig().await;
    let (a, b, c) = (
        keys().public_key(),
        keys().public_key(),
        keys().public_key(),
    );

    let added = apply_add(&rig.client, &rig.config, &hexes(&[a, b]))
        .await
        .unwrap();
    assert!(!added.follow_set_found);
    assert!(added.published);
    assert_eq!(added.changed, vec![a, b]);
    assert!(
        added.relay_results.iter().all(|r| r.ok),
        "{:?}",
        added.relay_results
    );
    let first = rig
        .on_relay()
        .await
        .expect("the follow set reached the relay");
    assert_eq!(
        sorted(MirrorSet::from_event(&first).pubkeys()),
        sorted(vec![a, b])
    );

    let again = apply_add(&rig.client, &rig.config, &hexes(&[a]))
        .await
        .unwrap();
    assert!(again.follow_set_found);
    assert!(!again.published);
    assert!(again.relay_results.is_empty());
    assert_eq!(again.unchanged, vec![a]);
    assert_eq!(rig.on_relay().await.unwrap().id, first.id);

    let removed = apply_remove(&rig.client, &rig.config, &hexes(&[b, c]))
        .await
        .unwrap();
    assert!(removed.published);
    assert_eq!(removed.changed, vec![b]);
    assert_eq!(removed.unchanged, vec![c]);
    let second = rig.on_relay().await.unwrap();
    assert_eq!(MirrorSet::from_event(&second).pubkeys(), vec![a]);
    assert!(
        second.created_at > first.created_at,
        "a change within the same second must still outdate the previous set"
    );

    let none = apply_remove(&rig.client, &rig.config, &hexes(&[c]))
        .await
        .unwrap();
    assert!(!none.published);
    assert_eq!(rig.on_relay().await.unwrap().id, second.id);
}

#[tokio::test]
async fn growing_the_set_past_the_cap_is_refused_before_publishing() {
    let rig = mirror_rig().await;
    let first = keys().public_key();
    apply_add(&rig.client, &rig.config, &hexes(&[first]))
        .await
        .unwrap();
    let before = rig.on_relay().await.unwrap();

    let many: Vec<PublicKey> = (0..nostr::budget::MAX_FOLLOW_SET_ENTRIES)
        .map(|_| keys().public_key())
        .collect();
    let err = apply_add(&rig.client, &rig.config, &hexes(&many))
        .await
        .unwrap_err();
    let cap = err
        .downcast_ref::<FollowSetCapExceeded>()
        .unwrap_or_else(|| panic!("{err:#}"));
    assert_eq!(cap.total, nostr::budget::MAX_FOLLOW_SET_ENTRIES + 1);
    assert_eq!(rig.on_relay().await.unwrap().id, before.id);
}
