use std::collections::HashSet;
use std::future::Future;

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use super::super::budget;
use super::{RelayClient, capped_limit};

pub trait ReportRelay {
    fn public_key(&self) -> PublicKey;
    fn fetch_own_reports(
        &self,
        report_kind: u16,
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn send_report(&self, report: EventBuilder) -> impl Future<Output = Result<bool>> + Send;
}

impl ReportRelay for RelayClient {
    fn public_key(&self) -> PublicKey {
        RelayClient::public_key(self)
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Vec<Event>> {
        let filter = Filter::new()
            .kind(Kind::Custom(report_kind))
            .author(RelayClient::public_key(self))
            .limit(capped_limit(budget::MAX_SITES_PER_AUTHOR_LISTED, 2));
        self.fetch_one(filter, "fetching own replica reports").await
    }

    async fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> Result<Vec<Event>> {
        let kind = Kind::Custom(report_kind);
        let events = self
            .fetch_per_author(
                reporters,
                "fetching replica reports about own sites",
                |reporter| {
                    let filter = Filter::new()
                        .kind(kind)
                        .author(reporter)
                        .pubkey(author)
                        .limit(capped_limit(budget::MAX_SITES_PER_AUTHOR_LISTED, 2));
                    match since {
                        Some(since) => filter.since(Timestamp::from_secs(since)),
                        None => filter,
                    }
                },
            )
            .await?;
        let requested: HashSet<PublicKey> = reporters.iter().copied().collect();
        Ok(events
            .into_iter()
            .filter(|e| {
                e.kind == kind
                    && requested.contains(&e.pubkey)
                    && e.tags.public_keys().any(|pk| pk == author)
            })
            .collect())
    }

    async fn send_report(&self, report: EventBuilder) -> Result<bool> {
        let event = self.sign(report).await.context("signing replica report")?;
        let output = self.publish_to_relays(&event).await?;
        Ok(!output.success.is_empty())
    }
}

impl<T: ReportRelay + Send + Sync> ReportRelay for std::sync::Arc<T> {
    fn public_key(&self) -> PublicKey {
        T::public_key(self)
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Vec<Event>> {
        T::fetch_own_reports(self, report_kind).await
    }

    async fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> Result<Vec<Event>> {
        T::fetch_reports_about(self, report_kind, author, reporters, since).await
    }

    async fn send_report(&self, report: EventBuilder) -> Result<bool> {
        T::send_report(self, report).await
    }
}
