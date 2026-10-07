use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, ERROR_UNSUPPORTED_TYPE,
    FALSE,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
};

use super::ownership::{Part, Registration, exe_from_run_command, exe_from_task_xml};
use super::process::{decode_output, output_with_timeout, run_command_with_timeout, task_listed};
use super::templates::{schtasks_xml, strip_verbatim, task_xml_bytes, tray_run_command};
use super::{GRACEFUL_STOP_TIMEOUT, InstallOptions, install_tray, windows_system_tool};
use crate::config::resolve_config_path;

const TASK_NAME: &str = "swing";
const SCHTASKS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const SCHTASKS_CHANGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "swing-tray";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn register_tray(tray: &Path, config: &Path, workdir: &Path, no_start: bool) -> Result<()> {
    let command_line = tray_run_command(tray, config)?;
    let data = wide(&command_line);
    let len = u32::try_from(data.len() * 2).context("swing-tray command line is too long")?;
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_VALUE).as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            len,
        )
    };
    if status != ERROR_SUCCESS {
        bail!("writing HKCU\\{RUN_KEY}\\{RUN_VALUE} failed (error {status})");
    }
    println!("Registered swing-tray to start at login (HKCU\\{RUN_KEY}\\{RUN_VALUE}).");
    if !no_start {
        spawn_without_handles(&command_line, workdir)
            .with_context(|| format!("starting {}", tray.display()))?;
        println!("Started swing-tray.");
    }
    Ok(())
}

// std::process::Command always inherits handles, so the tray would hold the caller's pipes open and `swing service install | ...` would never finish.
fn spawn_without_handles(command_line: &str, workdir: &Path) -> Result<()> {
    let mut command_line = wide(command_line);
    let workdir = wide(&strip_verbatim(&workdir.to_string_lossy()));
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            FALSE,
            0,
            std::ptr::null(),
            workdir.as_ptr(),
            &startup,
            &mut info,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    unsafe {
        CloseHandle(info.hProcess);
        CloseHandle(info.hThread);
    }
    Ok(())
}

pub fn unregister_tray() -> Result<()> {
    let status = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_VALUE).as_ptr(),
        )
    };
    match status {
        ERROR_SUCCESS => {
            println!("Removed swing-tray from HKCU\\{RUN_KEY}.");
            Ok(())
        }
        ERROR_FILE_NOT_FOUND => Ok(()),
        other => bail!("removing HKCU\\{RUN_KEY}\\{RUN_VALUE} failed (error {other})"),
    }
}

// swing-tray is a GUI-subsystem process; without this each schtasks call flashes a console window.
fn schtasks_command(args: &[&str]) -> Command {
    let mut cmd = Command::new(windows_system_tool("schtasks.exe"));
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args(args);
    cmd
}

fn schtasks(args: &[&str]) -> Result<Output> {
    run_command_with_timeout(schtasks_command(args), SCHTASKS_CHANGE_TIMEOUT)
}

fn current_user() -> Result<String> {
    let username = std::env::var("USERNAME").context("USERNAME is not set")?;
    if let Ok(domain) = std::env::var("USERDOMAIN")
        && !domain.is_empty()
    {
        return Ok(format!("{domain}\\{username}"));
    }
    Ok(username)
}

fn log_path(workdir: &Path) -> PathBuf {
    workdir.join("swing.log")
}

pub fn install(exe: &Path, config: &Path, workdir: &Path, opts: &InstallOptions<'_>) -> Result<()> {
    let user = current_user()?;
    let log = log_path(workdir);
    let xml = schtasks_xml(exe, config, workdir, &log, &user)?;

    // schtasks only reads the task from a file; a random name keeps other users from pre-creating it in the shared temp dir.
    let tmp_path =
        std::env::temp_dir().join(format!("swing-task-{}.xml", crate::auth::random_hex(8)));
    crate::auth::write_private_bytes(&tmp_path, &task_xml_bytes(&xml))?;

    let result = schtasks(&[
        "/Create",
        "/TN",
        TASK_NAME,
        "/XML",
        &tmp_path.to_string_lossy(),
        "/F",
    ]);
    let _ = std::fs::remove_file(&tmp_path);
    result?;
    println!("Registered swing with Task Scheduler as \"{TASK_NAME}\".");
    println!("Logs are written to {}.", log.display());

    if !opts.no_start {
        schtasks(&["/Run", "/TN", TASK_NAME])?;
        println!("Started the task.");
    } else {
        println!(
            "Service installed but not started (--no-start). Start it with `schtasks /Run /TN {TASK_NAME}`."
        );
    }
    println!("Check status with `schtasks /Query /TN {TASK_NAME}`.");
    install_tray(exe, config, workdir, opts)
}

pub fn start(_system: bool) -> Result<()> {
    schtasks(&["/Run", "/TN", TASK_NAME])?;
    println!("Started the task.");
    Ok(())
}

// schtasks exits with 1 for every error and localizes the message, so absence is only
// concluded from a successful listing of all tasks that lacks the task.
fn query_installed() -> Result<bool> {
    let query = output_with_timeout(
        &mut schtasks_command(&["/Query", "/TN", TASK_NAME]),
        SCHTASKS_TIMEOUT,
    )?;
    if query.status.success() {
        return Ok(true);
    }
    let list = output_with_timeout(
        &mut schtasks_command(&["/Query", "/FO", "CSV", "/NH"]),
        SCHTASKS_TIMEOUT,
    )?;
    if !list.status.success() {
        bail!(
            "`schtasks /Query` failed: {}",
            decode_output(&list.stderr).trim()
        );
    }
    Ok(task_listed(&decode_output(&list.stdout), TASK_NAME))
}

pub fn is_installed(_system: bool) -> Option<bool> {
    query_installed().ok()
}

fn load_stop_config() -> Option<crate::config::Config> {
    let path = resolve_config_path(None);
    if !path.exists() {
        println!(
            "Warning: could not find the config file (swing.toml) at {} to stop swing through its dashboard; set SWING_CONFIG or run this from the directory containing swing.toml. Falling back to `schtasks /End`.",
            path.display()
        );
        return None;
    }
    match crate::config::Config::load(Some(&path)) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            println!(
                "Warning: could not read the config file {} to stop swing through its dashboard ({e:#}). Falling back to `schtasks /End`.",
                path.display()
            );
            None
        }
    }
}

async fn stop_gracefully() -> bool {
    let Some(cfg) = load_stop_config() else {
        return false;
    };
    match crate::stop::run(&cfg, false, GRACEFUL_STOP_TIMEOUT).await {
        Ok(()) => true,
        Err(e) => {
            println!("Warning: graceful stop failed ({e:#}); falling back to `schtasks /End`.");
            false
        }
    }
}

pub async fn stop(_system: bool) -> Result<()> {
    if stop_gracefully().await {
        return Ok(());
    }
    schtasks(&["/End", "/TN", TASK_NAME])?;
    Ok(())
}

// RRF_RT_REG_EXPAND_SZ without RRF_NOEXPAND is rejected as an invalid parameter; RRF_RT_REG_SZ alone still accepts an expanded REG_EXPAND_SZ.
fn read_tray_value() -> Result<Option<String>> {
    let key = wide(RUN_KEY);
    let value = wide(RUN_VALUE);
    let mut buf: Vec<u16> = Vec::new();
    loop {
        let mut len =
            u32::try_from(buf.len() * 2).context("swing-tray command line is too long")?;
        let data = if buf.is_empty() {
            std::ptr::null_mut()
        } else {
            buf.as_mut_ptr().cast()
        };
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                data,
                &mut len,
            )
        };
        match status {
            ERROR_SUCCESS if !buf.is_empty() => {
                buf.truncate((len as usize) / 2);
                while buf.last() == Some(&0) {
                    buf.pop();
                }
                return Ok(Some(String::from_utf16_lossy(&buf)));
            }
            ERROR_SUCCESS | ERROR_MORE_DATA => buf = vec![0u16; (len as usize).div_ceil(2).max(1)],
            ERROR_FILE_NOT_FOUND => return Ok(None),
            ERROR_UNSUPPORTED_TYPE => return Ok(Some(String::new())),
            other => bail!("reading HKCU\\{RUN_KEY}\\{RUN_VALUE} failed (error {other})"),
        }
    }
}

fn decode_task_xml(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    decode_output(bytes)
}

pub fn registrations(_system: bool) -> Result<Vec<Registration>> {
    let mut out = Vec::new();
    if query_installed().context("checking whether swing is registered with Task Scheduler")? {
        let xml = output_with_timeout(
            &mut schtasks_command(&["/Query", "/TN", TASK_NAME, "/XML"]),
            SCHTASKS_TIMEOUT,
        )?;
        if !xml.status.success() {
            bail!(
                "`schtasks /Query /TN {TASK_NAME} /XML` failed: {}",
                decode_output(&xml.stderr).trim()
            );
        }
        out.push(Registration {
            part: Part::Service,
            what: format!("The Task Scheduler task \"{TASK_NAME}\""),
            exe: exe_from_task_xml(&decode_task_xml(&xml.stdout)),
        });
    }
    if let Some(command_line) = read_tray_value()? {
        out.push(Registration {
            part: Part::Tray,
            what: format!("The login item HKCU\\{RUN_KEY}\\{RUN_VALUE}"),
            exe: exe_from_run_command(&command_line),
        });
    }
    Ok(out)
}

pub async fn uninstall_parts(_system: bool, service: bool, tray: bool) -> Result<()> {
    if tray {
        unregister_tray()?;
    }
    if !service {
        return Ok(());
    }
    stop_gracefully().await;
    let _ = output_with_timeout(
        &mut schtasks_command(&["/End", "/TN", TASK_NAME]),
        SCHTASKS_TIMEOUT,
    );
    schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])?;
    println!("Uninstalled swing from Task Scheduler.");
    Ok(())
}

pub fn status(_system: bool) -> Result<()> {
    if !query_installed().context("checking whether swing is registered with Task Scheduler")? {
        println!("not installed");
        return Ok(());
    }
    let out = output_with_timeout(
        &mut schtasks_command(&["/Query", "/TN", TASK_NAME, "/FO", "LIST", "/V"]),
        SCHTASKS_TIMEOUT,
    )?;
    if !out.status.success() {
        bail!(
            "`schtasks /Query /TN {TASK_NAME} /V` failed: {}",
            decode_output(&out.stderr).trim()
        );
    }
    print!("{}", decode_output(&out.stdout));
    Ok(())
}
