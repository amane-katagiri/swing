use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::ipfs::IpfsClient;

use super::access::{ApiAccess, read_peer_id};
use super::config::multiaddr_to_http_url;
use super::remove_if_present;

const SHUTDOWN_RPC_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const SIGTERM_GRACE: Duration = Duration::from_secs(10);
#[cfg(unix)]
const ESCALATION: Duration = SIGTERM_GRACE;
#[cfg(not(unix))]
const ESCALATION: Duration = Duration::ZERO;

pub const fn daemon_stop_budget(grace: Duration) -> Duration {
    SHUTDOWN_RPC_TIMEOUT
        .saturating_add(grace)
        .saturating_add(ESCALATION)
}

pub(super) fn is_repo_lock_error(line: &str) -> bool {
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
                && let Err(err) = crate::proc::send_signal(pid, libc::SIGTERM)
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
