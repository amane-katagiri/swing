use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use crate::mfs;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfsEntry {
    pub name: String,
    pub is_dir: bool,
    pub cid: String,
}

pub trait KuboStore {
    fn fetch_dag(
        &self,
        cid: &str,
        limits: FetchLimits,
    ) -> impl Future<Output = Result<Fetched>> + Send;
    fn dag_size_local(&self, cids: &[&str]) -> impl Future<Output = Result<u64>> + Send;
    fn mfs_put(&self, cid: &str, path: &str) -> impl Future<Output = Result<()>> + Send;
    fn mfs_remove(&self, path: &str) -> impl Future<Output = Result<()>> + Send;
    fn mfs_list(&self, path: &str) -> impl Future<Output = Result<Vec<MfsEntry>>> + Send;
    fn mfs_stat_cid(&self, path: &str) -> impl Future<Output = Result<Option<String>>> + Send;
    fn is_directory(&self, cid: &str) -> impl Future<Output = Result<bool>> + Send;
}

#[derive(Clone)]
pub struct IpfsClient {
    http: reqwest::Client,
    api: String,
}

impl KuboStore for IpfsClient {
    async fn fetch_dag(&self, cid: &str, limits: FetchLimits) -> Result<Fetched> {
        IpfsClient::fetch_dag(self, cid, limits).await
    }

    async fn dag_size_local(&self, cids: &[&str]) -> Result<u64> {
        IpfsClient::dag_size_local(self, cids).await
    }

    async fn mfs_put(&self, cid: &str, path: &str) -> Result<()> {
        IpfsClient::mfs_put(self, cid, path).await
    }

    async fn mfs_remove(&self, path: &str) -> Result<()> {
        IpfsClient::mfs_remove(self, path).await
    }

    async fn mfs_list(&self, path: &str) -> Result<Vec<MfsEntry>> {
        IpfsClient::mfs_list(self, path).await
    }

    async fn mfs_stat_cid(&self, path: &str) -> Result<Option<String>> {
        IpfsClient::mfs_stat_cid(self, path).await
    }

    async fn is_directory(&self, cid: &str) -> Result<bool> {
        IpfsClient::is_directory(self, cid).await
    }
}

pub(crate) fn percent_encode_segment(segment: &str) -> String {
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
        // metadata() follows symlinks (unlike DirEntry::file_type()), so linked files/dirs aren't skipped.
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
struct DagStatResponse {
    #[serde(rename = "TotalSize")]
    total_size: u64,
}

#[derive(Debug, Deserialize)]
struct FilesStatResponse {
    #[serde(rename = "Hash")]
    hash: String,
    #[serde(rename = "Type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct FilesLsResponse {
    #[serde(rename = "Entries")]
    entries: Option<Vec<FilesLsEntry>>,
}

#[derive(Debug, Deserialize)]
struct FilesLsEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Type")]
    kind: u8,
    #[serde(rename = "Hash")]
    hash: String,
}

const MFS_MISSING: &str = "file does not exist";

fn query_path(path: &str) -> String {
    percent_encode_relative_path(path)
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

    async fn call(&self, endpoint: &str, query: &str, timeout: Duration) -> Result<String> {
        let resp = self
            .http
            .post(self.url(&format!("/api/v0/{endpoint}?{query}")))
            .timeout(timeout)
            .send()
            .await
            .with_context(|| format!("POST /api/v0/{endpoint}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .with_context(|| format!("reading {endpoint} response"))?;
        if !status.is_success() {
            bail!("{endpoint} failed: {status}: {}", text.trim());
        }
        Ok(text)
    }

    pub async fn add_dir(&self, dir: &Path, mfs_path: &str) -> Result<String> {
        if !dir.is_dir() {
            bail!("not a directory: {}", dir.display());
        }
        // Kubo infers the wrapping directory node from a shared filename prefix, so the root name must be included or files land as unlinked blobs.
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

        self.mfs_mkdir(mfs::parent(mfs_path)).await?;
        self.mfs_remove(mfs_path).await?;
        let url = self.url(&format!(
            "/api/v0/add?recursive=true&cid-version=1&pin=false&quieter=true&wrap-with-directory=false&to-files={}",
            query_path(mfs_path)
        ));
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
            bail!("ipfs add failed: {status}: {}", text.trim());
        }

        let last_line = text
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .context("empty response from ipfs add")?;
        let parsed: AddResponseLine =
            serde_json::from_str(last_line).context("parsing ipfs add response")?;
        Ok(parsed.hash)
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
                bail!("dag/export failed: {status}: {}", text.trim());
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

    pub async fn dag_size_local(&self, cids: &[&str]) -> Result<u64> {
        if cids.is_empty() {
            return Ok(0);
        }
        let args: String = cids
            .iter()
            .map(|cid| format!("arg={}&", urlencoding_cid(cid)))
            .collect();
        let url = self.url(&format!(
            "/api/v0/dag/stat?{args}progress=false&offline=true"
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
            bail!("dag/stat failed: {status}: {}", text.trim());
        }
        let parsed: DagStatResponse =
            serde_json::from_str(&text).context("parsing dag/stat response")?;
        Ok(parsed.total_size)
    }

    async fn mfs_mkdir(&self, path: &str) -> Result<()> {
        self.call(
            "files/mkdir",
            &format!("arg={}&parents=true", query_path(path)),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    pub async fn mfs_put(&self, cid: &str, path: &str) -> Result<()> {
        self.mfs_mkdir(mfs::parent(path)).await?;
        self.mfs_remove(path).await?;
        // offline=true: callers have already checked the whole DAG is local.
        self.call(
            "files/cp",
            &format!(
                "arg=/ipfs/{}&arg={}&offline=true",
                urlencoding_cid(cid),
                query_path(path)
            ),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    pub async fn mfs_remove(&self, path: &str) -> Result<()> {
        // files/rm reports failures as a 200 response with a message body.
        let text = self
            .call(
                "files/rm",
                &format!("arg={}&recursive=true&force=true", query_path(path)),
                Duration::from_secs(60),
            )
            .await?;
        if !text.trim().is_empty() {
            bail!("files/rm failed: {}", text.trim());
        }
        Ok(())
    }

    pub async fn mfs_list(&self, path: &str) -> Result<Vec<MfsEntry>> {
        let text = match self
            .call(
                "files/ls",
                &format!("arg={}&long=true", query_path(path)),
                Duration::from_secs(60),
            )
            .await
        {
            Ok(text) => text,
            Err(e) if e.to_string().contains(MFS_MISSING) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let parsed: FilesLsResponse =
            serde_json::from_str(&text).context("parsing files/ls response")?;
        Ok(parsed
            .entries
            .unwrap_or_default()
            .into_iter()
            .map(|e| MfsEntry {
                name: e.name,
                is_dir: e.kind == 1,
                cid: e.hash,
            })
            .collect())
    }

    pub async fn mfs_stat_cid(&self, path: &str) -> Result<Option<String>> {
        let text = match self
            .call(
                "files/stat",
                &format!("arg={}&hash=true", query_path(path)),
                Duration::from_secs(60),
            )
            .await
        {
            Ok(text) => text,
            Err(e) if e.to_string().contains(MFS_MISSING) => return Ok(None),
            Err(e) => return Err(e),
        };
        let parsed: FilesStatResponse =
            serde_json::from_str(&text).context("parsing files/stat response")?;
        Ok(Some(parsed.hash))
    }

    pub async fn is_directory(&self, cid: &str) -> Result<bool> {
        let text = self
            .call(
                "files/stat",
                &format!("arg=/ipfs/{}", urlencoding_cid(cid)),
                Duration::from_secs(60),
            )
            .await?;
        let parsed: FilesStatResponse =
            serde_json::from_str(&text).context("parsing files/stat response")?;
        Ok(parsed.kind == "directory")
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
