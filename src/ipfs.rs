use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::Deserialize;

use crate::kubo::ApiSecret;
use crate::mfs;

mod site;

pub use site::{SiteEntry, SiteListing};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Bandwidth {
    #[serde(rename = "TotalIn")]
    pub total_in: u64,
    #[serde(rename = "TotalOut")]
    pub total_out: u64,
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

#[derive(Debug, Deserialize)]
struct LsResponse {
    #[serde(rename = "Objects")]
    objects: Vec<LsObject>,
}

#[derive(Debug, Deserialize)]
struct LsObject {
    #[serde(rename = "Links")]
    links: Option<Vec<LsLink>>,
}

#[derive(Debug, Deserialize)]
struct LsLink {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Hash")]
    hash: String,
    #[serde(rename = "Type")]
    kind: u8,
}

const UNIXFS_DIRECTORY: u8 = 1;

const MFS_MISSING: &str = "file does not exist";
const MAX_RESPONSE_BYTES: usize = 16 << 20;

async fn read_body(resp: reqwest::Response, what: &str) -> Result<String> {
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("reading {what} response"))?;
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            bail!("{what} response is larger than {MAX_RESPONSE_BYTES} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

pub fn kubo_http_client(api_secret: Option<&ApiSecret>) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(secret) = api_secret {
        headers.insert(reqwest::header::AUTHORIZATION, secret.authorization());
    }
    reqwest::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()
        .expect("reqwest client uses only built-in TLS options")
}

#[derive(Debug, Deserialize)]
struct IdResponse {
    #[serde(rename = "ID")]
    id: String,
}

impl IpfsClient {
    pub fn new(api: impl Into<String>) -> Self {
        Self::with_secret(api, None)
    }

    pub fn with_secret(api: impl Into<String>, api_secret: Option<&ApiSecret>) -> Self {
        Self {
            http: kubo_http_client(api_secret),
            api: api.into().trim_end_matches('/').to_string(),
        }
    }

    pub fn api_url(&self) -> &str {
        &self.api
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api, path)
    }

    async fn send(&self, request: reqwest::RequestBuilder, endpoint: &str) -> Result<String> {
        let resp = request
            .send()
            .await
            .with_context(|| format!("POST /api/v0/{endpoint}"))?;
        let status = resp.status();
        let text = read_body(resp, endpoint).await?;
        if !status.is_success() {
            bail!("{endpoint} failed: {status}: {}", text.trim());
        }
        Ok(text)
    }

    async fn call(&self, endpoint: &str, query: &str, timeout: Duration) -> Result<String> {
        let request = self
            .http
            .post(self.url(&format!("/api/v0/{endpoint}?{query}")))
            .timeout(timeout);
        self.send(request, endpoint).await
    }

    pub async fn add_dir(&self, dir: &Path, mfs_path: &str) -> Result<String> {
        self.add_site(SiteListing::read_async(dir).await?, mfs_path)
            .await
    }

    pub async fn add_site(&self, site: SiteListing, mfs_path: &str) -> Result<String> {
        if site.is_empty() {
            bail!("directory is empty: {}", site.dir().display());
        }
        let boundary = crate::auth::random_hex(16);
        self.mfs_mkdir(mfs::parent(mfs_path)).await?;
        self.mfs_remove(mfs_path).await?;
        let url = self.url(&format!(
            "/api/v0/add?recursive=true&cid-version=1&pin=false&quieter=true&wrap-with-directory=false&to-files={}",
            percent_encode_relative_path(mfs_path)
        ));
        let request = self
            .http
            .post(&url)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(reqwest::Body::wrap_stream(site.into_multipart(boundary)))
            .timeout(Duration::from_secs(300));
        let text = self.send(request, "add").await?;

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
            percent_encode_segment(cid)
        ));
        let fetch = async {
            let resp = tokio::time::timeout(limits.idle, self.http.post(&url).send())
                .await
                .context("dag/export stalled before the first block")?
                .context("POST /api/v0/dag/export")?;
            let status = resp.status();
            if !status.is_success() {
                let text = read_body(resp, "dag/export").await.unwrap_or_default();
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
            .map(|cid| format!("arg={}&", percent_encode_segment(cid)))
            .collect();
        let text = self
            .call(
                "dag/stat",
                &format!("{args}progress=false&offline=true"),
                Duration::from_secs(300),
            )
            .await?;
        let parsed: DagStatResponse =
            serde_json::from_str(&text).context("parsing dag/stat response")?;
        Ok(parsed.total_size)
    }

    async fn mfs_mkdir(&self, path: &str) -> Result<()> {
        self.call(
            "files/mkdir",
            &format!("arg={}&parents=true", percent_encode_relative_path(path)),
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
                percent_encode_segment(cid),
                percent_encode_relative_path(path)
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
                &format!(
                    "arg={}&recursive=true&force=true",
                    percent_encode_relative_path(path)
                ),
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
                &format!("arg={}&long=true", percent_encode_relative_path(path)),
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
                &format!("arg={}&hash=true", percent_encode_relative_path(path)),
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

    pub async fn list_files_local(&self, cid: &str, max_entries: usize) -> Result<Vec<String>> {
        let mut files = Vec::new();
        let mut entries = 0usize;
        let mut pending = vec![(cid.to_string(), String::new())];
        while let Some((dir_cid, prefix)) = pending.pop() {
            let text = self
                .call(
                    "ls",
                    &format!(
                        "arg={}&resolve-type=true&size=false&offline=true",
                        percent_encode_segment(&dir_cid)
                    ),
                    Duration::from_secs(60),
                )
                .await?;
            let parsed: LsResponse = serde_json::from_str(&text).context("parsing ls response")?;
            for link in parsed
                .objects
                .into_iter()
                .flat_map(|o| o.links.unwrap_or_default())
            {
                entries += 1;
                if entries > max_entries {
                    bail!("more than {max_entries} entries");
                }
                let path = if prefix.is_empty() {
                    link.name
                } else {
                    format!("{prefix}/{}", link.name)
                };
                if link.kind == UNIXFS_DIRECTORY {
                    pending.push((link.hash, path));
                } else {
                    files.push(path);
                }
            }
        }
        files.sort();
        Ok(files)
    }

    pub async fn peer_id(&self) -> Result<String> {
        let text = self.call("id", "", Duration::from_secs(10)).await?;
        let parsed: IdResponse = serde_json::from_str(&text).context("parsing id response")?;
        Ok(parsed.id)
    }

    pub async fn shutdown(&self, timeout: Duration) -> Result<()> {
        self.call("shutdown", "", timeout).await.map(drop)
    }

    pub async fn bandwidth(&self) -> Result<Bandwidth> {
        let text = self.call("stats/bw", "", Duration::from_secs(10)).await?;
        serde_json::from_str(&text).context("parsing stats/bw response")
    }

    pub async fn is_directory(&self, cid: &str) -> Result<bool> {
        let text = self
            .call(
                "files/stat",
                &format!("arg=/ipfs/{}", percent_encode_segment(cid)),
                Duration::from_secs(60),
            )
            .await?;
        let parsed: FilesStatResponse =
            serde_json::from_str(&text).context("parsing files/stat response")?;
        Ok(parsed.kind == "directory")
    }
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

    #[test]
    fn bandwidth_reads_the_totals_of_stats_bw() {
        let bw: Bandwidth =
            serde_json::from_str(r#"{"TotalIn":123,"TotalOut":456,"RateIn":1.5,"RateOut":2.25}"#)
                .unwrap();
        assert_eq!(
            bw,
            Bandwidth {
                total_in: 123,
                total_out: 456
            }
        );
    }

    fn fake_ls() -> axum::Router {
        use axum::extract::Query;
        use std::collections::HashMap;

        axum::Router::new().route(
            "/api/v0/ls",
            axum::routing::post(|Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q.get("offline").map(String::as_str), Some("true"));
                let links = match q["arg"].as_str() {
                    "root" => serde_json::json!([
                        { "Name": "index.html", "Hash": "f1", "Type": 2 },
                        { "Name": "css", "Hash": "css", "Type": 1 },
                        { "Name": "big", "Hash": "big", "Type": 1 },
                    ]),
                    "css" => serde_json::json!([{ "Name": "a b.css", "Hash": "f2", "Type": 2 }]),
                    "big" => serde_json::json!([{ "Name": "x", "Hash": "f3", "Type": 0 }]),
                    other => panic!("unexpected ls of {other}"),
                };
                axum::Json(serde_json::json!({ "Objects": [{ "Hash": q["arg"], "Links": links }] }))
            }),
        )
    }

    #[tokio::test]
    async fn list_files_local_walks_directories_and_caps_the_entries() {
        let addr = crate::test_support::serve_router(fake_ls()).await;
        let client = IpfsClient::new(format!("http://{addr}"));
        assert_eq!(
            client.list_files_local("root", 5).await.unwrap(),
            vec!["big/x", "css/a b.css", "index.html"]
        );
        let err = client.list_files_local("root", 4).await.unwrap_err();
        assert!(err.to_string().contains("more than 4 entries"), "{err}");
    }

    #[tokio::test]
    async fn read_body_refuses_responses_over_the_cap() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let len = MAX_RESPONSE_BYTES + 1;
            let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {len}\r\n\r\n");
            sock.write_all(head.as_bytes()).await.unwrap();
            let chunk = vec![b'x'; 1 << 16];
            let mut sent = 0;
            while sent < len {
                let n = chunk.len().min(len - sent);
                if sock.write_all(&chunk[..n]).await.is_err() {
                    break;
                }
                sent += n;
            }
        });
        let client = IpfsClient::new(format!("http://{addr}"));
        let err = client
            .call("stats/bw", "", Duration::from_secs(10))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
        server.abort();
    }

    #[tokio::test]
    #[ignore]
    async fn kubo_client_probe() {
        let Ok(api) = std::env::var("SWING_TEST_KUBO_PROBE_API") else {
            return;
        };
        IpfsClient::new(api)
            .call("id", "", Duration::from_secs(5))
            .await
            .unwrap();
    }

    #[test]
    fn kubo_client_ignores_proxy_environment() {
        use std::io::{Read, Write};
        let kubo = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let kubo_addr = kubo.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = kubo.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
                .unwrap();
        });
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ipfs::tests::kubo_client_probe",
                "--ignored",
                "--quiet",
            ])
            .env("SWING_TEST_KUBO_PROBE_API", format!("http://{kubo_addr}"))
            .env("HTTP_PROXY", &proxy_url)
            .env("http_proxy", &proxy_url)
            .env("ALL_PROXY", &proxy_url)
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .status()
            .unwrap();
        assert!(status.success());
        server.join().unwrap();
        assert_eq!(
            proxy.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "the Kubo client must not go through the proxy"
        );
    }
}
