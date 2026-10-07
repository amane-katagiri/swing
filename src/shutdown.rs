use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

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

#[derive(Clone)]
pub struct SignalWatch {
    token: CancellationToken,
    grace: Duration,
    started: Arc<AtomicBool>,
}

impl SignalWatch {
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    fn request(&self, reason: &'static str) -> bool {
        if self.started.swap(true, Ordering::SeqCst) {
            return false;
        }
        info!(reason, grace_period = ?self.grace, "shutdown requested");
        self.token.cancel();
        let grace = self.grace;
        // A std thread, unlike a tokio task, survives Runtime::shutdown_timeout, so the deadline also covers it.
        std::thread::spawn(move || {
            std::thread::sleep(grace);
            error!(
                grace_period = ?grace,
                "graceful shutdown did not finish within the grace period; forcing exit"
            );
            std::process::exit(1);
        });
        true
    }

    fn on_signal(&self, signal: &'static str) {
        if !self.request(signal) {
            error!(
                signal,
                "received another signal during graceful shutdown; exiting immediately"
            );
            std::process::exit(1);
        }
    }

    // Task Scheduler's `/End` only kills `conhost.exe --headless`, leaving swing running as its orphan.
    #[cfg(windows)]
    pub fn cancel_when_parent_exits(&self) -> Result<()> {
        let parent = parent_process::open()?;
        let watch = self.clone();
        std::thread::spawn(move || {
            parent.wait();
            watch.request("parent exited");
        });
        Ok(())
    }
}

pub fn cancel_on_signal(grace: Duration) -> Result<SignalWatch> {
    let watch = SignalWatch {
        token: CancellationToken::new(),
        grace,
        started: Arc::new(AtomicBool::new(false)),
    };
    spawn_watcher(watch.clone())?;
    Ok(watch)
}

#[cfg(unix)]
fn spawn_watcher(watch: SignalWatch) -> Result<()> {
    use anyhow::Context;
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigint = signal(SignalKind::interrupt()).context("registering SIGINT handler")?;
    let mut sigterm = signal(SignalKind::terminate()).context("registering SIGTERM handler")?;
    tokio::spawn(async move {
        loop {
            let signal = tokio::select! {
                _ = sigint.recv() => "SIGINT",
                _ = sigterm.recv() => "SIGTERM",
            };
            watch.on_signal(signal);
        }
    });
    Ok(())
}

#[cfg(windows)]
fn spawn_watcher(watch: SignalWatch) -> Result<()> {
    use anyhow::Context;

    let mut ctrl_c = tokio::signal::windows::ctrl_c().context("registering the ctrl-c handler")?;
    tokio::spawn(async move {
        while ctrl_c.recv().await.is_some() {
            watch.on_signal("ctrl-c");
        }
    });
    Ok(())
}

#[cfg(windows)]
mod parent_process {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    use anyhow::{Result, bail};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, INFINITE, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    pub struct Parent(OwnedHandle);

    impl Parent {
        pub fn wait(&self) {
            unsafe {
                WaitForSingleObject(self.0.as_raw_handle(), INFINITE);
            }
        }
    }

    pub fn open() -> Result<Parent> {
        let pid = parent_pid()?;
        match crate::proc::open_process(pid, PROCESS_SYNCHRONIZE) {
            Ok(handle) => Ok(Parent(handle)),
            Err(e) => bail!("opening parent process {pid}: {e}"),
        }
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
            let snapshot = OwnedHandle::from_raw_handle(snapshot);
            let me = GetCurrentProcessId();
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = None;
            let mut ok = Process32FirstW(snapshot.as_raw_handle(), &mut entry);
            while ok != 0 {
                if entry.th32ProcessID == me {
                    found = Some(entry.th32ParentProcessID);
                    break;
                }
                ok = Process32NextW(snapshot.as_raw_handle(), &mut entry);
            }
            match found {
                Some(pid) => Ok(pid),
                None => bail!("own process {me} not found in the process snapshot"),
            }
        }
    }
}
