use anyhow::{Context, Result, bail};
use cid::multibase::Base;
use cid::multihash::Multihash;
use nostr_sdk::prelude::*;

use super::{budget, newest_by_address, parse_decimal, plausible_at, tag_value};

pub const ANCHOR_KIND: u16 = 35982;

const IDENTITY_CODE: u64 = 0x00;
const SHA2_256_CODE: u64 = 0x12;
const MAX_INLINE_KEY_BYTES: usize = 42;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdReachability {
    Public,
    Private,
}

impl AdReachability {
    pub fn token(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Self::Public),
            "private" => Some(Self::Private),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorAd {
    pub advertiser: PublicKey,
    pub peer_id: String,
    pub reachability: AdReachability,
    pub created_at: u64,
    pub expiration: u64,
}

impl AnchorAd {
    pub fn usable_at(&self, now: u64) -> bool {
        self.reachability == AdReachability::Public
            && plausible_at(self.created_at, now)
            && self.expiration > now
    }
}

pub fn validate_peer_id(s: &str) -> Result<()> {
    let bytes = Base::Base58Btc
        .decode(s)
        .map_err(|e| anyhow::anyhow!("peer id is not base58btc: {e}"))?;
    let hash = Multihash::<64>::from_bytes(&bytes).context("peer id is not a multihash")?;
    let ok = match hash.code() {
        IDENTITY_CODE => hash.size() as usize <= MAX_INLINE_KEY_BYTES,
        SHA2_256_CODE => hash.size() == 32,
        _ => false,
    };
    if !ok {
        bail!("peer id is not an identity or sha2-256 multihash");
    }
    if Base::Base58Btc.encode(&bytes) != s {
        bail!("peer id is not in its canonical spelling");
    }
    Ok(())
}

pub fn parse_anchor_ad(event: &Event) -> Result<AnchorAd> {
    if event.kind != Kind::Custom(ANCHOR_KIND) {
        bail!("unexpected kind {}", event.kind);
    }
    if event.content.len() > budget::MAX_CONTENT_BYTES {
        bail!("content too long: {} bytes", event.content.len());
    }
    let peer_id = event.tags.identifier().context("missing d tag")?;
    validate_peer_id(&peer_id)?;
    let reachability = tag_value(event, "reachability")
        .context("missing reachability tag")
        .and_then(|s| AdReachability::parse(s).context("invalid reachability tag"))?;
    let expiration = tag_value(event, "expiration")
        .context("missing expiration tag")
        .and_then(|s| parse_decimal(s).context("invalid expiration tag"))?;
    Ok(AnchorAd {
        advertiser: event.pubkey,
        peer_id,
        reachability,
        created_at: event.created_at.as_secs(),
        expiration,
    })
}

// The newest event per address is chosen before parsing, so a broken newer ad hides an older valid one as the protocol asks.
pub fn latest_anchor_ads(events: impl IntoIterator<Item = Event>, now: u64) -> Vec<AnchorAd> {
    let anchors = events
        .into_iter()
        .filter(|e| e.kind == Kind::Custom(ANCHOR_KIND));
    let mut ads: Vec<AnchorAd> = newest_by_address(anchors, now)
        .iter()
        .filter_map(|e| match parse_anchor_ad(e) {
            Ok(ad) => Some(ad),
            Err(err) => {
                tracing::debug!(event_id = %e.id, error = %err, "ignoring anchor ad");
                None
            }
        })
        .collect();
    ads.sort_by(|a, b| (a.advertiser, &a.peer_id).cmp(&(b.advertiser, &b.peer_id)));
    ads
}

pub fn build_anchor_ad_builder(
    peer_id: &str,
    reachability: AdReachability,
    expiration: Timestamp,
) -> EventBuilder {
    EventBuilder::new(Kind::Custom(ANCHOR_KIND), "")
        .tag(Tag::identifier(peer_id))
        .tag(Tag::custom("reachability", [reachability.token()]))
        .tag(Tag::expiration(expiration))
        .tag(Tag::custom("alt", [format!("SWING anchor: {peer_id}")]))
}

#[cfg(test)]
mod tests {
    use super::super::MAX_FUTURE_SKEW;
    use super::*;
    use crate::test_support::keys;

    const PEER_A: &str = "12D3KooWQqg6fSEnehBFPAzVpSwWzFheYrfktHPXLoNDJUisx9LJ";
    const PEER_QM: &str = "QmYwAPJzv5CZsnA9LqYKXfRSZryVXxNn7ZP1FyEBgvJvHR";

    fn ad(k: &Keys, peer: &str, reach: AdReachability, created_at: u64, exp: u64) -> Event {
        build_anchor_ad_builder(peer, reach, Timestamp::from_secs(exp))
            .custom_created_at(Timestamp::from_secs(created_at))
            .finalize(k)
            .unwrap()
    }

    fn raw(k: &Keys, tags: &[&[&str]]) -> Event {
        let mut b = EventBuilder::new(Kind::Custom(ANCHOR_KIND), "")
            .custom_created_at(Timestamp::from_secs(1000));
        for t in tags {
            b = b.tag(Tag::parse(t.iter().copied()).unwrap());
        }
        b.finalize(k).unwrap()
    }

    #[test]
    fn anchor_ad_round_trips() {
        let k = keys();
        let ev = ad(&k, PEER_A, AdReachability::Public, 1000, 2000);
        assert_eq!(ev.content, "");
        assert_eq!(
            tag_value(&ev, "alt"),
            Some(format!("SWING anchor: {PEER_A}").as_str())
        );
        let parsed = parse_anchor_ad(&ev).unwrap();
        assert_eq!(
            parsed,
            AnchorAd {
                advertiser: k.public_key(),
                peer_id: PEER_A.to_string(),
                reachability: AdReachability::Public,
                created_at: 1000,
                expiration: 2000,
            }
        );
        assert!(parsed.usable_at(1999));
        assert!(!parsed.usable_at(2000));
    }

    #[test]
    fn peer_ids_in_both_default_spellings_are_accepted() {
        validate_peer_id(PEER_A).unwrap();
        validate_peer_id(PEER_QM).unwrap();
        for bad in [
            "",
            "not-a-peer",
            "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
            "k51qzi5uqu5dlvj2baxnqndepeb86cbk3ng7n3i46uzyxzyqj2xjonzllnv0v8",
            "12D3KooW",
            "/p2p/12D3KooWQqg6fSEnehBFPAzVpSwWzFheYrfktHPXLoNDJUisx9LJ",
            "3yZe7d",
        ] {
            assert!(validate_peer_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn invalid_ads_are_rejected() {
        let k = keys();
        let cases: Vec<(&str, Event)> = vec![
            (
                "no d",
                raw(&k, &[&["reachability", "public"], &["expiration", "2000"]]),
            ),
            (
                "bad d",
                raw(
                    &k,
                    &[
                        &["d", "nope"],
                        &["reachability", "public"],
                        &["expiration", "2000"],
                    ],
                ),
            ),
            (
                "no reachability",
                raw(&k, &[&["d", PEER_A], &["expiration", "2000"]]),
            ),
            (
                "bad reachability",
                raw(
                    &k,
                    &[
                        &["d", PEER_A],
                        &["reachability", "Public"],
                        &["expiration", "2000"],
                    ],
                ),
            ),
            (
                "no expiration",
                raw(&k, &[&["d", PEER_A], &["reachability", "public"]]),
            ),
            (
                "bad expiration",
                raw(
                    &k,
                    &[
                        &["d", PEER_A],
                        &["reachability", "public"],
                        &["expiration", "+2000"],
                    ],
                ),
            ),
        ];
        for (name, ev) in cases {
            assert!(parse_anchor_ad(&ev).is_err(), "{name}");
        }
        let site = EventBuilder::new(Kind::Custom(35980), "")
            .tag(Tag::identifier(PEER_A))
            .finalize(&k)
            .unwrap();
        assert!(parse_anchor_ad(&site).is_err());
    }

    #[test]
    fn private_expired_and_future_ads_are_not_usable() {
        let k = keys();
        let private =
            parse_anchor_ad(&ad(&k, PEER_A, AdReachability::Private, 1000, 5000)).unwrap();
        assert!(!private.usable_at(1000));
        let future = parse_anchor_ad(&ad(
            &k,
            PEER_A,
            AdReachability::Public,
            1000 + MAX_FUTURE_SKEW + 1,
            9000,
        ))
        .unwrap();
        assert!(!future.usable_at(1000));
    }

    #[test]
    fn only_the_latest_ad_per_peer_counts_even_when_it_is_broken() {
        let k = keys();
        let other = keys();
        let old_public = ad(&k, PEER_A, AdReachability::Public, 1000, 9000);
        let new_private = ad(&k, PEER_A, AdReachability::Private, 1100, 9000);
        let qm = ad(&k, PEER_QM, AdReachability::Public, 900, 9000);
        let others = ad(&other, PEER_A, AdReachability::Public, 900, 9000);
        let ads = latest_anchor_ads(vec![old_public.clone(), new_private, qm, others], 2000);
        assert_eq!(ads.len(), 3);
        let mine: Vec<_> = ads
            .iter()
            .filter(|a| a.advertiser == k.public_key())
            .collect();
        assert!(
            mine.iter()
                .any(|a| a.peer_id == PEER_A && a.reachability == AdReachability::Private)
        );

        let broken = raw(
            &k,
            &[
                &["d", PEER_A],
                &["reachability", "maybe"],
                &["expiration", "9000"],
            ],
        );
        let broken = EventBuilder::new(Kind::Custom(ANCHOR_KIND), "")
            .tags(broken.tags.iter().cloned())
            .custom_created_at(Timestamp::from_secs(1200))
            .finalize(&k)
            .unwrap();
        assert!(latest_anchor_ads(vec![old_public, broken], 2000).is_empty());
    }
}
