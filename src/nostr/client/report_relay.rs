use std::future::Future;

use anyhow::Result;
use nostr_sdk::prelude::*;

use super::super::budget;
use super::{Paged, RelayClient, capped_limit};

const OWN_REPORTS_PAGE: usize = 500;

pub trait ReportRelay {
    fn public_key(&self) -> PublicKey;
    fn fetch_own_reports(&self, report_kind: u16) -> impl Future<Output = Result<Paged>> + Send;
    fn fetch_reports_about(
        &self,
        report_kind: u16,
        author: PublicKey,
        reporters: &[PublicKey],
        since: Option<u64>,
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn fetch_reports_by(
        &self,
        report_kind: u16,
        sites: &[Coordinate],
        reporters: &[PublicKey],
    ) -> impl Future<Output = Result<Vec<Event>>> + Send;
    fn send_report(&self, report: EventBuilder) -> impl Future<Output = Result<bool>> + Send;
}

impl ReportRelay for RelayClient {
    fn public_key(&self) -> PublicKey {
        RelayClient::public_key(self)
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Paged> {
        let filter = Filter::new()
            .kind(Kind::Custom(report_kind))
            .author(RelayClient::public_key(self));
        self.fetch_pages(filter, OWN_REPORTS_PAGE, "fetching our own replica reports")
            .await
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
                "fetching replica reports about our own sites",
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
        Ok(events
            .into_iter()
            .filter(|e| e.kind == kind && e.tags.public_keys().any(|pk| pk == author))
            .collect())
    }

    async fn fetch_reports_by(
        &self,
        report_kind: u16,
        sites: &[Coordinate],
        reporters: &[PublicKey],
    ) -> Result<Vec<Event>> {
        self.fetch_replica_reports_by(report_kind, sites, reporters)
            .await
    }

    async fn send_report(&self, report: EventBuilder) -> Result<bool> {
        self.sign_and_publish(report, "signing replica report")
            .await
    }
}

impl<T: ReportRelay + Send + Sync> ReportRelay for std::sync::Arc<T> {
    fn public_key(&self) -> PublicKey {
        T::public_key(self)
    }

    async fn fetch_own_reports(&self, report_kind: u16) -> Result<Paged> {
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

    async fn fetch_reports_by(
        &self,
        report_kind: u16,
        sites: &[Coordinate],
        reporters: &[PublicKey],
    ) -> Result<Vec<Event>> {
        T::fetch_reports_by(self, report_kind, sites, reporters).await
    }

    async fn send_report(&self, report: EventBuilder) -> Result<bool> {
        T::send_report(self, report).await
    }
}
