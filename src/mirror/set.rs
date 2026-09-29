use std::collections::HashSet;

use nostr_sdk::prelude::*;

use crate::nostr::FOLLOW_SET_KIND;

const DEFAULT_TITLE: &str = "SWING mirror list";

// NIP-01 keeps the later created_at, so an edit must outdate a set signed on a clock that runs ahead.
pub(super) fn next_created_at(now: u64, previous: Option<&Event>) -> u64 {
    previous.map_or(now, |ev| now.max(ev.created_at.as_secs().saturating_add(1)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorSet {
    other_tags: Vec<Tag>,
    p_tags: Vec<Tag>,
    content: String,
}

impl MirrorSet {
    pub fn empty() -> Self {
        Self {
            other_tags: vec![Tag::custom("title", [DEFAULT_TITLE.to_string()])],
            p_tags: Vec::new(),
            content: String::new(),
        }
    }

    pub fn from_event(event: &Event) -> Self {
        let mut other_tags = Vec::new();
        let mut p_tags = Vec::new();
        for tag in event.tags.iter() {
            match tag.kind() {
                "p" => p_tags.push(tag.clone()),
                "d" => {}
                _ => other_tags.push(tag.clone()),
            }
        }
        Self {
            other_tags,
            p_tags,
            content: event.content.clone(),
        }
    }

    pub fn pubkeys(&self) -> Vec<PublicKey> {
        self.p_tags
            .iter()
            .filter_map(|t| t.content().and_then(|s| PublicKey::parse(s).ok()))
            .collect()
    }

    pub fn title(&self) -> Option<&str> {
        self.other_tags
            .iter()
            .find(|t| t.kind() == "title")
            .and_then(|t| t.content())
    }

    pub fn add(&mut self, keys: &[PublicKey]) -> Vec<PublicKey> {
        let mut present: HashSet<PublicKey> = self.pubkeys().into_iter().collect();
        let mut added = Vec::new();
        for &key in keys {
            if present.insert(key) {
                self.p_tags.push(Tag::public_key(key));
                added.push(key);
            }
        }
        added
    }

    pub fn remove(&mut self, keys: &[PublicKey]) -> Vec<PublicKey> {
        let to_remove: HashSet<PublicKey> = keys.iter().copied().collect();
        let mut removed = Vec::new();
        self.p_tags.retain(
            |t| match t.content().and_then(|s| PublicKey::parse(s).ok()) {
                Some(pk) if to_remove.contains(&pk) => {
                    removed.push(pk);
                    false
                }
                _ => true,
            },
        );
        removed
    }

    pub fn build_event_builder(&self, mirror_set: &str) -> EventBuilder {
        EventBuilder::new(Kind::Custom(FOLLOW_SET_KIND), self.content.clone())
            .tag(Tag::identifier(mirror_set))
            .tags(self.other_tags.clone())
            .tags(self.p_tags.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::keys;

    fn make_follow_set(
        author: &Keys,
        d: &str,
        title: Option<&str>,
        content: &str,
        p: &[PublicKey],
    ) -> Event {
        let mut builder =
            EventBuilder::new(Kind::Custom(FOLLOW_SET_KIND), content).tag(Tag::identifier(d));
        if let Some(title) = title {
            builder = builder.tag(Tag::custom("title", [title.to_string()]));
        }
        for pk in p {
            builder = builder.tag(Tag::public_key(*pk));
        }
        builder.finalize(author).unwrap()
    }

    #[test]
    fn from_event_preserves_other_tags_and_content() {
        let author = keys();
        let p1 = keys().public_key();
        let ev = make_follow_set(&author, "swing", Some("my list"), "encrypted-blob", &[p1]);
        let set = MirrorSet::from_event(&ev);
        assert_eq!(set.content, "encrypted-blob");
        assert_eq!(set.title(), Some("my list"));
        assert_eq!(set.pubkeys(), vec![p1]);
    }

    #[test]
    fn add_is_idempotent_and_reports_no_new_keys() {
        let author = keys();
        let p1 = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        let added = set.add(&[p1]);
        assert!(added.is_empty());
        assert_eq!(set.pubkeys(), vec![p1]);
    }

    #[test]
    fn add_appends_new_keys_keeping_existing() {
        let author = keys();
        let p1 = keys().public_key();
        let p2 = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        let added = set.add(&[p1, p2]);
        assert_eq!(added, vec![p2]);
        let pubkeys = set.pubkeys();
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&p1));
        assert!(pubkeys.contains(&p2));
    }

    #[test]
    fn remove_drops_matching_keys_and_reports_absent() {
        let author = keys();
        let p1 = keys().public_key();
        let p2 = keys().public_key();
        let absent = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1, p2]);
        let mut set = MirrorSet::from_event(&ev);
        let removed = set.remove(&[p1, absent]);
        assert_eq!(removed, vec![p1]);
        assert_eq!(set.pubkeys(), vec![p2]);
    }

    #[test]
    fn remove_absent_key_is_no_op() {
        let author = keys();
        let p1 = keys().public_key();
        let absent = keys().public_key();
        let ev = make_follow_set(&author, "swing", None, "", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        let removed = set.remove(&[absent]);
        assert!(removed.is_empty());
        assert_eq!(set.pubkeys(), vec![p1]);
    }

    #[test]
    fn build_event_builder_keeps_content_and_non_p_tags_byte_for_byte() {
        let author = keys();
        let p1 = keys().public_key();
        let p2 = keys().public_key();
        let ev = make_follow_set(&author, "swing", Some("my list"), "private-content", &[p1]);
        let mut set = MirrorSet::from_event(&ev);
        set.add(&[p2]);
        let rebuilt = set.build_event_builder("swing").finalize(&author).unwrap();

        assert_eq!(rebuilt.content, "private-content");
        assert_eq!(rebuilt.tags.identifier().as_deref(), Some("swing"));
        assert_eq!(
            rebuilt
                .tags
                .iter()
                .find(|t| t.kind() == "title")
                .and_then(|t| t.content()),
            Some("my list")
        );
        let pubkeys: Vec<PublicKey> = rebuilt.tags.public_keys().collect();
        assert_eq!(pubkeys.len(), 2);
        assert!(pubkeys.contains(&p1));
        assert!(pubkeys.contains(&p2));
    }

    #[test]
    fn next_created_at_outdates_a_future_dated_previous_set() {
        let author = keys();
        let mut previous = make_follow_set(&author, "swing", None, "", &[]);
        assert_eq!(next_created_at(100, None), 100);
        previous.created_at = Timestamp::from_secs(50);
        assert_eq!(next_created_at(100, Some(&previous)), 100);
        previous.created_at = Timestamp::from_secs(100);
        assert_eq!(next_created_at(100, Some(&previous)), 101);
        previous.created_at = Timestamp::from_secs(700);
        assert_eq!(next_created_at(100, Some(&previous)), 701);
    }

    #[test]
    fn empty_mirror_set_has_default_title_and_no_pubkeys() {
        let set = MirrorSet::empty();
        assert_eq!(set.title(), Some(DEFAULT_TITLE));
        assert!(set.pubkeys().is_empty());
    }
}
