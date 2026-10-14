use std::collections::HashSet;
use std::future::Future;

use anyhow::Result;
use nostr_sdk::prelude::*;

use super::super::{ANCHOR_KIND, FOLLOW_SET_KIND, is_follow_set_of, newest_by_address};
use super::{RelayClient, by_authors, capped_limit};

const ADS_PER_ADVERTISER: usize = 8;

pub trait AnchorRelay {
    fn fetch_anchor_ads(
        &self,
        advertisers: &[PublicKey],
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn fetch_own_anchor_ads(
        &self,
        peer_id: &str,
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn fetch_followers_among(
        &self,
        mirror_set: &str,
        accounts: &[PublicKey],
        followed: PublicKey,
    ) -> impl Future<Output = Result<HashSet<PublicKey>>> + Send;
    fn send_anchor_ad(&self, ad: EventBuilder) -> impl Future<Output = Result<bool>> + Send;
}

impl AnchorRelay for RelayClient {
    async fn fetch_anchor_ads(&self, advertisers: &[PublicKey]) -> Result<Vec<Event>> {
        let kind = Kind::Custom(ANCHOR_KIND);
        let events = self
            .fetch_per_author(advertisers, "fetching anchor ads", |advertiser| {
                Filter::new()
                    .kind(kind)
                    .author(advertiser)
                    .limit(ADS_PER_ADVERTISER)
            })
            .await?;
        Ok(events.into_iter().filter(|e| e.kind == kind).collect())
    }

    async fn fetch_own_anchor_ads(&self, peer_id: &str) -> Result<Vec<Event>> {
        let own = RelayClient::public_key(self);
        let filter = Filter::new()
            .kind(Kind::Custom(ANCHOR_KIND))
            .author(own)
            .identifier(peer_id)
            .limit(capped_limit(1, 2));
        let events = self.fetch_one(filter, "fetching our own anchor ad").await?;
        Ok(events.into_iter().filter(|e| e.pubkey == own).collect())
    }

    async fn fetch_followers_among(
        &self,
        mirror_set: &str,
        accounts: &[PublicKey],
        followed: PublicKey,
    ) -> Result<HashSet<PublicKey>> {
        let events = self
            .fetch_by_authors(accounts, "fetching follow sets that include us", |batch| {
                Filter::new()
                    .kind(Kind::Custom(FOLLOW_SET_KIND))
                    .authors(batch.iter().copied())
                    .identifier(mirror_set)
                    .pubkey(followed)
                    .limit(capped_limit(batch.len(), 2))
            })
            .await?;
        let sets = by_authors(events, accounts)
            .into_iter()
            .filter(|e| is_follow_set_of(e, &e.pubkey, mirror_set));
        Ok(newest_by_address(sets, Timestamp::now().as_secs())
            .into_iter()
            .filter(|e| e.tags.public_keys().any(|pk| pk == followed))
            .map(|e| e.pubkey)
            .collect())
    }

    async fn send_anchor_ad(&self, ad: EventBuilder) -> Result<bool> {
        self.sign_and_publish(ad, "signing anchor ad").await
    }
}

impl<T: AnchorRelay + Send + Sync> AnchorRelay for std::sync::Arc<T> {
    async fn fetch_anchor_ads(&self, advertisers: &[PublicKey]) -> Result<Vec<Event>> {
        T::fetch_anchor_ads(self, advertisers).await
    }

    async fn fetch_own_anchor_ads(&self, peer_id: &str) -> Result<Vec<Event>> {
        T::fetch_own_anchor_ads(self, peer_id).await
    }

    async fn fetch_followers_among(
        &self,
        mirror_set: &str,
        accounts: &[PublicKey],
        followed: PublicKey,
    ) -> Result<HashSet<PublicKey>> {
        T::fetch_followers_among(self, mirror_set, accounts, followed).await
    }

    async fn send_anchor_ad(&self, ad: EventBuilder) -> Result<bool> {
        T::send_anchor_ad(self, ad).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::nostr::{AdReachability, build_anchor_ad_builder};
    use crate::signer::Signer;
    use crate::test_support::{
        PEER_A, PEER_B, PEER_OWN, anchor_ad_event, follow_set, keys, relay_holding,
    };

    fn ad(k: &Keys, peer: &str, created_at: u64) -> Event {
        anchor_ad_event(
            k,
            peer,
            AdReachability::Public,
            created_at,
            created_at + 10_000,
        )
    }

    #[tokio::test]
    async fn anchor_queries_reach_a_local_relay() {
        let own = keys();
        let alice = keys();
        let bob = keys();
        let stranger = keys();
        let now = Timestamp::now().as_secs();
        let (_relay, url) = relay_holding(&[
            ad(&alice, PEER_A, now - 10),
            ad(&bob, PEER_B, now - 10),
            ad(&stranger, PEER_B, now - 10),
            ad(&own, PEER_OWN, now - 10),
            ad(&own, PEER_A, now - 10),
            follow_set(&alice, "swing", None, "", &[own.public_key()], now),
            follow_set(&bob, "swing", None, "", &[stranger.public_key()], now),
        ])
        .await;
        let client = RelayClient::connect(Signer::Local(own.clone()), &[url])
            .await
            .unwrap();

        let ads = client
            .fetch_anchor_ads(&[alice.public_key(), bob.public_key()])
            .await
            .unwrap();
        let authors: BTreeSet<PublicKey> = ads.iter().map(|e| e.pubkey).collect();
        assert_eq!(
            authors,
            BTreeSet::from([alice.public_key(), bob.public_key()])
        );

        let mine = client.fetch_own_anchor_ads(PEER_OWN).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].tags.identifier().as_deref(), Some(PEER_OWN));

        let mutual = client
            .fetch_followers_among(
                "swing",
                &[alice.public_key(), bob.public_key()],
                own.public_key(),
            )
            .await
            .unwrap();
        assert_eq!(mutual, HashSet::from([alice.public_key()]));

        let sent = client
            .send_anchor_ad(build_anchor_ad_builder(
                PEER_B,
                AdReachability::Private,
                Timestamp::from_secs(now + 100),
            ))
            .await
            .unwrap();
        assert!(sent);
        assert_eq!(client.fetch_own_anchor_ads(PEER_B).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn one_advertiser_with_many_ads_does_not_crowd_out_another() {
        let busy = keys();
        let quiet = keys();
        let now = Timestamp::now().as_secs();
        let mut events: Vec<Event> = (0..2 * ADS_PER_ADVERTISER)
            .map(|i| ad(&busy, &format!("peer{i}"), now - 10))
            .collect();
        events.push(ad(&quiet, PEER_A, now - 1000));
        let (_relay, url) = relay_holding(&events).await;
        let client = RelayClient::connect(Signer::Local(keys()), &[url])
            .await
            .unwrap();

        let ads = client
            .fetch_anchor_ads(&[busy.public_key(), quiet.public_key()])
            .await
            .unwrap();
        assert_eq!(
            ads.iter()
                .filter(|e| e.pubkey == quiet.public_key())
                .count(),
            1
        );
        assert!(ads.iter().filter(|e| e.pubkey == busy.public_key()).count() <= ADS_PER_ADVERTISER);
    }
}
