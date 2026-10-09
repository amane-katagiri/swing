use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};

use super::ownership::{Part, Registration, exe_from_launchd_plist};
use super::process::run_command;
use super::templates::{launchd_plist, launchd_tray_plist};
use super::{
    InstallOptions, MACOS_LABEL, MACOS_TRAY_LABEL, ensure_parent_dir, install_tray,
    write_service_file,
};

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("cannot determine HOME")
}

fn agent_plist_path(label: &str) -> Result<PathBuf> {
    Ok(home()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{label}.plist")))
}

fn plist_path() -> Result<PathBuf> {
    agent_plist_path(MACOS_LABEL)
}

fn log_path() -> Result<PathBuf> {
    Ok(home()?.join("Library").join("Logs").join("swing.log"))
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

fn domain() -> String {
    format!("gui/{}", uid())
}

fn service_target(label: &str) -> String {
    format!("{}/{label}", domain())
}

fn launchctl_command(args: &[&str]) -> Command {
    let mut cmd = Command::new("launchctl");
    cmd.args(args);
    cmd
}

fn launchctl(args: &[&str]) -> Result<Output> {
    run_command(launchctl_command(args))
}

fn launchctl_succeeds(args: &[&str]) -> bool {
    launchctl_command(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn bootstrap(plist: &Path) -> Result<()> {
    launchctl(&["bootstrap", &domain(), &plist.to_string_lossy()])?;
    Ok(())
}

fn bootout(label: &str) {
    launchctl_succeeds(&["bootout", &service_target(label)]);
}

fn is_loaded() -> bool {
    launchctl_succeeds(&["print", &service_target(MACOS_LABEL)])
}

pub fn install(exe: &Path, config: &Path, workdir: &Path, opts: &InstallOptions<'_>) -> Result<()> {
    let path = plist_path()?;
    let log = log_path()?;
    let plist = launchd_plist(exe, config, workdir, &log)?;
    ensure_parent_dir(&log)?;
    ensure_parent_dir(&path)?;

    write_service_file(&path, plist)?;
    println!("Wrote launchd agent to {}.", path.display());

    if is_loaded() {
        bootout(MACOS_LABEL);
    }

    if !opts.no_start {
        bootstrap(&path)?;
        println!("Loaded swing with launchd.");
    } else {
        println!(
            "Service installed but not started (--no-start). Load it with `launchctl bootstrap {} {}`.",
            domain(),
            path.display()
        );
    }
    println!("Logs are written to {}.", log.display());
    install_tray(exe, config, workdir, opts)
}

pub fn register_tray(tray: &Path, config: &Path, workdir: &Path, no_start: bool) -> Result<()> {
    let path = agent_plist_path(MACOS_TRAY_LABEL)?;
    let plist = launchd_tray_plist(tray, config, workdir)?;
    write_service_file(&path, plist)?;
    bootout(MACOS_TRAY_LABEL);
    println!(
        "Registered swing-tray to start at login ({}).",
        path.display()
    );
    if !no_start {
        bootstrap(&path)?;
        println!("Started swing-tray.");
    }
    Ok(())
}

pub fn unregister_tray() -> Result<()> {
    let path = agent_plist_path(MACOS_TRAY_LABEL)?;
    if !path.exists() {
        return Ok(());
    }
    bootout(MACOS_TRAY_LABEL);
    std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    println!("Removed {}.", path.display());
    Ok(())
}

pub fn start(_system: bool) -> Result<()> {
    if is_loaded() {
        launchctl(&["kickstart", &service_target(MACOS_LABEL)])?;
    } else {
        let path = plist_path()?;
        if !path.exists() {
            bail!("swing is not registered as a service; run `swing service install` first");
        }
        bootstrap(&path)?;
    }
    println!("Started swing with launchd.");
    Ok(())
}

pub fn is_installed(_system: bool) -> Option<bool> {
    Some(plist_path().is_ok_and(|p| p.exists()))
}

pub async fn stop(_system: bool) -> Result<()> {
    launchctl(&["kill", "SIGTERM", &service_target(MACOS_LABEL)])?;
    println!(
        "Sent SIGTERM to swing via launchctl. It stays stopped until the next login; start it again with `launchctl kickstart -k gui/<uid>/{MACOS_LABEL}`."
    );
    Ok(())
}

pub fn registrations(_system: bool) -> Result<Vec<Registration>> {
    let mut out = Vec::new();
    for (part, label) in [(Part::Service, MACOS_LABEL), (Part::Tray, MACOS_TRAY_LABEL)] {
        let path = agent_plist_path(label)?;
        let plist = match std::fs::read(&path) {
            Ok(plist) => plist,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(anyhow::Error::new(e).context(format!("reading {}", path.display())));
            }
        };
        out.push(Registration {
            part,
            what: format!("The launch agent {}", path.display()),
            exe: std::str::from_utf8(&plist)
                .ok()
                .and_then(exe_from_launchd_plist),
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
    let path = plist_path()?;
    bootout(MACOS_LABEL);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        println!("Removed {}.", path.display());
    } else {
        println!("No launch agent found at {}.", path.display());
    }
    println!("Uninstalled swing from launchd.");
    Ok(())
}

pub fn status(_system: bool) -> Result<()> {
    let path = plist_path()?;
    if !path.exists() {
        println!("not installed");
        return Ok(());
    }
    let _ = launchctl_command(&["print", &service_target(MACOS_LABEL)])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    Ok(())
}
