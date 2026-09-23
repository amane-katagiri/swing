use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

const FORCE_EXIT_GRACE_PERIOD: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Stop,
    Restart,
}

#[derive(Clone)]
pub struct ExitRequest {
    token: CancellationToken,
    restart: Arc<AtomicBool>,
}

impl ExitRequest {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            restart: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn stop(&self) {
        self.token.cancel();
    }

    pub fn restart(&self) {
        self.restart.store(true, Ordering::SeqCst);
        self.token.cancel();
    }

    pub fn restart_requested(&self) -> bool {
        self.restart.load(Ordering::SeqCst)
    }

    pub fn exit(&self) -> Exit {
        if self.restart_requested() {
            Exit::Restart
        } else {
            Exit::Stop
        }
    }
}

pub fn cancel_on_signal() -> Result<CancellationToken> {
    let token = CancellationToken::new();
    spawn_watcher(token.clone())?;
    Ok(token)
}

#[cfg(unix)]
fn spawn_watcher(token: CancellationToken) -> Result<()> {
    use anyhow::Context;
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).context("registering SIGTERM handler")?;
    tokio::spawn(async move {
        let signal = tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        };
        on_signal(token, signal).await;
    });
    Ok(())
}

#[cfg(not(unix))]
fn spawn_watcher(token: CancellationToken) -> Result<()> {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        on_signal(token, "ctrl-c").await;
    });
    Ok(())
}

/// Task Scheduler's `/End` only terminates `conhost.exe --headless`, the task's
/// own process, and leaves swing running as its orphaned child.
#[cfg(windows)]
pub fn cancel_when_parent_exits(token: CancellationToken) -> Result<()> {
    let parent = parent_process::open()?;
    let runtime = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        parent.wait();
        runtime.spawn(on_signal(token, "parent exited"));
    });
    Ok(())
}

#[cfg(windows)]
mod parent_process {
    use anyhow::{Result, bail};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    pub struct Parent(HANDLE);

    unsafe impl Send for Parent {}

    impl Parent {
        pub fn wait(&self) {
            unsafe {
                WaitForSingleObject(self.0, INFINITE);
            }
        }
    }

    impl Drop for Parent {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn open() -> Result<Parent> {
        let pid = parent_pid()?;
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            bail!(
                "opening parent process {pid}: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Parent(handle))
    }

    fn parent_pid() -> Result<u32> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                bail!(
                    "taking a process snapshot: {}",
                    std::io::Error::last_os_error()
                );
            }
            let me = GetCurrentProcessId();
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = None;
            let mut ok = Process32FirstW(snapshot, &mut entry);
            while ok != 0 {
                if entry.th32ProcessID == me {
                    found = Some(entry.th32ParentProcessID);
                    break;
                }
                ok = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            match found {
                Some(pid) => Ok(pid),
                None => bail!("own process {me} not found in the process snapshot"),
            }
        }
    }
}

async fn on_signal(token: CancellationToken, signal: &'static str) {
    info!(signal, "shutdown requested");
    token.cancel();
    tokio::time::sleep(FORCE_EXIT_GRACE_PERIOD).await;
    error!(
        grace_period = ?FORCE_EXIT_GRACE_PERIOD,
        "graceful shutdown did not finish within the grace period; forcing exit"
    );
    std::process::exit(1);
}
