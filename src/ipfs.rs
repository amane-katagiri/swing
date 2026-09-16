use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchLimits {
    pub max_bytes: u64,
    pub total: Duration,
    pub idle: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched {
    Complete,
    TooLarge,
}

pub trait KuboPins {
    fn fetch_dag(
        &self,
        cid: &str,
        limits: FetchLimits,
    ) -> impl Future<Output = Result<Fetched>> + Send;
    fn pin_add_local(
        &self,
        cid: &str,
        timeout: Duration,
    ) -> impl Future<Output = Result<()>> + Send;
    fn dag_size_local(&self, cid: &str) -> impl Future<Output = Result<u64>> + Send;
    fn is_pinned(&self, cid: &str) -> impl Future<Output = Result<bool>> + Send;
    fn recursive_pins(&self) -> impl Future<Output = Result<HashSet<String>>> + Send;
    fn pin_rm(&self, cid: &str) -> impl Future<Output = Result<()>> + Send;
}

pub struct IpfsClient {
    http: reqwest::Client,
    api: String,
}

impl KuboPins for IpfsClient {
    async fn fetch_dag(&self, cid: &str, limits: FetchLimits) -> Result<Fetched> {
        IpfsClient::fetch_dag(self, cid, limits).await
    }

    async fn pin_add_local(&self, cid: &str, timeout: Duration) -> Result<()> {
        IpfsClient::pin_add_local(self, cid, timeout).await
    }

    async fn dag_size_local(&self, cid: &str) -> Result<u64> {
        IpfsClient::dag_size_local(self, cid).await
    }

    async fn is_pinned(&self, cid: &str) -> Result<bool> {
        IpfsClient::is_pinned(self, cid).await
    }

    async fn recursive_pins(&self) -> Result<HashSet<String>> {
        IpfsClient::pin_ls(self).await
    }

    async fn pin_rm(&self, cid: &str) -> Result<()> {
        IpfsClient::pin_rm(self, cid).await
    }
}

fn percent_encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_encode_relative_path(rel: &str) -> String {
    rel.split('/')
        .map(percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug)]
enum Entry {
    Dir(String),
    File(String, PathBuf),
}

fn walk(
    current: &Path,
    rel_prefix: &str,
    out: &mut Vec<Entry>,
    dir_ancestors: &mut HashSet<PathBuf>,
) -> Result<()> {
    let mut children: Vec<std::fs::DirEntry> = std::fs::read_dir(current)
        .with_context(|| format!("reading dir {}", current.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading dir {}", current.display()))?;
    children.sort_by_key(|e| e.file_name());

    for entry in children {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        // metadata() follows symlinks (unlike DirEntry::file_type()), so linked
        // files/dirs are included instead of silently skipped.
        let metadata = std::fs::metadata(&path)
            .with_context(|| format!("reading metadata for {}", path.display()))?;
        if metadata.is_dir() {
            let canon = std::fs::canonicalize(&path)
                .with_context(|| format!("resolving path {}", path.display()))?;
            if !dir_ancestors.insert(canon.clone()) {
                bail!("symlink loop detected at {}", path.display());
            }
            out.push(Entry::Dir(rel.clone()));
            walk(&path, &rel, out, dir_ancestors)?;
            dir_ancestors.remove(&canon);
        } else if metadata.is_file() {
            out.push(Entry::File(rel, path));
        }
    }
    Ok(())
}

fn walk_root(dir: &Path, root_name: &str) -> Result<Vec<Entry>> {
    let canon_root =
        std::fs::canonicalize(dir).with_context(|| format!("resolving path {}", dir.display()))?;
    let mut dir_ancestors = HashSet::new();
    dir_ancestors.insert(canon_root);
    let mut entries = Vec::new();
    walk(dir, root_name, &mut entries, &mut dir_ancestors)?;
    Ok(entries)
}

#[derive(Debug, Deserialize)]
struct AddResponseLine {
    #[serde(rename = "Hash")]
    hash: String,
}

#[derive(Debug, Deserialize)]
struct FilesStatResponse {
    #[serde(rename = "CumulativeSize")]
    cumulative_size: u64,
}

#[derive(Debug, Deserialize)]
struct DagStatResponse {
    #[serde(rename = "TotalSize")]
    total_size: u64,
}

#[derive(Debug, Deserialize)]
struct PinLsResponse {
    #[serde(rename = "Keys")]
    keys: std::collections::HashMap<String, serde_json::Value>,
}

impl IpfsClient {
    pub fn new(api: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api: api.into().trim_end_matches('/').to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api, path)
    }

    pub async fn add_dir(&self, dir: &Path) -> Result<String> {
        if !dir.is_dir() {
            bail!("not a directory: {}", dir.display());
        }
        // Kubo's multipart add infers the wrapping directory node from a shared
        // filename prefix; without the root dir's own name as that prefix it
        // adds each file as an unlinked blob instead of one directory CID.
        let root_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "root".to_string());
        let dir_owned = dir.to_path_buf();
        let entries = tokio::task::spawn_blocking(move || walk_root(&dir_owned, &root_name))
            .await
            .context("walk task panicked")??;
        if entries.is_empty() {
            bail!("directory is empty: {}", dir.display());
        }

        let mut form = Form::new();
        for entry in entries {
            let part = match entry {
                Entry::Dir(rel) => Part::bytes(Vec::new())
                    .file_name(percent_encode_relative_path(&rel))
                    .mime_str("application/x-directory")?,
                Entry::File(rel, path) => {
                    let file = tokio::fs::File::open(&path)
                        .await
                        .with_context(|| format!("opening {}", path.display()))?;
                    Part::stream(file)
                        .file_name(percent_encode_relative_path(&rel))
                        .mime_str("application/octet-stream")?
                }
            };
            form = form.part("file", part);
        }

        let url = self.url(
            "/api/v0/add?recursive=true&cid-version=1&pin=true&quieter=true&wrap-with-directory=false",
        );
        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .context("POST /api/v0/add")?;
        let status = resp.status();
        let text = resp.text().await.context("reading add response body")?;
        if !status.is_success() {
            bail!("ipfs add failed: {status}: {text}");
        }

        let last_line = text
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .context("empty response from ipfs add")?;
        let parsed: AddResponseLine =
            serde_json::from_str(last_line).context("parsing ipfs add response")?;
        Ok(parsed.hash)
    }

    pub async fn files_stat(&self, cid: &str) -> Result<u64> {
        let url = self.url(&format!(
            "/api/v0/files/stat?arg=/ipfs/{}",
            urlencoding_cid(cid)
        ));
        let resp = self
            .http
            .post(&url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("POST /api/v0/files/stat")?;
        let status = resp.status();
        let text = resp.text().await.context("reading files/stat response")?;
        if !status.is_success() {
            bail!("files/stat failed: {status}: {text}");
        }
        let parsed: FilesStatResponse =
            serde_json::from_str(&text).context("parsing files/stat response")?;
        Ok(parsed.cumulative_size)
    }

    pub async fn fetch_dag(&self, cid: &str, limits: FetchLimits) -> Result<Fetched> {
        let url = self.url(&format!(
            "/api/v0/dag/export?arg={}&progress=false",
            urlencoding_cid(cid)
        ));
        let fetch = async {
            let resp = tokio::time::timeout(limits.idle, self.http.post(&url).send())
                .await
                .context("dag/export stalled before the first block")?
                .context("POST /api/v0/dag/export")?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                bail!("dag/export failed: {status}: {text}");
            }
            let mut stream = resp.bytes_stream();
            let mut received: u64 = 0;
            while let Some(chunk) = tokio::time::timeout(limits.idle, stream.next())
                .await
                .context("dag/export stalled")?
            {
                received += chunk.context("reading dag/export body")?.len() as u64;
                if received > limits.max_bytes {
                    return Ok(Fetched::TooLarge);
                }
            }
            Ok(Fetched::Complete)
        };
        tokio::time::timeout(limits.total, fetch)
            .await
            .context("dag/export timed out")?
    }

    pub async fn dag_size_local(&self, cid: &str) -> Result<u64> {
        let url = self.url(&format!(
            "/api/v0/dag/stat?arg={}&progress=false&offline=true",
            urlencoding_cid(cid)
        ));
        let resp = self
            .http
            .post(&url)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .context("POST /api/v0/dag/stat")?;
        let status = resp.status();
        let text = resp.text().await.context("reading dag/stat response")?;
        if !status.is_success() {
            bail!("dag/stat failed: {status}: {text}");
        }
        let parsed: DagStatResponse =
            serde_json::from_str(&text).context("parsing dag/stat response")?;
        Ok(parsed.total_size)
    }

    pub async fn pin_add_local(&self, cid: &str, timeout: Duration) -> Result<()> {
        // offline=true keeps Kubo from fetching blocks that fetch_dag did not
        // count, so the size cap cannot be bypassed by a truncated export.
        let url = self.url(&format!(
            "/api/v0/pin/add?arg={}&recursive=true&offline=true",
            urlencoding_cid(cid)
        ));
        let resp = self
            .http
            .post(&url)
            .timeout(timeout)
            .send()
            .await
            .context("POST /api/v0/pin/add")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("pin/add failed: {status}: {text}");
        }
        Ok(())
    }

    pub async fn pin_rm(&self, cid: &str) -> Result<()> {
        let url = self.url(&format!(
            "/api/v0/pin/rm?arg={}&recursive=true",
            urlencoding_cid(cid)
        ));
        let resp = self
            .http
            .post(&url)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .context("POST /api/v0/pin/rm")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            if text.contains("not pinned or pinned indirectly") {
                return Ok(());
            }
            bail!("pin/rm failed: {status}: {text}");
        }
        Ok(())
    }

    // Recursive and direct are checked separately because type=all also
    // searches indirect pins, which walks every pinned DAG for unpinned CIDs.
    pub async fn is_pinned(&self, cid: &str) -> Result<bool> {
        for pin_type in ["recursive", "direct"] {
            let url = self.url(&format!(
                "/api/v0/pin/ls?arg={}&type={pin_type}",
                urlencoding_cid(cid)
            ));
            let resp = self
                .http
                .post(&url)
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .context("POST /api/v0/pin/ls")?;
            let status = resp.status();
            let text = resp.text().await.context("reading pin/ls response")?;
            if status.is_success() {
                return Ok(true);
            }
            if !text.contains("is not pinned") {
                bail!("pin/ls failed: {status}: {text}");
            }
        }
        Ok(false)
    }

    pub async fn pin_ls(&self) -> Result<HashSet<String>> {
        let url = self.url("/api/v0/pin/ls?type=recursive");
        let resp = self
            .http
            .post(&url)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .context("POST /api/v0/pin/ls")?;
        let status = resp.status();
        let text = resp.text().await.context("reading pin/ls response")?;
        if !status.is_success() {
            bail!("pin/ls failed: {status}: {text}");
        }
        let parsed: PinLsResponse =
            serde_json::from_str(&text).context("parsing pin/ls response")?;
        Ok(parsed.keys.into_keys().collect())
    }
}

fn urlencoding_cid(cid: &str) -> String {
    percent_encode_segment(cid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_leaves_safe_chars_and_slash_boundary() {
        assert_eq!(percent_encode_segment("index.html"), "index.html");
        assert_eq!(percent_encode_segment("a b"), "a%20b");
        assert_eq!(percent_encode_relative_path("css/a b.css"), "css/a%20b.css");
    }

    fn entry_names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Dir(r) => format!("dir:{r}"),
                Entry::File(r, _) => format!("file:{r}"),
            })
            .collect()
    }

    #[test]
    fn walk_collects_files_and_dirs_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
        std::fs::create_dir(dir.path().join("css")).unwrap();
        std::fs::write(dir.path().join("css/style.css"), b"body{}").unwrap();
        std::fs::create_dir(dir.path().join("empty")).unwrap();

        let entries = walk_root(dir.path(), "").unwrap();
        let names = entry_names(&entries);
        assert!(names.contains(&"file:index.html".to_string()));
        assert!(names.contains(&"dir:css".to_string()));
        assert!(names.contains(&"file:css/style.css".to_string()));
        assert!(names.contains(&"dir:empty".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn walk_follows_symlinked_files_and_dirs() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        std::fs::write(target_dir.path().join("real.txt"), b"real").unwrap();
        std::fs::write(dir.path().join("real_file.txt"), b"hi").unwrap();

        symlink(
            dir.path().join("real_file.txt"),
            dir.path().join("link_file.txt"),
        )
        .unwrap();
        symlink(target_dir.path(), dir.path().join("link_dir")).unwrap();

        let entries = walk_root(dir.path(), "").unwrap();
        let names = entry_names(&entries);
        assert!(names.contains(&"file:link_file.txt".to_string()));
        assert!(names.contains(&"dir:link_dir".to_string()));
        assert!(names.contains(&"file:link_dir/real.txt".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn walk_rejects_symlink_loop() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        symlink(dir.path(), dir.path().join("self_loop")).unwrap();

        let err = walk_root(dir.path(), "").unwrap_err();
        assert!(err.to_string().contains("symlink loop"));
    }
}
