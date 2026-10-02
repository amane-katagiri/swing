use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::ipfs::IpfsClient;

pub const KUBO_VERSION: &str = "0.43.1";

const API_ACCESS_FILE: &str = "kubo-api.json";
const API_SECRET_BYTES: usize = 32;

const SHUTDOWN_RPC_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const SIGTERM_GRACE: Duration = Duration::from_secs(10);
#[cfg(unix)]
const ESCALATION: Duration = SIGTERM_GRACE;
#[cfg(not(unix))]
const ESCALATION: Duration = Duration::ZERO;

const ORPHAN_SHUTDOWN_RPC_TIMEOUT: Duration = Duration::from_secs(3);
const ORPHAN_SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
#[cfg(unix)]
const ORPHAN_SIGTERM_GRACE: Duration = Duration::from_secs(30);
const ORPHAN_KILL_WAIT: Duration = Duration::from_secs(10);

pub const fn daemon_stop_budget(grace: Duration) -> Duration {
    SHUTDOWN_RPC_TIMEOUT
        .saturating_add(grace)
        .saturating_add(ESCALATION)
}

#[derive(Clone, PartialEq, Eq)]
pub struct ApiSecret(String);

impl ApiSecret {
    pub fn generate() -> Self {
        Self(crate::auth::random_hex(API_SECRET_BYTES))
    }

    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(text.to_string()))
    }

    pub fn authorization(&self) -> reqwest::header::HeaderValue {
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.0))
            .expect("a hex secret is a valid header value");
        value.set_sensitive(true);
        value
    }

    fn kubo_authorizations(&self) -> serde_json::Value {
        json!({
            "swing": {
                "AuthSecret": format!("bearer:{}", self.0),
                "AllowedPaths": ["/api/v0"],
            }
        })
    }
}

impl std::fmt::Debug for ApiSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiAccess {
    pub port: u16,
    pub secret: ApiSecret,
}

#[derive(Serialize, Deserialize)]
struct ApiAccessFile {
    port: u16,
    secret: String,
}

impl ApiAccess {
    pub fn generate(port: u16) -> Self {
        Self {
            port,
            secret: ApiSecret::generate(),
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn multiaddr(&self) -> String {
        format!("/ip4/127.0.0.1/tcp/{}", self.port)
    }

    pub fn client(&self) -> IpfsClient {
        IpfsClient::with_secret(self.url(), Some(&self.secret))
    }
}

fn api_access_path(state_dir: &Path) -> PathBuf {
    state_dir.join(API_ACCESS_FILE)
}

pub fn write_api_access(state_dir: &Path, access: &ApiAccess) -> Result<()> {
    crate::auth::create_private_dir_all(state_dir)?;
    let file = ApiAccessFile {
        port: access.port,
        secret: access.secret.0.clone(),
    };
    let text = serde_json::to_string(&file).context("serializing the Kubo API access")?;
    crate::auth::write_private_file(&api_access_path(state_dir), &format!("{text}\n"))
}

pub fn read_api_access(state_dir: &Path) -> Result<Option<ApiAccess>> {
    let path = api_access_path(state_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let file: ApiAccessFile =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let secret = ApiSecret::parse(&file.secret)
        .with_context(|| format!("{} does not hold a hex secret", path.display()))?;
    Ok(Some(ApiAccess {
        port: file.port,
        secret,
    }))
}

pub fn remove_api_access(state_dir: &Path) -> Result<()> {
    remove_if_present(&api_access_path(state_dir))
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

pub async fn managed_client(state_dir: &Path, repo: &Path) -> Result<IpfsClient> {
    let Some(access) = read_api_access(state_dir)? else {
        bail!(
            "Kubo is not running: {} does not exist (start `swing up`, or set [kubo].managed = false and [ipfs].api to use an external Kubo)",
            api_access_path(state_dir).display()
        );
    };
    let client = access.client();
    ensure_own_daemon(&client, repo).await?;
    Ok(client)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KuboSettings {
    pub storage_max: u64,
    pub provide_strategy: String,
    pub api: ApiAccess,
    pub gateway: SocketAddr,
    pub swarm_port: Option<u16>,
    pub public_gateway_hosts: Vec<String>,
}

pub fn multiaddr_to_http_url(addr: &str) -> Result<String> {
    let parts: Vec<&str> = addr.trim().trim_start_matches('/').split('/').collect();
    let [proto, host, "tcp", port] = parts.as_slice() else {
        bail!("unsupported multiaddr (expected /ip4|ip6|dns4|dns6|dns/<host>/tcp/<port>): {addr}");
    };
    let port: u16 = port
        .parse()
        .with_context(|| format!("invalid port in multiaddr: {addr}"))?;
    let host = match *proto {
        "ip4" | "dns4" | "dns6" | "dns" => host.to_string(),
        "ip6" => format!("[{host}]"),
        other => bail!("unsupported multiaddr protocol {other}: {addr}"),
    };
    if host.is_empty() || host == "[]" {
        bail!("empty host in multiaddr: {addr}");
    }
    Ok(format!("http://{host}:{port}"))
}

pub fn locate_binary(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if !path.is_file() {
            bail!("Kubo binary not found: {}", path.display());
        }
        return Ok(path.to_path_buf());
    }

    let exe_name = if cfg!(windows) { "ipfs.exe" } else { "ipfs" };

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            tracing::info!(
                path = %candidate.display(),
                "using Kubo binary found next to the swing executable"
            );
            return Ok(candidate);
        }
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(exe_name);
            if candidate.is_file() {
                tracing::info!(
                    path = %candidate.display(),
                    "using Kubo binary found on PATH"
                );
                return Ok(candidate);
            }
        }
    }

    bail!(
        "Kubo binary not found: no [kubo].binary configured, no {exe_name} next to the swing executable, and no {exe_name} on PATH"
    );
}

async fn run(bin: &Path, repo: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new(bin);
    command.args(args).stdin(Stdio::null());
    if let Some(repo) = repo {
        command.env("IPFS_PATH", repo);
    }
    let output = command
        .output()
        .await
        .with_context(|| format!("running `{} {}`", bin.display(), args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "`{} {}` failed ({}): {}",
            bin.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub async fn version(bin: &Path) -> Result<String> {
    Ok(run(bin, None, &["version", "--number"])
        .await?
        .trim()
        .to_string())
}

pub async fn ensure_repo(bin: &Path, repo: &Path) -> Result<bool> {
    std::fs::create_dir_all(repo)
        .with_context(|| format!("creating Kubo repo dir {}", repo.display()))?;
    if repo.join("config").is_file() {
        return Ok(false);
    }
    run(bin, Some(repo), &["init"]).await?;
    Ok(true)
}

fn public_gateways_json(hosts: &[String]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for host in hosts {
        map.insert(
            host.clone(),
            json!({
                "Paths": [],
                "UseSubdomains": false,
                "NoDNSLink": false,
            }),
        );
    }
    serde_json::Value::Object(map)
}

fn default_swarm_addrs(port: u16) -> Vec<String> {
    vec![
        format!("/ip4/0.0.0.0/tcp/{port}"),
        format!("/ip6/::/tcp/{port}"),
        format!("/ip4/0.0.0.0/udp/{port}/webrtc-direct"),
        format!("/ip4/0.0.0.0/udp/{port}/quic-v1"),
        format!("/ip4/0.0.0.0/udp/{port}/quic-v1/webtransport"),
        format!("/ip6/::/udp/{port}/webrtc-direct"),
        format!("/ip6/::/udp/{port}/quic-v1"),
        format!("/ip6/::/udp/{port}/quic-v1/webtransport"),
    ]
}

fn gateway_multiaddr(addr: SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(ip) => format!("/ip4/{ip}/tcp/{}", addr.port()),
        IpAddr::V6(ip) => format!("/ip6/{ip}/tcp/{}", addr.port()),
    }
}

async fn set_config(bin: &Path, repo: &Path, key: &str, value: &str) -> Result<()> {
    run(bin, Some(repo), &["config", key, value]).await?;
    Ok(())
}

async fn set_config_json(
    bin: &Path,
    repo: &Path,
    key: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let value = serde_json::to_string(value).with_context(|| format!("serializing {key}"))?;
    run(bin, Some(repo), &["config", "--json", key, &value]).await?;
    Ok(())
}

pub async fn apply_config(bin: &Path, repo: &Path, s: &KuboSettings) -> Result<()> {
    set_config(
        bin,
        repo,
        "Datastore.StorageMax",
        &s.storage_max.to_string(),
    )
    .await?;
    set_config(bin, repo, "Provide.Strategy", &s.provide_strategy).await?;

    set_config_json(bin, repo, "Gateway.NoFetch", &json!(true)).await?;
    set_config_json(bin, repo, "Gateway.NoDNSLink", &json!(true)).await?;
    set_config_json(
        bin,
        repo,
        "Gateway.PublicGateways",
        &public_gateways_json(&s.public_gateway_hosts),
    )
    .await?;

    set_config_json(
        bin,
        repo,
        "Addresses.Gateway",
        &json!([gateway_multiaddr(s.gateway)]),
    )
    .await?;

    if let Some(port) = s.swarm_port {
        set_config_json(
            bin,
            repo,
            "Addresses.Swarm",
            &json!(default_swarm_addrs(port)),
        )
        .await?;
    }

    set_api_access(repo, &s.api)
}

// Edits the file directly because `ipfs config` would put the port and secret on a command line other users can read.
fn set_api_access(repo: &Path, access: &ApiAccess) -> Result<()> {
    let path = repo.join("config");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut config: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let root = config
        .as_object_mut()
        .with_context(|| format!("{} is not a JSON object", path.display()))?;
    object_entry(root, "Addresses", &path)?.insert("API".to_string(), json!([access.multiaddr()]));
    object_entry(root, "API", &path)?.insert(
        "Authorizations".to_string(),
        access.secret.kubo_authorizations(),
    );
    let text = serde_json::to_string_pretty(&config).context("serializing the Kubo config")?;
    crate::settings::write_atomic(&path, &text)
}

fn object_entry<'a>(
    root: &'a mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    path: &Path,
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>> {
    let value = root.entry(key).or_insert_with(|| json!({}));
    if value.is_null() {
        *value = json!({});
    }
    value
        .as_object_mut()
        .with_context(|| format!("{key} in {} is not a JSON object", path.display()))
}

fn pid_file_path(state_dir: &Path) -> PathBuf {
    state_dir.join("kubo.pid")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PidRecord {
    pid: u32,
    api_port: u16,
    started_at: String,
}

pub fn write_pid_file(state_dir: &Path, pid: u32, api_port: u16) -> Result<()> {
    let started_at = process_start_marker(pid)
        .with_context(|| format!("determining the start time of Kubo process {pid}"))?;
    let record = PidRecord {
        pid,
        api_port,
        started_at,
    };
    let path = pid_file_path(state_dir);
    let text = serde_json::to_string(&record).context("serializing kubo.pid")?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

fn read_pid_file(state_dir: &Path) -> Result<Option<PidRecord>> {
    let path = pid_file_path(state_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let record: PidRecord =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(record))
}

pub fn remove_pid_file(state_dir: &Path) -> Result<()> {
    let path = pid_file_path(state_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(any(test, target_os = "linux"))]
fn parse_proc_stat_starttime(stat: &str) -> Option<String> {
    crate::proc::stat_fields(stat)?
        .nth(19)
        .map(|s| s.to_string())
}

#[cfg(target_os = "linux")]
fn process_start_marker(pid: u32) -> Option<String> {
    parse_proc_stat_starttime(&crate::proc::read_stat(pid)?)
}

#[cfg(target_os = "macos")]
fn process_start_marker(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(windows)]
fn process_start_marker(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(format!(
            "{}-{}",
            creation.dwHighDateTime, creation.dwLowDateTime
        ))
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if ret == 0 {
        return true;
    }
    // ESRCH doesn't reliably map to io::ErrorKind::NotFound, so compare the raw errno instead.
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

async fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    wait_until(timeout, Duration::from_millis(500), || async {
        !process_alive(pid)
    })
    .await
}

async fn attempt_graceful_shutdown(api_port: u16, pid: u32, secret: Option<&ApiSecret>) -> bool {
    let url = format!("http://127.0.0.1:{api_port}/api/v0/shutdown");
    let responded = crate::ipfs::kubo_http_client(secret)
        .post(&url)
        .timeout(ORPHAN_SHUTDOWN_RPC_TIMEOUT)
        .send()
        .await
        .is_ok_and(|resp| resp.status().is_success());
    if !responded {
        return false;
    }
    wait_for_exit(pid, ORPHAN_SHUTDOWN_GRACE).await
}

#[cfg(unix)]
fn send_signal(pid: u32, signal: libc::c_int) -> std::io::Result<()> {
    let ret = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn terminate_process(pid: u32) -> Result<()> {
    send_signal(pid, libc::SIGTERM).context("sending SIGTERM to orphaned Kubo")?;
    if wait_for_exit(pid, ORPHAN_SIGTERM_GRACE).await {
        return Ok(());
    }
    tracing::warn!(
        pid,
        "orphaned Kubo did not exit after SIGTERM, sending SIGKILL"
    );
    send_signal(pid, libc::SIGKILL).context("sending SIGKILL to orphaned Kubo")?;
    if wait_for_exit(pid, ORPHAN_KILL_WAIT).await {
        return Ok(());
    }
    bail!("orphaned Kubo (pid {pid}) did not exit after SIGKILL")
}

#[cfg(windows)]
async fn terminate_process(pid: u32) -> Result<()> {
    let output = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .output()
        .await
        .context("running taskkill")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("taskkill failed for pid {pid}: {}", stderr.trim());
    }
    if wait_for_exit(pid, ORPHAN_KILL_WAIT).await {
        Ok(())
    } else {
        bail!("orphaned Kubo (pid {pid}) did not exit after taskkill /F")
    }
}

pub async fn recover_orphan(state_dir: &Path, repo: &Path) -> Result<()> {
    let record = match read_pid_file(state_dir) {
        Ok(Some(record)) => record,
        Ok(None) => return Ok(()),
        Err(e) => {
            tracing::warn!(
                error = %e,
                repo = %repo.display(),
                "kubo.pid is not a valid pid file; leaving its process alone"
            );
            remove_pid_file(state_dir)?;
            return Ok(());
        }
    };

    let Some(current_started_at) = process_start_marker(record.pid) else {
        tracing::info!(
            pid = record.pid,
            repo = %repo.display(),
            "stale kubo.pid (process is no longer running)"
        );
        remove_pid_file(state_dir)?;
        return Ok(());
    };
    if current_started_at != record.started_at {
        tracing::warn!(
            pid = record.pid,
            repo = %repo.display(),
            "pid in kubo.pid no longer belongs to the recorded Kubo (start time differs); leaving it alone"
        );
        remove_pid_file(state_dir)?;
        return Ok(());
    }

    let secret = read_api_access(state_dir)
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "cannot read the Kubo API access; asking the orphan to shut down without it");
            None
        })
        .filter(|access| access.port == record.api_port)
        .map(|access| access.secret);
    if attempt_graceful_shutdown(record.api_port, record.pid, secret.as_ref()).await {
        tracing::info!(
            pid = record.pid,
            repo = %repo.display(),
            "orphaned Kubo shut down gracefully via its API"
        );
        remove_pid_file(state_dir)?;
        return Ok(());
    }

    tracing::warn!(
        pid = record.pid,
        repo = %repo.display(),
        "terminating orphaned Kubo left by a previous swing"
    );
    terminate_process(record.pid).await?;
    remove_pid_file(state_dir)?;
    Ok(())
}

pub fn pick_free_port() -> Result<u16> {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").context("binding an ephemeral port")?;
    let port = listener
        .local_addr()
        .context("reading ephemeral port")?
        .port();
    Ok(port)
}

async fn wait_until<F, Fut>(timeout: Duration, interval: Duration, mut ready: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if ready().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(interval).await;
    }
}

fn is_repo_lock_error(line: &str) -> bool {
    line.contains("repo.lock") || line.contains("someone else has the lock")
}

fn forward_lines<R>(reader: R, stream: &'static str, saw_repo_lock: Option<Arc<AtomicBool>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(flag) = &saw_repo_lock
                        && is_repo_lock_error(&line)
                    {
                        flag.store(true, Ordering::Relaxed);
                    }
                    tracing::info!(target: "kubo", stream, "{line}");
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(target: "kubo", stream, "error reading output: {e}");
                    break;
                }
            }
        }
    });
}

pub struct Daemon {
    child: Child,
    api_url: String,
    ipfs: IpfsClient,
    saw_repo_lock: Arc<AtomicBool>,
    // Held only for its Drop: closing the Job Object handle kills the child.
    #[cfg(windows)]
    #[allow(dead_code)]
    job: windows_job::JobHandle,
}

impl Daemon {
    pub async fn spawn(bin: &Path, repo: &Path, api: &ApiAccess) -> Result<Daemon> {
        remove_if_present(&repo.join("api"))?;
        let mut command = Command::new(bin);
        command
            .args([
                "daemon",
                "--migrate=true",
                "--enable-gc",
                "--agent-version-suffix=swing",
            ])
            .env("IPFS_PATH", repo)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(target_os = "linux")]
        unsafe {
            command.pre_exec(|| {
                let ret = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                if ret != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning `{} daemon`", bin.display()))?;

        #[cfg(windows)]
        let job = windows_job::assign_child_to_job(&child)?;

        let saw_repo_lock = Arc::new(AtomicBool::new(false));
        if let Some(stdout) = child.stdout.take() {
            forward_lines(stdout, "stdout", None);
        }
        if let Some(stderr) = child.stderr.take() {
            forward_lines(stderr, "stderr", Some(Arc::clone(&saw_repo_lock)));
        }

        Ok(Daemon {
            child,
            api_url: api.url(),
            ipfs: api.client(),
            saw_repo_lock,
            #[cfg(windows)]
            job,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn ipfs(&self) -> &IpfsClient {
        &self.ipfs
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub fn saw_repo_lock_error(&self) -> bool {
        self.saw_repo_lock.load(Ordering::Relaxed)
    }

    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    pub async fn stop(mut self, grace: Duration) -> Result<()> {
        match self.ipfs.shutdown(SHUTDOWN_RPC_TIMEOUT).await {
            Ok(()) => tracing::debug!("kubo RPC shutdown requested"),
            Err(e) => {
                tracing::debug!(error = %format!("{e:#}"), "kubo RPC shutdown request failed (daemon may be exiting anyway)");
            }
        }

        match tokio::time::timeout(grace, self.child.wait()).await {
            Ok(Ok(_)) => {
                tracing::debug!("kubo exited after RPC shutdown");
                return Ok(());
            }
            Ok(Err(e)) => return Err(e).context("waiting for Kubo to exit"),
            Err(_) => {
                tracing::debug!(
                    "kubo did not exit within {grace:?} of the RPC shutdown request; escalating"
                );
            }
        }

        #[cfg(unix)]
        {
            if let Some(pid) = self.child.id()
                && let Err(err) = send_signal(pid, libc::SIGTERM)
            {
                tracing::warn!("failed to send SIGTERM to Kubo (pid {pid}): {err}");
            }
            match tokio::time::timeout(SIGTERM_GRACE, self.child.wait()).await {
                Ok(Ok(_)) => {
                    tracing::debug!("kubo exited after SIGTERM");
                    return Ok(());
                }
                Ok(Err(e)) => return Err(e).context("waiting for Kubo to exit"),
                Err(_) => {
                    tracing::warn!("Kubo did not exit after SIGTERM, killing it");
                }
            }
        }

        self.child.kill().await.context("killing Kubo")?;
        self.child
            .wait()
            .await
            .context("waiting for Kubo to exit")?;
        tracing::debug!("kubo killed");
        Ok(())
    }
}

// A Job Object is the closest Windows equivalent to Linux's PR_SET_PDEATHSIG.
#[cfg(windows)]
mod windows_job {
    use anyhow::{Result, bail};
    use tokio::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    pub struct JobHandle(HANDLE);

    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    impl Drop for JobHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn assign_child_to_job(child: &Child) -> Result<JobHandle> {
        let Some(raw) = child.raw_handle() else {
            bail!("Kubo child process has already exited; cannot assign it to a Job Object");
        };
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                bail!("CreateJobObjectW failed: {}", GetLastError());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let err = GetLastError();
                CloseHandle(job);
                bail!("SetInformationJobObject failed: {err}");
            }
            let ok = AssignProcessToJobObject(job, raw as HANDLE);
            if ok == 0 {
                let err = GetLastError();
                CloseHandle(job);
                bail!("AssignProcessToJobObject failed: {err}");
            }
            Ok(JobHandle(job))
        }
    }
}

pub async fn ensure_own_daemon(ipfs: &IpfsClient, repo: &Path) -> Result<()> {
    let expected = read_peer_id(repo)?;
    let answered = ipfs
        .peer_id()
        .await
        .context("asking the Kubo API for its peer ID")?;
    if answered != expected {
        bail!(
            "the Kubo API at {} answered with peer ID {answered}, not this repo's {expected}; it is not the Kubo `swing up` started (is `swing up` running?)",
            ipfs.api_url()
        );
    }
    Ok(())
}

fn read_peer_id(repo: &Path) -> Result<String> {
    let path = repo.join("config");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let config: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    config["Identity"]["PeerID"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .with_context(|| format!("{} has no Identity.PeerID", path.display()))
}

pub async fn wait_healthy(ipfs: &IpfsClient, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let err = match ipfs.peer_id().await {
            Ok(_) => return Ok(()),
            Err(e) => e,
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(err.context(format!(
                "Kubo at {} did not become healthy within {timeout:?}",
                ipfs.api_url()
            )));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

impl Daemon {
    fn owns_api_file(&self, repo: &Path) -> bool {
        std::fs::read_to_string(repo.join("api"))
            .ok()
            .and_then(|text| multiaddr_to_http_url(text.trim()).ok())
            .is_some_and(|url| url == self.api_url)
    }

    // Waits for Kubo to write <repo>/api before sending the secret, since that file appears only after this child bound the port.
    pub async fn wait_healthy(&mut self, repo: &Path, timeout: Duration) -> Result<()> {
        let expected = read_peer_id(repo)?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut foreign: Option<String> = None;
        loop {
            if let Some(status) = self.child.try_wait().context("checking the Kubo daemon")? {
                bail!("Kubo exited ({status}) before becoming healthy");
            }
            if self.owns_api_file(repo) {
                match self.ipfs.peer_id().await {
                    Ok(id) if id == expected => return Ok(()),
                    Ok(id) => foreign = Some(id),
                    Err(_) => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                match foreign {
                    Some(got) => bail!(
                        "Kubo did not become healthy within {timeout:?}: {} answered with peer ID {got:?}, expected {expected}",
                        self.api_url
                    ),
                    None => bail!(
                        "Kubo did not become healthy within {timeout:?} ({})",
                        self.api_url
                    ),
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_kubo_bin() -> Option<PathBuf> {
        match std::env::var("SWING_TEST_KUBO_BIN") {
            Ok(bin) => Some(PathBuf::from(bin)),
            Err(_) => {
                eprintln!("skipping: SWING_TEST_KUBO_BIN not set");
                None
            }
        }
    }

    #[test]
    fn converts_ip4_and_ip6_multiaddrs() {
        assert_eq!(
            multiaddr_to_http_url("/ip4/127.0.0.1/tcp/5001").unwrap(),
            "http://127.0.0.1:5001"
        );
        assert_eq!(
            multiaddr_to_http_url("/ip6/::1/tcp/5001\n").unwrap(),
            "http://[::1]:5001"
        );
        assert!(multiaddr_to_http_url("/unix/tmp/api.sock").is_err());
        assert!(multiaddr_to_http_url("/ip4/127.0.0.1/udp/5001").is_err());
    }

    #[tokio::test]
    async fn missing_api_access_says_kubo_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("api"), "/ip4/127.0.0.1/tcp/41234\n").unwrap();
        let Err(err) = managed_client(dir.path(), dir.path()).await else {
            panic!("a missing API access file must not yield a client");
        };
        assert!(err.to_string().contains("Kubo is not running"), "{err}");
    }

    #[test]
    fn public_gateways_json_matches_shell_script_shape() {
        let hosts = vec!["a.example.com".to_string(), "b.example.com".to_string()];
        let value = public_gateways_json(&hosts);
        assert_eq!(
            value,
            json!({
                "a.example.com": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false},
                "b.example.com": {"Paths": [], "UseSubdomains": false, "NoDNSLink": false},
            })
        );
    }

    #[test]
    fn public_gateways_json_empty_when_no_hosts() {
        assert_eq!(public_gateways_json(&[]), json!({}));
    }

    #[test]
    fn default_swarm_addrs_substitutes_port() {
        let addrs = default_swarm_addrs(4321);
        assert_eq!(
            addrs,
            vec![
                "/ip4/0.0.0.0/tcp/4321",
                "/ip6/::/tcp/4321",
                "/ip4/0.0.0.0/udp/4321/webrtc-direct",
                "/ip4/0.0.0.0/udp/4321/quic-v1",
                "/ip4/0.0.0.0/udp/4321/quic-v1/webtransport",
                "/ip6/::/udp/4321/webrtc-direct",
                "/ip6/::/udp/4321/quic-v1",
                "/ip6/::/udp/4321/quic-v1/webtransport",
            ]
        );
    }

    #[test]
    fn gateway_multiaddr_formats_v4_and_v6() {
        assert_eq!(
            gateway_multiaddr("127.0.0.1:8080".parse().unwrap()),
            "/ip4/127.0.0.1/tcp/8080"
        );
        assert_eq!(
            gateway_multiaddr("[::1]:8080".parse().unwrap()),
            "/ip6/::1/tcp/8080"
        );
    }

    #[test]
    fn storage_max_is_decimal_bytes() {
        assert_eq!((100u64 * (1u64 << 30)).to_string(), "107374182400");
    }

    #[test]
    fn repo_lock_detection_ignores_unrelated_lock_words() {
        assert!(is_repo_lock_error("Error: someone else has the lock"));
        assert!(is_repo_lock_error(
            "cannot acquire lock: Lock FcntlFlock of /x/repo.lock failed"
        ));
        assert!(!is_repo_lock_error("fetched block bafy from peer"));
        assert!(!is_repo_lock_error("blockstore: 12 blocks"));
    }

    #[test]
    fn read_peer_id_reads_identity_from_repo_config() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_peer_id(dir.path()).is_err());
        std::fs::write(dir.path().join("config"), r#"{"Identity":{}}"#).unwrap();
        assert!(read_peer_id(dir.path()).is_err());
        std::fs::write(
            dir.path().join("config"),
            r#"{"Identity":{"PeerID":"12D3KooWmine"}}"#,
        )
        .unwrap();
        assert_eq!(read_peer_id(dir.path()).unwrap(), "12D3KooWmine");
    }

    #[cfg(unix)]
    fn fake_kubo(dir: &Path, script: &str, peer_id: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-ipfs");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            dir.join("config"),
            format!(r#"{{"Identity":{{"PeerID":"{peer_id}"}}}}"#),
        )
        .unwrap();
        path
    }

    #[cfg(unix)]
    fn writes_api_file_then_sleeps(port: u16) -> String {
        format!("echo /ip4/127.0.0.1/tcp/{port} > \"$IPFS_PATH/api\"\nexec sleep 30")
    }

    struct IdServer {
        port: u16,
        requests: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl IdServer {
        fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    async fn serve(status: &'static str, body: String) -> IdServer {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let seen = Arc::clone(&seen);
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    seen.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase());
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });
        IdServer { port, requests }
    }

    async fn serve_id(peer_id: &str) -> IdServer {
        serve("200 OK", format!(r#"{{"ID":"{peer_id}"}}"#)).await
    }

    fn access(port: u16) -> ApiAccess {
        ApiAccess {
            port,
            secret: ApiSecret::parse("abcd").unwrap(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_wait_healthy_fails_fast_when_the_child_exits() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_kubo(dir.path(), "exit 3", "mine");
        let server = serve_id("mine").await;
        let mut daemon = Daemon::spawn(&bin, dir.path(), &access(server.port))
            .await
            .unwrap();
        let _ = daemon.wait().await;
        let err = daemon
            .wait_healthy(dir.path(), Duration::from_secs(30))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("exited"), "{err}");
        assert!(server.requests().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_wait_healthy_requires_the_repo_peer_id() {
        let dir = tempfile::tempdir().unwrap();

        let foreign = serve_id("someone-else").await;
        let bin = fake_kubo(
            dir.path(),
            &writes_api_file_then_sleeps(foreign.port),
            "mine",
        );
        let mut daemon = Daemon::spawn(&bin, dir.path(), &access(foreign.port))
            .await
            .unwrap();
        let err = daemon
            .wait_healthy(dir.path(), Duration::from_secs(1))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("someone-else") && err.contains("mine"),
            "{err}"
        );
        daemon.stop(Duration::ZERO).await.unwrap();

        let own = serve_id("mine").await;
        let bin = fake_kubo(dir.path(), &writes_api_file_then_sleeps(own.port), "mine");
        let mut daemon = Daemon::spawn(&bin, dir.path(), &access(own.port))
            .await
            .unwrap();
        daemon
            .wait_healthy(dir.path(), Duration::from_secs(5))
            .await
            .unwrap();
        daemon.stop(Duration::ZERO).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_wait_healthy_sends_nothing_until_kubo_writes_the_api_file() {
        let dir = tempfile::tempdir().unwrap();
        let squatter = serve_id("mine").await;
        std::fs::write(
            dir.path().join("api"),
            format!("/ip4/127.0.0.1/tcp/{}", squatter.port),
        )
        .unwrap();
        let bin = fake_kubo(dir.path(), "exec sleep 30", "mine");
        let mut daemon = Daemon::spawn(&bin, dir.path(), &access(squatter.port))
            .await
            .unwrap();
        assert!(!dir.path().join("api").exists());
        assert!(
            daemon
                .wait_healthy(dir.path(), Duration::from_secs(2))
                .await
                .is_err()
        );
        daemon.stop(Duration::ZERO).await.unwrap();
        let requests = squatter.requests();
        assert!(
            requests.iter().all(|r| !r.contains("/api/v0/id")),
            "{requests:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_wait_healthy_refuses_without_a_readable_peer_id() {
        let dir = tempfile::tempdir().unwrap();
        let server = serve_id("mine").await;
        let bin = fake_kubo(
            dir.path(),
            &writes_api_file_then_sleeps(server.port),
            "mine",
        );
        std::fs::write(dir.path().join("config"), r#"{"Identity":{}}"#).unwrap();
        let mut daemon = Daemon::spawn(&bin, dir.path(), &access(server.port))
            .await
            .unwrap();
        let err = daemon
            .wait_healthy(dir.path(), Duration::from_secs(5))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("PeerID"), "{err}");
        daemon.stop(Duration::ZERO).await.unwrap();
        assert!(
            server.requests().iter().all(|r| !r.contains("/api/v0/id")),
            "no probe may carry the secret when the peer ID cannot be checked"
        );
    }

    #[tokio::test]
    async fn ensure_own_daemon_rejects_an_api_with_another_peer_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config"),
            r#"{"Identity":{"PeerID":"mine"}}"#,
        )
        .unwrap();
        let foreign = crate::ipfs::IpfsClient::new(serve_id("someone-else").await.url());
        let err = ensure_own_daemon(&foreign, dir.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("someone-else") && err.contains("not the Kubo"),
            "{err}"
        );
        let own = crate::ipfs::IpfsClient::new(serve_id("mine").await.url());
        ensure_own_daemon(&own, dir.path()).await.unwrap();
    }

    #[tokio::test]
    async fn managed_client_uses_the_port_stored_with_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config"),
            r#"{"Identity":{"PeerID":"mine"}}"#,
        )
        .unwrap();
        let stale = serve_id("someone-else").await;
        let own = serve_id("mine").await;
        std::fs::write(
            dir.path().join("api"),
            format!("/ip4/127.0.0.1/tcp/{}", stale.port),
        )
        .unwrap();
        write_api_access(dir.path(), &access(own.port)).unwrap();
        let client = managed_client(dir.path(), dir.path()).await.unwrap();
        assert_eq!(client.api_url(), own.url());
        assert!(stale.requests().is_empty());
        assert!(
            own.requests()[0].contains("authorization: bearer abcd"),
            "{:?}",
            own.requests()
        );
    }

    #[test]
    fn api_access_round_trips_through_a_private_file_and_stays_out_of_debug() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("state");
        assert!(read_api_access(&state_dir).unwrap().is_none());
        let first = ApiAccess::generate(4001);
        write_api_access(&state_dir, &first).unwrap();
        assert_eq!(read_api_access(&state_dir).unwrap(), Some(first.clone()));
        let second = ApiAccess::generate(4002);
        assert_ne!(first.secret, second.secret);
        write_api_access(&state_dir, &second).unwrap();
        assert_eq!(read_api_access(&state_dir).unwrap(), Some(second.clone()));
        assert!(!format!("{second:?}").contains(&second.secret.0));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(api_access_path(&state_dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(
            api_access_path(&state_dir),
            r#"{"port":4001,"secret":"not hex"}"#,
        )
        .unwrap();
        assert!(read_api_access(&state_dir).is_err());
        remove_api_access(&state_dir).unwrap();
        assert!(read_api_access(&state_dir).unwrap().is_none());
        remove_api_access(&state_dir).unwrap();
    }

    #[test]
    fn api_access_is_written_into_the_repo_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config"),
            r#"{"Identity":{"PeerID":"mine"},"API":{"HTTPHeaders":{}},"Addresses":{"Gateway":["/ip4/127.0.0.1/tcp/8080"]}}"#,
        )
        .unwrap();
        let api = access(41234);
        set_api_access(dir.path(), &api).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(config["Identity"]["PeerID"], "mine");
        assert_eq!(config["API"]["HTTPHeaders"], json!({}));
        assert_eq!(
            config["API"]["Authorizations"],
            json!({"swing": {"AuthSecret": "bearer:abcd", "AllowedPaths": ["/api/v0"]}})
        );
        assert_eq!(
            config["Addresses"],
            json!({"API": ["/ip4/127.0.0.1/tcp/41234"], "Gateway": ["/ip4/127.0.0.1/tcp/8080"]})
        );
        assert_eq!(api.secret.authorization(), "Bearer abcd");

        std::fs::write(dir.path().join("config"), r#"{"API":null}"#).unwrap();
        set_api_access(dir.path(), &api).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config")).unwrap();
        assert!(
            text.contains("bearer:abcd") && text.contains("/tcp/41234"),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn api_access_writes_through_a_symlinked_repo_config() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real-config");
        std::fs::write(&real, r#"{"Identity":{"PeerID":"mine"}}"#).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        std::os::unix::fs::symlink(&real, repo.join("config")).unwrap();
        set_api_access(&repo, &access(41234)).unwrap();
        assert!(
            std::fs::symlink_metadata(repo.join("config"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::read_to_string(&real)
                .unwrap()
                .contains("bearer:abcd")
        );
    }

    #[tokio::test]
    async fn kubo_http_client_sends_the_secret_as_a_bearer_token() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let body = r#"{"ID":"mine"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase()
        });
        let secret = ApiSecret::parse("abcd").unwrap();
        let client = crate::ipfs::IpfsClient::with_secret(format!("http://{addr}"), Some(&secret));
        assert_eq!(client.peer_id().await.unwrap(), "mine");
        let request = server.await.unwrap();
        assert!(request.contains("authorization: bearer abcd"), "{request}");
    }

    #[test]
    fn locate_binary_errors_on_missing_explicit_path() {
        let missing = PathBuf::from("/nonexistent/path/to/ipfs-binary-that-does-not-exist");
        let err = locate_binary(Some(&missing)).unwrap_err().to_string();
        assert!(err.contains("Kubo binary not found"), "{err}");
    }

    #[test]
    fn windows_installer_pins_the_same_kubo_version() {
        let pinned = include_str!("../packaging/windows/kubo.sha512");
        let fields: Vec<&str> = pinned.split_whitespace().collect();
        assert_eq!(fields.len(), 2, "{pinned}");
        assert_eq!(fields[0].len(), 128, "{pinned}");
        assert_eq!(fields[1], format!("kubo_v{KUBO_VERSION}_windows-amd64.zip"));
    }

    #[test]
    fn pick_free_port_returns_bindable_port() {
        let port = pick_free_port().unwrap();
        assert!(port > 0);
        std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    #[test]
    fn pid_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_pid_file(dir.path()).unwrap(), None);
        let pid = std::process::id();
        write_pid_file(dir.path(), pid, 4242).unwrap();
        let record = read_pid_file(dir.path()).unwrap().unwrap();
        assert_eq!(record.pid, pid);
        assert_eq!(record.api_port, 4242);
        assert!(!record.started_at.is_empty());
        remove_pid_file(dir.path()).unwrap();
        assert_eq!(read_pid_file(dir.path()).unwrap(), None);
        remove_pid_file(dir.path()).unwrap();
    }

    #[test]
    fn read_pid_file_rejects_unparseable_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kubo.pid"), "not a pid file").unwrap();
        assert!(read_pid_file(dir.path()).is_err());
    }

    #[test]
    fn parses_starttime_from_proc_stat_with_simple_comm() {
        let stat = "12345 (ipfs) S 1 12345 12345 0 -1 4194304 100 0 0 0 10 5 0 0 20 0 1 0 \
                     987654321 20971520 512";
        assert_eq!(
            parse_proc_stat_starttime(stat),
            Some("987654321".to_string())
        );
    }

    #[test]
    fn parses_starttime_from_proc_stat_with_spaces_and_parens_in_comm() {
        let stat = "12345 (my ip)fs proc) S 1 12345 12345 0 -1 4194304 100 0 0 0 10 5 0 0 20 0 1 0 \
                     555555 20971520 512";
        assert_eq!(parse_proc_stat_starttime(stat), Some("555555".to_string()));
    }

    #[test]
    fn parse_proc_stat_starttime_is_none_without_a_closing_paren() {
        assert_eq!(parse_proc_stat_starttime("garbage"), None);
    }

    #[tokio::test]
    async fn recover_orphan_with_no_pid_file_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        recover_orphan(dir.path(), dir.path()).await.unwrap();
    }

    #[tokio::test]
    async fn recover_orphan_removes_unparseable_pid_file_without_killing_anything() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kubo.pid"), "not a pid file").unwrap();
        recover_orphan(dir.path(), dir.path()).await.unwrap();
        assert_eq!(read_pid_file(dir.path()).unwrap(), None);
    }

    #[tokio::test]
    async fn recover_orphan_removes_stale_pid_file() {
        let dir = tempfile::tempdir().unwrap();
        let record = PidRecord {
            pid: 999_999_999,
            api_port: pick_free_port().unwrap(),
            started_at: "0".to_string(),
        };
        std::fs::write(
            dir.path().join("kubo.pid"),
            serde_json::to_string(&record).unwrap(),
        )
        .unwrap();
        recover_orphan(dir.path(), dir.path()).await.unwrap();
        assert_eq!(read_pid_file(dir.path()).unwrap(), None);
    }

    #[cfg(unix)]
    async fn orphan_shutdown_request(access_port_matches: bool) -> String {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        std::thread::spawn(move || child.wait());
        let server = serve("500 Internal Server Error", String::new()).await;
        let record = PidRecord {
            pid,
            api_port: server.port,
            started_at: process_start_marker(pid).unwrap(),
        };
        std::fs::write(
            dir.path().join("kubo.pid"),
            serde_json::to_string(&record).unwrap(),
        )
        .unwrap();
        let port = if access_port_matches {
            server.port
        } else {
            pick_free_port().unwrap()
        };
        write_api_access(dir.path(), &access(port)).unwrap();

        recover_orphan(dir.path(), dir.path()).await.unwrap();

        assert!(!process_alive(pid));
        let requests = server.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        requests[0].clone()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recover_orphan_sends_the_secret_only_to_the_port_it_was_issued_for() {
        let request = orphan_shutdown_request(true).await;
        assert!(request.contains("authorization: bearer abcd"), "{request}");
        let request = orphan_shutdown_request(false).await;
        assert!(!request.contains("authorization"), "{request}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recover_orphan_does_not_kill_on_start_time_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id();
        let real_started_at =
            process_start_marker(pid).expect("spawned process should have a start time");
        let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        api.set_nonblocking(true).unwrap();
        let record = PidRecord {
            pid,
            api_port: api.local_addr().unwrap().port(),
            started_at: format!("{real_started_at}-not-the-real-one"),
        };
        std::fs::write(
            dir.path().join("kubo.pid"),
            serde_json::to_string(&record).unwrap(),
        )
        .unwrap();

        recover_orphan(dir.path(), dir.path()).await.unwrap();

        assert_eq!(read_pid_file(dir.path()).unwrap(), None);
        assert!(
            process_alive(pid),
            "a start-time mismatch must not kill the process"
        );
        assert_eq!(
            api.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "a start-time mismatch must not send a shutdown to the recorded API port"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[tokio::test]
    #[ignore]
    async fn full_lifecycle_against_real_kubo() {
        let Some(bin) = test_kubo_bin() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("kubo-repo");

        let created = ensure_repo(&bin, &repo).await.unwrap();
        assert!(created);

        run(&bin, Some(&repo), &["config", "profile", "apply", "test"])
            .await
            .unwrap();

        let api = ApiAccess::generate(pick_free_port().unwrap());
        let api_port = api.port;
        let gateway_port = pick_free_port().unwrap();
        let settings = KuboSettings {
            storage_max: 123_456_789,
            provide_strategy: "pinned+mfs".to_string(),
            api: api.clone(),
            gateway: SocketAddr::from(([127, 0, 0, 1], gateway_port)),
            swarm_port: None,
            public_gateway_hosts: vec!["example.com".to_string()],
        };
        apply_config(&bin, &repo, &settings).await.unwrap();

        std::fs::write(repo.join("api"), "/ip4/127.0.0.1/tcp/1\n").unwrap();
        let mut daemon = Daemon::spawn(&bin, &repo, &api).await.unwrap();

        let health = daemon.wait_healthy(&repo, Duration::from_secs(60)).await;
        if health.is_err() {
            let _ = daemon.wait().await;
        }
        health.unwrap();

        let anonymous = crate::ipfs::IpfsClient::new(api.url());
        assert!(anonymous.peer_id().await.is_err());
        assert_eq!(
            api.client().peer_id().await.unwrap(),
            read_peer_id(&repo).unwrap()
        );

        let config_text = std::fs::read_to_string(repo.join("config")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&config_text).unwrap();
        assert_eq!(config["Datastore"]["StorageMax"], "123456789");
        assert_eq!(config["Provide"]["Strategy"], "pinned+mfs");
        assert_eq!(config["Gateway"]["NoFetch"], true);
        assert_eq!(config["Gateway"]["NoDNSLink"], true);
        assert_eq!(
            config["API"]["Authorizations"]["swing"]["AllowedPaths"],
            json!(["/api/v0"])
        );
        assert_eq!(
            config["Gateway"]["PublicGateways"]["example.com"]["Paths"],
            json!([])
        );
        assert_eq!(
            config["Addresses"]["API"],
            json!([format!("/ip4/127.0.0.1/tcp/{api_port}")])
        );
        assert_eq!(
            config["Addresses"]["Gateway"],
            json!([format!("/ip4/127.0.0.1/tcp/{gateway_port}")])
        );

        assert_eq!(
            std::fs::read_to_string(repo.join("api")).unwrap().trim(),
            format!("/ip4/127.0.0.1/tcp/{api_port}")
        );

        daemon.stop(Duration::from_secs(10)).await.unwrap();
    }

    #[cfg(unix)]
    async fn spawn_orphan(bin: &Path, dir: &Path) -> (PathBuf, String, u32) {
        let repo = dir.join("kubo-repo");
        let state_dir = dir.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();

        ensure_repo(bin, &repo).await.unwrap();
        run(bin, Some(&repo), &["config", "profile", "apply", "test"])
            .await
            .unwrap();

        let api = ApiAccess::generate(pick_free_port().unwrap());
        let settings = KuboSettings {
            storage_max: 123_456_789,
            provide_strategy: "pinned+mfs".to_string(),
            api: api.clone(),
            gateway: SocketAddr::from(([127, 0, 0, 1], pick_free_port().unwrap())),
            swarm_port: None,
            public_gateway_hosts: vec![],
        };
        apply_config(bin, &repo, &settings).await.unwrap();

        let api_url = api.url();
        let mut daemon = Daemon::spawn(bin, &repo, &api).await.unwrap();
        let pid = daemon.pid().unwrap();
        write_pid_file(&state_dir, pid, api.port).unwrap();

        daemon
            .wait_healthy(&repo, Duration::from_secs(60))
            .await
            .unwrap();
        write_api_access(&state_dir, &api).unwrap();

        // Simulate an orphan: the child must survive past this test's own process exit.
        std::mem::forget(daemon);
        // mem::forget skips tokio's orphan reaper, so stand in for it here or the killed child zombies.
        std::thread::spawn(move || unsafe {
            let mut status = 0;
            libc::waitpid(pid as libc::pid_t, &mut status, 0);
        });

        (state_dir, api_url, pid)
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn recover_orphan_shuts_down_a_leftover_daemon_via_its_api() {
        let Some(bin) = test_kubo_bin() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (state_dir, api_url, pid) = spawn_orphan(&bin, dir.path()).await;

        recover_orphan(&state_dir, dir.path()).await.unwrap();

        assert_eq!(read_pid_file(&state_dir).unwrap(), None);
        assert!(
            !process_alive(pid),
            "orphaned Kubo should have exited after the API shutdown"
        );
        let client = reqwest::Client::new();
        let result = client
            .post(format!("{api_url}/api/v0/id"))
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        assert!(
            result.is_err(),
            "Kubo API should no longer answer after recovery"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn recover_orphan_falls_back_to_signals_when_api_is_unreachable() {
        let Some(bin) = test_kubo_bin() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (state_dir, _api_url, pid) = spawn_orphan(&bin, dir.path()).await;

        // Overwrite the recorded api_port with a bogus, unlisted one so the graceful-shutdown
        // attempt fails to connect, forcing the start-time check and signal escalation path.
        let mut record = read_pid_file(&state_dir).unwrap().unwrap();
        record.api_port = pick_free_port().unwrap();
        std::fs::write(
            state_dir.join("kubo.pid"),
            serde_json::to_string(&record).unwrap(),
        )
        .unwrap();

        recover_orphan(&state_dir, dir.path()).await.unwrap();

        assert_eq!(read_pid_file(&state_dir).unwrap(), None);
        assert!(
            !process_alive(pid),
            "orphaned Kubo should have been killed via signals"
        );
    }
}
