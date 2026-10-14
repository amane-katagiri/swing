use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::Deserialize;

use crate::kubo::ApiSecret;
use crate::mfs;

mod peering;
mod site;

pub use peering::{KuboNode, parse_peering_ls, parse_swarm_peers};
pub use site::{SiteEntry, SiteFile, SiteListing};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reachability {
    Public,
    Private,
    Unknown,
    Offline,
}

impl Reachability {
    pub fn token(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Unknown => "unknown",
            Self::Offline => "offline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatKind {
    CarrierGrade,
    Double,
}

impl NatKind {
    pub fn token(self) -> &'static str {
        match self {
            Self::CarrierGrade => "cgnat",
            Self::Double => "double-nat",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoNat {
    pub reachability: Reachability,
    pub nat: Option<NatKind>,
    pub reachable: Vec<String>,
    pub unreachable: Vec<String>,
    pub unknown: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AutoNatResponse {
    reachability: String,
    nat: Option<String>,
    reachable: Option<Vec<String>>,
    unreachable: Option<Vec<String>>,
    unknown: Option<Vec<String>>,
}

impl From<AutoNatResponse> for AutoNat {
    fn from(r: AutoNatResponse) -> Self {
        Self {
            reachability: match r.reachability.as_str() {
                "Public" => Reachability::Public,
                "Private" => Reachability::Private,
                _ => Reachability::Unknown,
            },
            nat: match r.nat.as_deref() {
                Some("cgnat") => Some(NatKind::CarrierGrade),
                Some("double-nat") => Some(NatKind::Double),
                _ => None,
            },
            reachable: r.reachable.unwrap_or_default(),
            unreachable: r.unreachable.unwrap_or_default(),
            unknown: r.unknown.unwrap_or_default(),
        }
    }
}

pub fn parse_autonat(text: &str) -> Result<AutoNat> {
    let parsed: AutoNatResponse =
        serde_json::from_str(text).context("parsing swarm/addrs/autonat response")?;
    Ok(parsed.into())
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

const ADD_QUERY: &str =
    "recursive=true&cid-version=1&pin=false&quieter=true&wrap-with-directory=false";

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
const MFS_ENTRY_EXISTS: &str = "directory already has entry by that name";
const BLOCK_MISSING: &str = "ipld: could not find";
const KUBO_NOT_ONLINE: &str = "this command must be run in online mode";

pub fn is_block_missing(e: &anyhow::Error) -> bool {
    format!("{e:#}").contains(BLOCK_MISSING)
}
const ADD_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
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

async fn with_idle_timeout<T>(
    work: impl Future<Output = Result<T>>,
    last_progress: &std::sync::Mutex<tokio::time::Instant>,
    idle: Duration,
    what: &str,
) -> Result<T> {
    tokio::pin!(work);
    loop {
        let deadline = *last_progress.lock().unwrap() + idle;
        tokio::select! {
            result = &mut work => return result,
            () = tokio::time::sleep_until(deadline) => {
                if *last_progress.lock().unwrap() + idle <= tokio::time::Instant::now() {
                    bail!("{what} made no progress for {idle:?}");
                }
            }
        }
    }
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
        let cid = self.add_site(SiteListing::read_async(dir).await?).await?;
        self.mfs_put(&cid, mfs_path).await?;
        Ok(cid)
    }

    // Not added with to-files: Kubo answers success without placing anything when the path already exists.
    pub async fn add_site(&self, site: SiteListing) -> Result<String> {
        self.add_listing(site, false).await
    }

    pub async fn hash_site(&self, site: SiteListing) -> Result<String> {
        self.add_listing(site, true).await
    }

    async fn add_listing(&self, site: SiteListing, only_hash: bool) -> Result<String> {
        if site.is_empty() {
            bail!("directory is empty: {}", site.dir().display());
        }
        let boundary = crate::auth::random_hex(16);
        let url = self.url(&format!("/api/v0/add?{ADD_QUERY}&only-hash={only_hash}"));
        let last_progress = std::sync::Arc::new(std::sync::Mutex::new(tokio::time::Instant::now()));
        let progress = std::sync::Arc::clone(&last_progress);
        let request = self
            .http
            .post(&url)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(reqwest::Body::wrap_stream(
                site.into_multipart(boundary)
                    .inspect(move |_| *progress.lock().unwrap() = tokio::time::Instant::now()),
            ));
        let text = with_idle_timeout(
            self.send(request, "add"),
            &last_progress,
            ADD_IDLE_TIMEOUT,
            "add",
        )
        .await?;

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
        self.mfs_copy(cid, path).await
    }

    pub async fn mfs_place(&self, cid: &str, path: &str) -> Result<bool> {
        self.mfs_mkdir(mfs::parent(path)).await?;
        match self.mfs_copy(cid, path).await {
            Ok(()) => Ok(true),
            Err(e) if format!("{e:#}").contains(MFS_ENTRY_EXISTS) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn mfs_copy(&self, cid: &str, path: &str) -> Result<()> {
        // offline=true: callers place content already fetched and check completeness afterwards, so a missing root fails fast instead of searching the network.
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

    pub async fn autonat(&self) -> Result<AutoNat> {
        match self
            .call("swarm/addrs/autonat", "", Duration::from_secs(10))
            .await
        {
            Ok(text) => parse_autonat(&text),
            Err(e) if format!("{e:#}").contains(KUBO_NOT_ONLINE) => Ok(AutoNat {
                reachability: Reachability::Offline,
                nat: None,
                reachable: Vec::new(),
                unreachable: Vec::new(),
                unknown: Vec::new(),
            }),
            Err(e) => Err(e),
        }
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
mod tests;
