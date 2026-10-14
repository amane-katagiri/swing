use std::collections::HashSet;

use nostr_sdk::prelude::*;

use super::{FOLLOW_SET_KIND, budget, is_newer_replaceable, plausible_at};

pub fn is_follow_set_of(event: &Event, author: &PublicKey, mirror_set: &str) -> bool {
    event.kind == Kind::Custom(FOLLOW_SET_KIND)
        && event.pubkey == *author
        && event.tags.identifier().as_deref() == Some(mirror_set)
}

pub fn is_saved_follow_set_of(event: &Event, author: &PublicKey, mirror_set: &str) -> bool {
    is_follow_set_of(event, author, mirror_set) && event.verify().is_ok()
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
    use super::*;
    use crate::test_support::{follow_set, keys};

    #[test]
    fn extracts_follow_set_pubkeys() {
        let author = keys();
        let target1 = keys().public_key();
        let target2 = keys().public_key();
        let ev = follow_set(&author, "site-mirror", None, "", &[target1, target2], 0);
        let pubkeys = extract_follow_set_pubkeys(&ev);
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&target1));
        assert!(pubkeys.contains(&target2));
    }

    #[test]
    fn follow_set_pubkeys_capped_dedups_repeated_p_tags() {
        let author = keys();
        let target = keys().public_key();
        let ev = follow_set(&author, "site-mirror", None, "", &[target, target], 0);

        let (pubkeys, truncated) = follow_set_pubkeys_capped(&ev);

        assert_eq!(pubkeys, vec![target]);
        assert!(!truncated);
    }

    #[test]
    fn follow_set_pubkeys_capped_stops_at_the_budget() {
        let author = keys();
        let first = keys().public_key();
        let p: Vec<PublicKey> = std::iter::once(first)
            .chain((0..budget::MAX_FOLLOW_SET_ENTRIES).map(|_| Keys::generate().public_key()))
            .collect();
        let ev = follow_set(&author, "site-mirror", None, "", &p, 0);

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
        let ev = follow_set(&k, "swing", None, "", &[], 100);
        assert!(is_follow_set_of(&ev, &k.public_key(), "swing"));
        assert!(!is_follow_set_of(&ev, &k.public_key(), "other"));
        assert!(!is_follow_set_of(&ev, &keys().public_key(), "swing"));

        let mut tampered = ev.clone();
        tampered.content = "changed".to_string();
        assert!(is_follow_set_of(&tampered, &k.public_key(), "swing"));
        assert!(is_saved_follow_set_of(&ev, &k.public_key(), "swing"));
        assert!(!is_saved_follow_set_of(&tampered, &k.public_key(), "swing"));

        let site = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("swing"))
            .finalize(&k)
            .unwrap();
        assert!(!is_follow_set_of(&site, &k.public_key(), "swing"));
    }

    #[test]
    fn choose_follow_set_prefers_the_newest_plausible_copy_and_repairs_relays() {
        let k = keys();
        let old = follow_set(&k, "swing", None, "old", &[], 100);
        let new = follow_set(&k, "swing", None, "new", &[], 200);
        let poisoned = follow_set(
            &k,
            "swing",
            None,
            "poisoned",
            &[],
            1000 + MAX_FUTURE_SKEW + 1,
        );
        let cases = [
            ("nothing", None, true, None, None),
            (
                "fetched only",
                Some(&new),
                true,
                None,
                Some((&new, true, false)),
            ),
            (
                "fetched newer",
                Some(&new),
                true,
                Some(&old),
                Some((&new, true, false)),
            ),
            (
                "stored newer",
                Some(&old),
                true,
                Some(&new),
                Some((&new, false, true)),
            ),
            (
                "same copy",
                Some(&new),
                true,
                Some(&new),
                Some((&new, false, false)),
            ),
            (
                "missing on relays",
                None,
                true,
                Some(&new),
                Some((&new, false, true)),
            ),
            (
                "fetch failed",
                None,
                false,
                Some(&new),
                Some((&new, false, false)),
            ),
            (
                "fetched from the future",
                Some(&poisoned),
                true,
                Some(&old),
                Some((&old, false, true)),
            ),
            (
                "stored from the future",
                Some(&new),
                true,
                Some(&poisoned),
                Some((&new, true, false)),
            ),
            (
                "only a stored copy from the future",
                None,
                true,
                Some(&poisoned),
                None,
            ),
        ];
        for (case, fetched, fetch_ok, stored, want) in cases {
            let got = choose_follow_set(fetched.cloned(), fetch_ok, stored.cloned(), 1000)
                .map(|c| (c.event.id, c.save, c.republish));
            let want =
                want.map(|(ev, save, republish): (&Event, bool, bool)| (ev.id, save, republish));
            assert_eq!(got, want, "{case}");
        }
    }
}
