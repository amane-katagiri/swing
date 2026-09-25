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

pub const KUBO_VERSION: &str = "0.43.1";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KuboSettings {
    pub storage_max: u64,
    pub provide_strategy: String,
    pub api_port: u16,
    pub gateway: SocketAddr,
    pub swarm_port: Option<u16>,
    pub public_gateway_hosts: Vec<String>,
}

pub fn api_url_from_repo(repo: &Path) -> Result<String> {
    let path = repo.join("api");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
            "Kubo is not running: {} does not exist (start `swing up`, or set [kubo].managed = false and [ipfs].api to use an external Kubo)",
            path.display()
        ),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    multiaddr_to_http_url(text.trim())
        .with_context(|| format!("parsing Kubo API address in {}", path.display()))
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

fn command_error(
    program: &str,
    args: &[&str],
    status: std::process::ExitStatus,
    stderr: &str,
) -> anyhow::Error {
    anyhow::anyhow!(
        "`{program} {}` failed ({status}): {}",
        args.join(" "),
        stderr.trim()
    )
}

async fn run_ipfs(bin: &Path, repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(bin)
        .args(args)
        .env("IPFS_PATH", repo)
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("running `{} {}`", bin.display(), args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(command_error(
            &bin.display().to_string(),
            args,
            output.status,
            &stderr,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub async fn version(bin: &Path) -> Result<String> {
    let output = Command::new(bin)
        .args(["version", "--number"])
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("running `{} version --number`", bin.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(command_error(
            &bin.display().to_string(),
            &["version", "--number"],
            output.status,
            &stderr,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn ensure_repo(bin: &Path, repo: &Path) -> Result<bool> {
    std::fs::create_dir_all(repo)
        .with_context(|| format!("creating Kubo repo dir {}", repo.display()))?;
    if repo.join("config").is_file() {
        return Ok(false);
    }
    run_ipfs(bin, repo, &["init"]).await?;
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
    run_ipfs(bin, repo, &["config", key, value]).await?;
    Ok(())
}

async fn set_config_json(
    bin: &Path,
    repo: &Path,
    key: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let value = serde_json::to_string(value).with_context(|| format!("serializing {key}"))?;
    run_ipfs(bin, repo, &["config", "--json", key, &value]).await?;
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
        "Addresses.API",
        &json!([format!("/ip4/127.0.0.1/tcp/{}", s.api_port)]),
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

    Ok(())
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

// comm can itself contain spaces and ')', so the field list is only unambiguous after the last ')'.
#[cfg(any(test, target_os = "linux"))]
fn parse_proc_stat_starttime(stat: &str) -> Option<String> {
    let rparen = stat.rfind(')')?;
    let rest = stat.get(rparen + 1..)?;
    rest.split_whitespace().nth(19).map(|s| s.to_string())
}

#[cfg(target_os = "linux")]
fn process_start_marker(pid: u32) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_proc_stat_starttime(&text)
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

// Only a real Kubo answers this; an unrelated process reusing the port will fail to connect
// or time out, so a response here is not itself proof of identity but a successful exit is.
async fn attempt_graceful_shutdown(api_port: u16, pid: u32) -> bool {
    let url = format!("http://127.0.0.1:{api_port}/api/v0/shutdown");
    let responded = reqwest::Client::new()
        .post(&url)
        .timeout(ORPHAN_SHUTDOWN_RPC_TIMEOUT)
        .send()
        .await
        .is_ok();
    if !responded {
        return false;
    }
    wait_for_exit(pid, ORPHAN_SHUTDOWN_GRACE).await
}

#[cfg(unix)]
async fn terminate_process(pid: u32) -> Result<()> {
    let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err).context("sending SIGTERM to orphaned Kubo");
        }
    }
    if wait_for_exit(pid, ORPHAN_SIGTERM_GRACE).await {
        return Ok(());
    }
    tracing::warn!(
        pid,
        "orphaned Kubo did not exit after SIGTERM, sending SIGKILL"
    );
    let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err).context("sending SIGKILL to orphaned Kubo");
        }
    }
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

    if attempt_graceful_shutdown(record.api_port, record.pid).await {
        tracing::info!(
            pid = record.pid,
            repo = %repo.display(),
            "orphaned Kubo shut down gracefully via its API"
        );
        remove_pid_file(state_dir)?;
        return Ok(());
    }

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
                        && line.contains("lock")
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
    saw_repo_lock: Arc<AtomicBool>,
    // Held only for its Drop: closing the Job Object handle kills the child.
    #[cfg(windows)]
    #[allow(dead_code)]
    job: windows_job::JobHandle,
}

impl Daemon {
    pub async fn spawn(bin: &Path, repo: &Path, api_url: String) -> Result<Daemon> {
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
            api_url,
            saw_repo_lock,
            #[cfg(windows)]
            job,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
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
        let shutdown_url = format!("{}/api/v0/shutdown", self.api_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        match client
            .post(&shutdown_url)
            .timeout(SHUTDOWN_RPC_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!("kubo RPC shutdown requested");
            }
            Ok(resp) => {
                tracing::debug!(status = %resp.status(), "kubo RPC shutdown request answered with an error (daemon may be exiting anyway)");
            }
            Err(e) => {
                tracing::debug!(error = %e, "kubo RPC shutdown request did not complete (daemon may have closed the connection while exiting)");
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
            if let Some(pid) = self.child.id() {
                let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
                if ret != 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!("failed to send SIGTERM to Kubo (pid {pid}): {err}");
                    }
                }
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

pub async fn wait_healthy(api_url: &str, timeout: Duration) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/v0/id", api_url.trim_end_matches('/'));
    let healthy = wait_until(timeout, Duration::from_secs(1), || async {
        matches!(
            client.post(&url).timeout(Duration::from_secs(5)).send().await,
            Ok(resp) if resp.status().is_success()
        )
    })
    .await;
    if healthy {
        Ok(())
    } else {
        bail!("Kubo did not become healthy within {timeout:?} (POST {url})")
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

    #[test]
    fn missing_api_file_says_kubo_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let err = api_url_from_repo(dir.path()).unwrap_err().to_string();
        assert!(err.contains("Kubo is not running"), "{err}");
        std::fs::write(dir.path().join("api"), "/ip4/127.0.0.1/tcp/41234\n").unwrap();
        assert_eq!(
            api_url_from_repo(dir.path()).unwrap(),
            "http://127.0.0.1:41234"
        );
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
    fn locate_binary_errors_on_missing_explicit_path() {
        let missing = PathBuf::from("/nonexistent/path/to/ipfs-binary-that-does-not-exist");
        let err = locate_binary(Some(&missing)).unwrap_err().to_string();
        assert!(err.contains("Kubo binary not found"), "{err}");
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
        let record = PidRecord {
            pid,
            api_port: pick_free_port().unwrap(),
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

        run_ipfs(&bin, &repo, &["config", "profile", "apply", "test"])
            .await
            .unwrap();

        let api_port = pick_free_port().unwrap();
        let gateway_port = pick_free_port().unwrap();
        let settings = KuboSettings {
            storage_max: 123_456_789,
            provide_strategy: "pinned+mfs".to_string(),
            api_port,
            gateway: SocketAddr::from(([127, 0, 0, 1], gateway_port)),
            swarm_port: None,
            public_gateway_hosts: vec!["example.com".to_string()],
        };
        apply_config(&bin, &repo, &settings).await.unwrap();

        let api_url = format!("http://127.0.0.1:{api_port}");
        let mut daemon = Daemon::spawn(&bin, &repo, api_url.clone()).await.unwrap();

        let health = wait_healthy(&api_url, Duration::from_secs(60)).await;
        if health.is_err() {
            let _ = daemon.wait().await;
        }
        health.unwrap();

        let config_text = std::fs::read_to_string(repo.join("config")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&config_text).unwrap();
        assert_eq!(config["Datastore"]["StorageMax"], "123456789");
        assert_eq!(config["Provide"]["Strategy"], "pinned+mfs");
        assert_eq!(config["Gateway"]["NoFetch"], true);
        assert_eq!(config["Gateway"]["NoDNSLink"], true);
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

        let reported_api = api_url_from_repo(&repo).unwrap();
        assert_eq!(reported_api, format!("http://127.0.0.1:{api_port}"));

        daemon.stop(Duration::from_secs(10)).await.unwrap();
    }

    /// Starts a real Kubo daemon, writes its pid file with the daemon's real api_port, then
    /// forgets the `Daemon` handle and reaps it manually to simulate an orphan left by a killed
    /// `swing up`. Returns the state dir, the daemon's real API URL, and its pid.
    #[cfg(unix)]
    async fn spawn_orphan(bin: &Path, dir: &Path) -> (PathBuf, String, u32) {
        let repo = dir.join("kubo-repo");
        let state_dir = dir.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();

        ensure_repo(bin, &repo).await.unwrap();
        run_ipfs(bin, &repo, &["config", "profile", "apply", "test"])
            .await
            .unwrap();

        let api_port = pick_free_port().unwrap();
        let settings = KuboSettings {
            storage_max: 123_456_789,
            provide_strategy: "pinned+mfs".to_string(),
            api_port,
            gateway: SocketAddr::from(([127, 0, 0, 1], pick_free_port().unwrap())),
            swarm_port: None,
            public_gateway_hosts: vec![],
        };
        apply_config(bin, &repo, &settings).await.unwrap();

        let api_url = format!("http://127.0.0.1:{api_port}");
        let daemon = Daemon::spawn(bin, &repo, api_url.clone()).await.unwrap();
        let pid = daemon.pid().unwrap();
        write_pid_file(&state_dir, pid, api_port).unwrap();

        wait_healthy(&api_url, Duration::from_secs(60))
            .await
            .unwrap();

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
