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
    use std::time::Duration;

    use super::*;
    use crate::test_support::KuboApi;

    const VERSION: &str = "/swing/publish/k/s/100";

    async fn wait_for_removal(kubo: &KuboApi) {
        for _ in 0..100 {
            if !kubo.s.lock().unwrap().removed.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn removed(kubo: &KuboApi) -> Vec<String> {
        kubo.s.lock().unwrap().removed.clone()
    }

    #[tokio::test]
    async fn withdraw_removes_the_version() {
        let kubo = KuboApi::serve("").await;
        let staged = StagedVersion::new(&kubo.client(), VERSION.to_string());
        staged.withdraw().await.unwrap();
        assert_eq!(removed(&kubo), [VERSION]);
    }

    #[tokio::test]
    async fn fail_removes_the_version_and_returns_the_error() {
        let kubo = KuboApi::serve("").await;
        let staged = StagedVersion::new(&kubo.client(), VERSION.to_string());
        let err = staged.fail(anyhow::anyhow!("no relay accepted")).await;
        assert_eq!(format!("{err:#}"), "no relay accepted");
        assert_eq!(removed(&kubo), [VERSION]);
    }

    #[tokio::test]
    async fn fail_reports_a_removal_failure_with_the_error() {
        let kubo = KuboApi::serve("").await;
        kubo.with(|s| {
            s.fail_remove.insert(VERSION.to_string());
        });
        let staged = StagedVersion::new(&kubo.client(), VERSION.to_string());
        let err = staged.fail(anyhow::anyhow!("signing failed")).await;
        let text = format!("{err:#}");
        assert!(text.starts_with("signing failed; could not remove /swing/publish/k/s/100"));
    }

    #[tokio::test]
    async fn keep_leaves_the_version_in_place() {
        let kubo = KuboApi::serve("").await;
        let staged = StagedVersion::new(&kubo.client(), VERSION.to_string());
        assert_eq!(staged.keep(), VERSION);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(kubo.calls().is_empty());
    }

    #[tokio::test]
    async fn dropping_an_unannounced_version_removes_it() {
        let kubo = KuboApi::serve("").await;
        drop(StagedVersion::new(&kubo.client(), VERSION.to_string()));
        wait_for_removal(&kubo).await;
        assert_eq!(removed(&kubo), [VERSION]);
    }
}
