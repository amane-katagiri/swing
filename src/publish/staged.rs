use anyhow::{Context, Result};
use tracing::warn;

use crate::ipfs::IpfsClient;

pub struct StagedVersion {
    ipfs: IpfsClient,
    path: Option<String>,
}

impl StagedVersion {
    pub fn new(ipfs: &IpfsClient, path: String) -> Self {
        Self {
            ipfs: ipfs.clone(),
            path: Some(path),
        }
    }

    pub fn path(&self) -> &str {
        self.path.as_deref().unwrap_or_default()
    }

    pub fn keep(mut self) -> String {
        self.path.take().unwrap_or_default()
    }

    pub async fn withdraw(mut self) -> Result<()> {
        let Some(path) = self.path.take() else {
            return Ok(());
        };
        self.ipfs
            .mfs_remove(&path)
            .await
            .with_context(|| format!("could not remove {path}"))
    }

    pub async fn fail(self, err: anyhow::Error) -> anyhow::Error {
        match self.withdraw().await {
            Ok(()) => err,
            Err(e) => anyhow::anyhow!("{err:#}; {e:#}"),
        }
    }
}

impl Drop for StagedVersion {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let ipfs = self.ipfs.clone();
        // A cancelled publish (upload timeout, closed connection) is dropped without reaching an await.
        runtime.spawn(async move {
            if let Err(e) = ipfs.mfs_remove(&path).await {
                warn!(path, error = %format!("{e:#}"), "could not remove an unannounced version");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::extract::{RawQuery, State};
    use axum::routing::post;

    use super::*;

    type Removed = Arc<Mutex<Vec<String>>>;

    async fn fake_kubo(answer: &'static str) -> (IpfsClient, Removed) {
        let removed: Removed = Arc::default();
        let router = axum::Router::new()
            .route(
                "/api/v0/files/rm",
                post(
                    move |State(removed): State<Removed>, RawQuery(query): RawQuery| async move {
                        removed.lock().unwrap().push(query.unwrap_or_default());
                        answer
                    },
                ),
            )
            .with_state(removed.clone());
        let addr = crate::test_support::serve_router(router).await;
        (IpfsClient::new(format!("http://{addr}")), removed)
    }

    fn removed_paths(removed: &Removed) -> Vec<String> {
        removed
            .lock()
            .unwrap()
            .iter()
            .filter_map(|q| {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix("arg="))
                    .map(str::to_string)
            })
            .collect()
    }

    async fn wait_for_removal(removed: &Removed) {
        for _ in 0..100 {
            if !removed.lock().unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn withdraw_removes_the_version() {
        let (ipfs, removed) = fake_kubo("").await;
        let staged = StagedVersion::new(&ipfs, "/swing/publish/k/s/100".to_string());
        staged.withdraw().await.unwrap();
        assert_eq!(removed_paths(&removed), vec!["/swing/publish/k/s/100"]);
    }

    #[tokio::test]
    async fn fail_removes_the_version_and_returns_the_error() {
        let (ipfs, removed) = fake_kubo("").await;
        let staged = StagedVersion::new(&ipfs, "/swing/publish/k/s/100".to_string());
        let err = staged.fail(anyhow::anyhow!("no relay accepted")).await;
        assert_eq!(format!("{err:#}"), "no relay accepted");
        assert_eq!(removed_paths(&removed), vec!["/swing/publish/k/s/100"]);
    }

    #[tokio::test]
    async fn fail_reports_a_removal_failure_with_the_error() {
        let (ipfs, _removed) = fake_kubo("file does not exist").await;
        let staged = StagedVersion::new(&ipfs, "/swing/publish/k/s/100".to_string());
        let err = staged.fail(anyhow::anyhow!("signing failed")).await;
        let text = format!("{err:#}");
        assert!(text.starts_with("signing failed; could not remove /swing/publish/k/s/100"));
    }

    #[tokio::test]
    async fn keep_leaves_the_version_in_place() {
        let (ipfs, removed) = fake_kubo("").await;
        let staged = StagedVersion::new(&ipfs, "/swing/publish/k/s/100".to_string());
        assert_eq!(staged.keep(), "/swing/publish/k/s/100");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(removed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dropping_an_unannounced_version_removes_it() {
        let (ipfs, removed) = fake_kubo("").await;
        drop(StagedVersion::new(
            &ipfs,
            "/swing/publish/k/s/100".to_string(),
        ));
        wait_for_removal(&removed).await;
        assert_eq!(removed_paths(&removed), vec!["/swing/publish/k/s/100"]);
    }
}
