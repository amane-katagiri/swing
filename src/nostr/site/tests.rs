use super::super::MAX_FUTURE_SKEW;
use super::*;
use crate::test_support::{
    CID_A, CID_B, keys, make_site_event, site_event_fixture, site_event_with,
};

#[test]
fn site_event_content_is_the_message() {
    let k = keys();
    let ev = build_site_event_builder(
        35980,
        &SiteFields {
            d: "example.com",
            cid: CID_A,
            message: Some("Add posts"),
            ..Default::default()
        },
    )
    .finalize(&k)
    .unwrap();
    assert_eq!(ev.content, "Add posts");
    let parsed = parse_site_event(&ev, 35980).unwrap();
    assert_eq!(parsed.message.as_deref(), Some("Add posts"));
}

#[test]
fn a_size_tag_must_be_plain_decimal_digits() {
    let k = keys();
    for (raw, want) in [
        ("1234", Some(1234)),
        ("+1234", None),
        ("-1", None),
        (" 1234", None),
        ("", None),
        ("1e3", None),
    ] {
        let ev = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier("example.com"))
            .tag(Tag::custom("cid", [CID_A.to_string()]))
            .tag(Tag::custom("size", [raw.to_string()]))
            .finalize(&k)
            .unwrap();
        assert_eq!(parse_site_event(&ev, 35980).unwrap().size, want, "{raw}");
    }
}

#[test]
fn parses_valid_site_event() {
    let k = keys();
    let ev = make_site_event(&k, 35980, "example.com", CID_A, 1000);
    let parsed = parse_site_event(&ev, 35980).unwrap();
    assert_eq!(parsed.d, "example.com");
    assert_eq!(parsed.cid, CID_A);
    assert_eq!(parsed.url.as_deref(), Some("https://example.com/"));
    assert_eq!(parsed.size, Some(1234));
    assert_eq!(parsed.created_at, 1000);
    assert_eq!(parsed.pubkey, k.public_key());
    assert_eq!(parsed.message, None);
    assert_eq!(parsed.title, None);
    assert!(
        ev.tags
            .iter()
            .any(|t| t.as_slice() == ["alt", "SWING site announcement: example.com"])
    );
}

#[test]
fn accepts_cidv0_and_canonicalizes_it() {
    let k = keys();
    let ev = make_site_event(&k, 35980, "example.com", CID_B, 1000);
    let parsed = parse_site_event(&ev, 35980).unwrap();
    assert_eq!(
        parsed.cid,
        "bafybeie5nqv6kd3qnfjuphmab6atx72bbz674e35siysg2di3q5jltctqq"
    );
}

#[test]
fn canonical_cid_normalizes_every_spelling_to_cidv1_base32() {
    for (case, input, want) in [
        (
            "cidv0",
            CID_B,
            "bafybeie5nqv6kd3qnfjuphmab6atx72bbz674e35siysg2di3q5jltctqq",
        ),
        (
            "base58 cidv1",
            "zdj7Wic6KcJAfWz1c9o4M6kq9Lwd5BfbxkVafnrojaaGiSFxM",
            CID_A,
        ),
        ("already canonical", CID_A, CID_A),
    ] {
        assert_eq!(canonical_cid(input).unwrap(), want, "{case}");
    }
    for (case, input, want) in [
        ("garbage", "not-a-cid", "invalid cid tag"),
        (
            "raw codec",
            "bafkreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
            "is not dag-pb",
        ),
        (
            "dag-cbor codec",
            "bafyreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
            "is not dag-pb",
        ),
    ] {
        let err = canonical_cid(input).unwrap_err().to_string();
        assert!(err.contains(want), "{case}: {err}");
    }
}

fn event_with_tags(k: &Keys, tags: &[(&str, &str)]) -> Event {
    let mut builder = EventBuilder::new(Kind::Custom(35980), "");
    for (name, value) in tags {
        builder = builder.tag(Tag::custom(*name, [value.to_string()]));
    }
    builder.finalize(k).unwrap()
}

#[test]
fn invalid_site_events_are_rejected() {
    let k = keys();
    let long_d = "a".repeat(254);
    let cases = [
        (
            "wrong kind",
            make_site_event(&k, 1, "example.com", CID_A, 1000),
            "unexpected kind",
        ),
        (
            "missing cid",
            event_with_tags(&k, &[("d", "example.com")]),
            "missing cid tag",
        ),
        (
            "invalid cid",
            event_with_tags(&k, &[("d", "example.com"), ("cid", "not-a-cid")]),
            "invalid cid tag",
        ),
        (
            "raw codec cid",
            make_site_event(
                &k,
                35980,
                "example.com",
                "bafkreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
                1000,
            ),
            "is not dag-pb",
        ),
        (
            "missing d",
            event_with_tags(&k, &[("cid", CID_A)]),
            "missing d tag",
        ),
        (
            "empty d",
            event_with_tags(&k, &[("d", ""), ("cid", CID_A)]),
            "empty d tag",
        ),
        (
            "d too long",
            event_with_tags(&k, &[("d", &long_d), ("cid", CID_A)]),
            "d tag too long",
        ),
        (
            "d with control characters",
            event_with_tags(&k, &[("d", "example.com\ncom"), ("cid", CID_A)]),
            "control or invisible",
        ),
    ];
    for (case, ev, want) in cases {
        let err = parse_site_event(&ev, 35980).unwrap_err().to_string();
        assert!(err.contains(want), "{case}: {err}");
    }
}

#[test]
fn accepts_d_tag_at_max_length() {
    let k = keys();
    let max_d = "a".repeat(253);
    let ev = EventBuilder::new(Kind::Custom(35980), "")
        .tag(Tag::custom("d", [max_d.clone()]))
        .tag(Tag::custom("cid", [CID_A.to_string()]))
        .finalize(&k)
        .unwrap();
    let parsed = parse_site_event(&ev, 35980).unwrap();
    assert_eq!(parsed.d, max_d);
}

#[test]
fn only_plain_http_urls_are_kept() {
    let k = keys();
    let control = "https://example.com/\x1b]0;pwned\x07";
    assert!(reqwest::Url::parse(control).is_ok());
    let too_long = format!("https://example.com/{}", "a".repeat(2048));
    for (case, url, want) in [
        ("not a url", "not a url", None),
        ("control characters", control, None),
        ("non-http scheme", "ftp://example.com/", None),
        ("too long", too_long.as_str(), None),
        (
            "https",
            "https://example.com/",
            Some("https://example.com/"),
        ),
    ] {
        let ev = event_with_tags(&k, &[("d", "example.com"), ("cid", CID_A), ("url", url)]);
        let parsed = parse_site_event(&ev, 35980).unwrap();
        assert_eq!(parsed.url.as_deref(), want, "{case}");
    }
}

#[test]
fn oversized_or_control_titles_are_dropped_but_the_event_is_kept() {
    let k = keys();
    let ev = build_site_event_builder(
        35980,
        &SiteFields {
            d: "example.com",
            cid: CID_A,
            title: Some("あまねけ！"),
            ..Default::default()
        },
    )
    .finalize(&k)
    .unwrap();
    let parsed = parse_site_event(&ev, 35980).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("あまねけ！"), "round trip");

    let oversized = "a".repeat(257);
    for (case, title) in [
        ("absent", None),
        ("oversized", Some(oversized.as_str())),
        ("control characters", Some("hello\nworld")),
    ] {
        let mut tags = vec![("d", "example.com"), ("cid", CID_A)];
        tags.extend(title.map(|t| ("title", t)));
        let parsed = parse_site_event(&event_with_tags(&k, &tags), 35980).unwrap();
        assert_eq!(parsed.title, None, "{case}");
    }
}

#[test]
fn cap_sites_per_author_keeps_the_first_n_by_d() {
    let a = keys().public_key();
    let b = keys().public_key();
    let sites = vec![
        site_event_fixture(a, "c.example", CID_A, 1),
        site_event_fixture(a, "a.example", CID_A, 1),
        site_event_fixture(a, "b.example", CID_A, 1),
        site_event_fixture(b, "only.example", CID_A, 1),
    ];

    let capped = cap_sites_per_author(&sites, 2);

    let a_ds: Vec<&str> = capped
        .iter()
        .filter(|s| s.pubkey == a)
        .map(|s| s.d.as_str())
        .collect();
    assert_eq!(a_ds, vec!["a.example", "b.example"]);
    assert_eq!(capped.iter().filter(|s| s.pubkey == b).count(), 1);
}

#[test]
fn select_latest_keeps_max_created_at_per_pubkey_and_d() {
    let k1 = keys();
    let k2 = keys();
    let events = vec![
        site_event_fixture(k1.public_key(), "site-a", "bafy-old", 100),
        site_event_fixture(k1.public_key(), "site-a", "bafy-new", 200),
        site_event_fixture(k1.public_key(), "site-b", "bafy-other-site", 50),
        site_event_fixture(k2.public_key(), "site-a", "bafy-k2", 999),
    ];
    let latest = select_latest(&events, 1000);
    assert_eq!(latest.len(), 3);
    assert_eq!(
        latest[&(k1.public_key().to_hex(), "site-a".to_string())].cid,
        "bafy-new"
    );
    assert_eq!(
        latest[&(k1.public_key().to_hex(), "site-b".to_string())].cid,
        "bafy-other-site"
    );
    assert_eq!(
        latest[&(k2.public_key().to_hex(), "site-a".to_string())].cid,
        "bafy-k2"
    );
}

#[test]
fn select_latest_ignores_an_implausible_future_created_at() {
    let k = keys();
    let ev = |pubkey, cid, created_at| site_event_fixture(pubkey, "site-a", cid, created_at);
    let events = vec![
        ev(k.public_key(), "bafy-plausible", 1000 + MAX_FUTURE_SKEW),
        ev(k.public_key(), "bafy-forged", 1000 + MAX_FUTURE_SKEW + 1),
    ];
    let latest = select_latest(&events, 1000);
    assert_eq!(
        latest[&(k.public_key().to_hex(), "site-a".to_string())].cid,
        "bafy-plausible"
    );
}

#[test]
fn rejects_d_tags_and_drops_titles_with_invisible_formatting_characters() {
    let k = keys();
    for c in [
        '\u{00AD}',
        '\u{200B}',
        '\u{200F}',
        '\u{2028}',
        '\u{2029}',
        '\u{202A}',
        '\u{202E}',
        '\u{2066}',
        '\u{2069}',
        '\u{FEFF}',
        '\u{061C}',
        '\u{180E}',
        '\u{2060}',
        '\u{2064}',
        '\u{FFF9}',
        '\u{FFFB}',
        '\u{E0001}',
        '\u{E007F}',
        '\u{0085}',
        '\u{009B}',
    ] {
        let d = format!("exa{c}mple.com");
        assert!(parse_site_event(&site_event_with(&k, &d, "ok", ""), 35980).is_err());
        let title = format!("My{c}Site");
        let parsed =
            parse_site_event(&site_event_with(&k, "example.com", &title, ""), 35980).unwrap();
        assert_eq!(parsed.title, None);
    }
    let parsed = parse_site_event(
        &site_event_with(&k, "例え.example", "サイト — 日記", ""),
        35980,
    )
    .unwrap();
    assert_eq!(parsed.title.as_deref(), Some("サイト — 日記"));
}

#[test]
fn drops_an_oversized_message_but_keeps_the_event() {
    let k = keys();
    let at_limit = "a".repeat(budget::MAX_CONTENT_BYTES);
    let parsed =
        parse_site_event(&site_event_with(&k, "example.com", "t", &at_limit), 35980).unwrap();
    assert_eq!(parsed.message.as_deref(), Some(at_limit.as_str()));

    let over = "a".repeat(budget::MAX_CONTENT_BYTES + 1);
    let parsed = parse_site_event(&site_event_with(&k, "example.com", "t", &over), 35980).unwrap();
    assert_eq!(parsed.message, None);
}

#[test]
fn select_latest_breaks_created_at_ties_by_the_lowest_id() {
    let k = keys();
    let a = parse_site_event(
        &make_site_event(&k, 35980, "example.com", CID_A, 500),
        35980,
    )
    .unwrap();
    let b = parse_site_event(
        &make_site_event(&k, 35980, "example.com", CID_B, 500),
        35980,
    )
    .unwrap();
    let expected = if a.id < b.id {
        a.cid.clone()
    } else {
        b.cid.clone()
    };
    for events in [vec![a.clone(), b.clone()], vec![b, a]] {
        let latest = select_latest(&events, 1000);
        assert_eq!(
            latest[&(k.public_key().to_hex(), "example.com".to_string())].cid,
            expected
        );
    }
}

#[test]
fn a_site_note_joins_title_url_and_message_with_spaces() {
    assert_eq!(
        site_note_content(Some("My site"), "https://example.com/", Some("Add posts")),
        "My site https://example.com/ Add posts #swingpublish"
    );
    assert_eq!(
        site_note_content(None, "https://example.com/", Some("")),
        "https://example.com/ #swingpublish"
    );
}

#[test]
fn a_site_note_is_a_text_note_pointing_at_the_site() {
    let k = keys();
    let ev = build_site_note_builder(35980, &k.public_key(), "example.com", "hello")
        .finalize(&k)
        .unwrap();
    assert_eq!(ev.kind, Kind::TextNote);
    assert_eq!(ev.content, "hello");
    let a = format!("35980:{}:example.com", k.public_key().to_hex());
    assert_eq!(tag_value(&ev, "a"), Some(a.as_str()));
    assert_eq!(tag_value(&ev, "t"), Some("swingpublish"));
}
