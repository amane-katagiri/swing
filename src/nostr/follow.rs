use std::collections::HashSet;

use nostr_sdk::prelude::*;

use super::{budget, is_newer_replaceable, plausible_at};

pub fn is_follow_set_of(event: &Event, author: &PublicKey, mirror_set: &str) -> bool {
    event.kind == Kind::Custom(30000)
        && event.pubkey == *author
        && event.tags.identifier().as_deref() == Some(mirror_set)
        && event.verify().is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowSetChoice {
    pub event: Event,
    pub save: bool,
    pub republish: bool,
}

pub fn choose_follow_set(
    fetched: Option<Event>,
    fetch_succeeded: bool,
    stored: Option<Event>,
    now: u64,
) -> Option<FollowSetChoice> {
    // The stored copy is filtered too, or a poisoned one could never be displaced by a plausible fetch.
    let fetched = fetched.filter(|e| plausible_at(e.created_at.as_secs(), now));
    let stored = stored.filter(|e| plausible_at(e.created_at.as_secs(), now));
    match (fetched, stored) {
        (None, None) => None,
        (Some(event), None) => Some(FollowSetChoice {
            event,
            save: true,
            republish: false,
        }),
        (None, Some(event)) => Some(FollowSetChoice {
            event,
            save: false,
            republish: fetch_succeeded,
        }),
        (Some(fetched), Some(stored)) if fetched.id == stored.id => Some(FollowSetChoice {
            event: stored,
            save: false,
            republish: false,
        }),
        (Some(fetched), Some(stored)) if is_newer_replaceable(&fetched, &stored) => {
            Some(FollowSetChoice {
                event: fetched,
                save: true,
                republish: false,
            })
        }
        (Some(_), Some(stored)) => Some(FollowSetChoice {
            event: stored,
            save: false,
            republish: true,
        }),
    }
}

pub fn extract_follow_set_pubkeys(event: &Event) -> Vec<PublicKey> {
    follow_set_pubkeys_capped(event).0
}

pub fn follow_set_pubkeys_capped(event: &Event) -> (Vec<PublicKey>, bool) {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for pk in event.tags.public_keys() {
        if !seen.insert(pk) {
            continue;
        }
        if out.len() == budget::MAX_FOLLOW_SET_ENTRIES {
            return (out, true);
        }
        out.push(pk);
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::super::MAX_FUTURE_SKEW;
    use super::super::fixtures::follow_set;
    use super::*;
    use crate::test_support::keys;

    #[test]
    fn extracts_follow_set_pubkeys() {
        let author = keys();
        let target1 = keys().public_key();
        let target2 = keys().public_key();
        let ev = EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier("site-mirror"))
            .tag(Tag::public_key(target1))
            .tag(Tag::public_key(target2))
            .finalize(&author)
            .unwrap();
        let pubkeys = extract_follow_set_pubkeys(&ev);
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&target1));
        assert!(pubkeys.contains(&target2));
    }

    #[test]
    fn follow_set_pubkeys_capped_dedups_repeated_p_tags() {
        let author = keys();
        let target = keys().public_key();
        let ev = EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier("site-mirror"))
            .tag(Tag::public_key(target))
            .tag(Tag::public_key(target))
            .finalize(&author)
            .unwrap();

        let (pubkeys, truncated) = follow_set_pubkeys_capped(&ev);

        assert_eq!(pubkeys, vec![target]);
        assert!(!truncated);
    }

    #[test]
    fn follow_set_pubkeys_capped_stops_at_the_budget() {
        let author = keys();
        let first = keys().public_key();
        let mut builder = EventBuilder::new(Kind::Custom(30000), "")
            .tag(Tag::identifier("site-mirror"))
            .tag(Tag::public_key(first));
        let extra: Vec<PublicKey> = (0..budget::MAX_FOLLOW_SET_ENTRIES)
            .map(|_| Keys::generate().public_key())
            .collect();
        for pk in &extra {
            builder = builder.tag(Tag::public_key(*pk));
        }
        let ev = builder.finalize(&author).unwrap();

        let (pubkeys, truncated) = follow_set_pubkeys_capped(&ev);

        assert!(truncated);
        assert_eq!(pubkeys.len(), budget::MAX_FOLLOW_SET_ENTRIES);
        assert_eq!(pubkeys[0], first);
        assert_eq!(
            extract_follow_set_pubkeys(&ev).len(),
            budget::MAX_FOLLOW_SET_ENTRIES
        );
    }

    #[test]
    fn follow_set_identity_checks_kind_author_d_and_signature() {
        let k = keys();
        let ev = follow_set(&k, "swing", 100, "");
        assert!(is_follow_set_of(&ev, &k.public_key(), "swing"));
        assert!(!is_follow_set_of(&ev, &k.public_key(), "other"));
        assert!(!is_follow_set_of(&ev, &keys().public_key(), "swing"));

        let mut tampered = ev.clone();
        tampered.content = "changed".to_string();
        assert!(!is_follow_set_of(&tampered, &k.public_key(), "swing"));

        let site = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("swing"))
            .finalize(&k)
            .unwrap();
        assert!(!is_follow_set_of(&site, &k.public_key(), "swing"));
    }

    #[test]
    fn choose_follow_set_prefers_the_newest_and_repairs_relays() {
        let k = keys();
        let old = follow_set(&k, "swing", 100, "old");
        let new = follow_set(&k, "swing", 200, "new");

        assert_eq!(choose_follow_set(None, true, None, 1000), None);

        let c = choose_follow_set(Some(new.clone()), true, None, 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, true, false));

        let c = choose_follow_set(Some(new.clone()), true, Some(old.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, true, false));

        let c = choose_follow_set(Some(old.clone()), true, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, true));

        let c = choose_follow_set(Some(new.clone()), true, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, false));

        let c = choose_follow_set(None, true, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, true));

        let c = choose_follow_set(None, false, Some(new.clone()), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (new.id, false, false));
    }

    #[test]
    fn choose_follow_set_drops_an_implausible_future_fetch_and_keeps_stored() {
        let k = keys();
        let stored = follow_set(&k, "swing", 100, "stored");
        let poisoned_fetch = follow_set(&k, "swing", 1000 + MAX_FUTURE_SKEW + 1, "poisoned");

        let c = choose_follow_set(
            Some(poisoned_fetch.clone()),
            true,
            Some(stored.clone()),
            1000,
        )
        .unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (stored.id, false, true));
    }

    #[test]
    fn choose_follow_set_drops_a_poisoned_stored_copy_and_saves_the_fetched_one() {
        let k = keys();
        let poisoned_stored = follow_set(&k, "swing", 1000 + MAX_FUTURE_SKEW + 1, "poisoned");
        let fetched = follow_set(&k, "swing", 100, "fetched");

        let c =
            choose_follow_set(Some(fetched.clone()), true, Some(poisoned_stored), 1000).unwrap();
        assert_eq!((c.event.id, c.save, c.republish), (fetched.id, true, false));
    }

    #[test]
    fn choose_follow_set_returns_none_when_only_a_poisoned_stored_copy_exists() {
        let k = keys();
        let poisoned_stored = follow_set(&k, "swing", 1000 + MAX_FUTURE_SKEW + 1, "poisoned");

        assert_eq!(
            choose_follow_set(None, true, Some(poisoned_stored), 1000),
            None
        );
    }
}
