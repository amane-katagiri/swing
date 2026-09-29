use std::collections::BTreeSet;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use super::{budget, canonical_cid, plausible_at, tag_value, validate_d_tag};

// `expiration` is self-declared, so only re-signing (not just the field) proves the reporter is still alive.
pub const MAX_REPORT_AGE: u64 = 7 * 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaReport {
    pub reporter: PublicKey,
    pub author: PublicKey,
    pub d: String,
    pub cids: BTreeSet<String>,
    pub created_at: u64,
    pub expiration: Option<u64>,
}

impl ReplicaReport {
    pub fn counts_at(&self, now: u64) -> bool {
        plausible_at(self.created_at, now)
            && now.saturating_sub(self.created_at) <= MAX_REPORT_AGE
            && self.expiration.is_none_or(|exp| exp > now)
    }
}

pub fn site_coordinate(site_event_kind: u16, author: &PublicKey, d: &str) -> Coordinate {
    Coordinate::new(Kind::Custom(site_event_kind), *author).identifier(d)
}

pub fn parse_replica_report(
    event: &Event,
    report_kind: u16,
    site_event_kind: u16,
) -> Result<ReplicaReport> {
    if event.kind != Kind::Custom(report_kind) {
        anyhow::bail!("unexpected kind {}", event.kind);
    }
    if event.content.len() > budget::MAX_CONTENT_BYTES {
        anyhow::bail!("content too long: {} bytes", event.content.len());
    }
    let identifier = event.tags.identifier().context("missing d tag")?;
    let (author_hex, d) = identifier
        .split_once(':')
        .context("d tag is not <pubkey>:<site>")?;
    let author = PublicKey::from_hex(author_hex).context("invalid author in d tag")?;
    if author.to_hex() != author_hex {
        anyhow::bail!("author in d tag is not lowercase hex");
    }
    validate_d_tag(d)?;
    let expected = site_coordinate(site_event_kind, &author, d).to_string();
    if tag_value(event, "a") != Some(expected.as_str()) {
        anyhow::bail!("a tag does not match d tag");
    }
    let cids = event
        .tags
        .iter()
        .filter(|t| t.kind() == "cid")
        .map(|t| canonical_cid(t.content().context("empty cid tag")?))
        .collect::<Result<BTreeSet<String>>>()?;
    let expiration = tag_value(event, "expiration")
        .map(|s| s.parse::<u64>().context("invalid expiration tag"))
        .transpose()?;
    Ok(ReplicaReport {
        reporter: event.pubkey,
        author,
        d: d.to_string(),
        cids,
        created_at: event.created_at.as_secs(),
        expiration,
    })
}

pub fn build_replica_report_builder(
    report_kind: u16,
    site_event_kind: u16,
    author: &PublicKey,
    d: &str,
    cids: &BTreeSet<String>,
    expiration: Timestamp,
) -> EventBuilder {
    let author_hex = author.to_hex();
    EventBuilder::new(Kind::Custom(report_kind), "")
        .tag(Tag::identifier(format!("{author_hex}:{d}")))
        .tag(Tag::custom(
            "a",
            [site_coordinate(site_event_kind, author, d).to_string()],
        ))
        .tag(Tag::public_key(*author))
        .tags(cids.iter().map(|cid| Tag::custom("cid", [cid.clone()])))
        .tag(Tag::expiration(expiration))
        .tag(Tag::custom("alt", [format!("SWING replica report: {d}")]))
}

#[cfg(test)]
mod tests {
    use super::super::MAX_FUTURE_SKEW;
    use super::super::fixtures::report;
    use super::*;
    use crate::test_support::{CID_A, CID_B, keys};

    fn report_with_tags(reporter: &Keys, d: &str, a: &str, cid: &str) -> Event {
        EventBuilder::new(Kind::Custom(35981), "")
            .tag(Tag::identifier(d))
            .tag(Tag::custom("a", [a.to_string()]))
            .tag(Tag::custom("cid", [cid.to_string()]))
            .finalize(reporter)
            .unwrap()
    }

    #[test]
    fn replica_report_round_trips() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(&reporter, &author, "a:b.example", &[CID_B, CID_A], 1000);

        assert_eq!(
            ev.tags.identifier().unwrap(),
            format!("{}:a:b.example", author.to_hex())
        );
        assert_eq!(
            tag_value(&ev, "a").unwrap(),
            format!("35980:{}:a:b.example", author.to_hex())
        );
        assert_eq!(ev.tags.public_keys().collect::<Vec<_>>(), vec![author]);
        assert_eq!(
            tag_value(&ev, "alt"),
            Some("SWING replica report: a:b.example")
        );

        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert_eq!(parsed.reporter, reporter.public_key());
        assert_eq!(parsed.author, author);
        assert_eq!(parsed.d, "a:b.example");
        assert_eq!(
            parsed.cids,
            BTreeSet::from([CID_A.to_string(), canonical_cid(CID_B).unwrap()])
        );
        assert_eq!(parsed.created_at, 1000);
        assert_eq!(parsed.expiration, Some(1100));
        assert!(parsed.counts_at(1099));
        assert!(!parsed.counts_at(1100));
    }

    #[test]
    fn replica_report_without_cids_is_a_withdrawal() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(&reporter, &author, "example.com", &[], 1000);
        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert!(parsed.cids.is_empty());
    }

    #[test]
    fn replica_report_collapses_the_same_cid_written_in_two_spellings() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(
            &reporter,
            &author,
            "example.com",
            &[CID_A, "zdj7Wic6KcJAfWz1c9o4M6kq9Lwd5BfbxkVafnrojaaGiSFxM"],
            1000,
        );
        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert_eq!(parsed.cids, BTreeSet::from([CID_A.to_string()]));
    }

    #[test]
    fn replica_report_rejects_a_cid_tag_whose_codec_is_not_dag_pb() {
        let reporter = keys();
        let author = keys().public_key();
        let ev = report(
            &reporter,
            &author,
            "example.com",
            &["bafyreigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"],
            1000,
        );
        assert!(parse_replica_report(&ev, 35981, 35980).is_err());
    }

    #[test]
    fn replica_report_rejects_inconsistent_or_invalid_tags() {
        let reporter = keys();
        let author = keys().public_key();
        let hex = author.to_hex();
        let d = format!("{hex}:example.com");
        let a = format!("35980:{hex}:example.com");

        assert!(
            parse_replica_report(&report_with_tags(&reporter, &d, &a, CID_A), 35981, 35980).is_ok()
        );
        assert!(
            parse_replica_report(&report_with_tags(&reporter, &d, &a, CID_A), 35982, 35980)
                .is_err()
        );

        let other_site = format!("35980:{hex}:other.example");
        let other_kind = format!("30023:{hex}:example.com");
        let other_author = format!("35980:{}:example.com", keys().public_key().to_hex());
        for bad_a in [other_site, other_kind, other_author] {
            let ev = report_with_tags(&reporter, &d, &bad_a, CID_A);
            assert!(parse_replica_report(&ev, 35981, 35980).is_err(), "{bad_a}");
        }

        let upper = format!("{}:example.com", hex.to_uppercase());
        for bad_d in [
            "example.com".to_string(),
            "abc:example.com".to_string(),
            upper,
            format!("{hex}:"),
        ] {
            let ev = report_with_tags(&reporter, &bad_d, &a, CID_A);
            assert!(parse_replica_report(&ev, 35981, 35980).is_err(), "{bad_d}");
        }

        let ev = report_with_tags(&reporter, &d, &a, "not-a-cid");
        assert!(parse_replica_report(&ev, 35981, 35980).is_err());
    }

    fn report_with_expiration(reporter: &Keys, d: &str, a: &str, expiration: &str) -> Event {
        EventBuilder::new(Kind::Custom(35981), "")
            .tag(Tag::identifier(d))
            .tag(Tag::custom("a", [a.to_string()]))
            .tag(Tag::custom("expiration", [expiration.to_string()]))
            .finalize(reporter)
            .unwrap()
    }

    #[test]
    fn replica_report_rejects_a_malformed_expiration_tag() {
        let reporter = keys();
        let author = keys().public_key();
        let hex = author.to_hex();
        let d = format!("{hex}:example.com");
        let a = format!("35980:{hex}:example.com");

        for bad in ["not-a-number", "", "12.5", "-1"] {
            let ev = report_with_expiration(&reporter, &d, &a, bad);
            assert!(parse_replica_report(&ev, 35981, 35980).is_err(), "{bad}");
        }
    }

    #[test]
    fn replica_report_without_an_expiration_tag_parses_as_none() {
        let reporter = keys();
        let author = keys().public_key();
        let hex = author.to_hex();
        let d = format!("{hex}:example.com");
        let a = format!("35980:{hex}:example.com");

        let ev = report_with_tags(&reporter, &d, &a, CID_A);
        let parsed = parse_replica_report(&ev, 35981, 35980).unwrap();
        assert_eq!(parsed.expiration, None);
    }

    fn report_at(created_at: u64, expiration: Option<u64>) -> ReplicaReport {
        ReplicaReport {
            reporter: keys().public_key(),
            author: keys().public_key(),
            d: "example.com".to_string(),
            cids: BTreeSet::from([CID_A.to_string()]),
            created_at,
            expiration,
        }
    }

    #[test]
    fn counts_at_accepts_a_fresh_report_with_no_expiration() {
        assert!(report_at(1000, None).counts_at(1000));
    }

    #[test]
    fn counts_at_rejects_a_report_older_than_max_report_age() {
        let report = report_at(1000, None);
        assert!(report.counts_at(1000 + MAX_REPORT_AGE));
        assert!(!report.counts_at(1000 + MAX_REPORT_AGE + 1));

        let far_future_expiration = report_at(1000, Some(u64::MAX));
        assert!(!far_future_expiration.counts_at(1000 + MAX_REPORT_AGE + 1));
    }

    #[test]
    fn counts_at_rejects_created_at_beyond_the_future_skew() {
        let report = report_at(1000 + MAX_FUTURE_SKEW, None);
        assert!(report.counts_at(1000));

        let report = report_at(1000 + MAX_FUTURE_SKEW + 1, None);
        assert!(!report.counts_at(1000));
    }

    #[test]
    fn counts_at_rejects_expiration_exactly_at_now() {
        let report = report_at(1000, Some(2000));
        assert!(report.counts_at(1999));
        assert!(!report.counts_at(2000));
    }

    #[test]
    fn replica_report_with_oversized_content_is_rejected() {
        let author = keys().public_key();
        let reporter = keys();
        let a = site_coordinate(35980, &author, "example.com").to_string();
        let ev = EventBuilder::new(
            Kind::Custom(35981),
            "a".repeat(budget::MAX_CONTENT_BYTES + 1),
        )
        .tag(Tag::identifier(format!("{}:example.com", author.to_hex())))
        .tag(Tag::custom("a", [a]))
        .tag(Tag::custom("cid", [CID_A.to_string()]))
        .finalize(&reporter)
        .unwrap();
        assert!(parse_replica_report(&ev, 35981, 35980).is_err());
    }
}
