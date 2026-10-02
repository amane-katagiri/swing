use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};

use super::ownership::{Part, Registration, exe_from_systemd_unit};
use super::process::run_command;
use super::templates::{SystemdScope, systemd_unit};
use super::{InstallOptions, ensure_parent_dir, write_service_file};

const SYSTEM_UNIT_PATH: &str = "/etc/systemd/system/swing.service";

fn unit_path(system: bool) -> Result<PathBuf> {
    if system {
        return Ok(PathBuf::from(SYSTEM_UNIT_PATH));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("cannot determine XDG_CONFIG_HOME or HOME")?;
    Ok(base.join("systemd").join("user").join("swing.service"))
}

fn systemctl_command(system: bool, args: &[&str]) -> Command {
    let mut cmd = Command::new("systemctl");
    if !system {
        cmd.arg("--user");
    }
    cmd.args(args);
    cmd
}

fn systemctl(system: bool, args: &[&str]) -> Result<Output> {
    run_command(systemctl_command(system, args))
}

#[derive(Debug, PartialEq, Eq)]
enum ServiceUser {
    Name(String),
    Uid(u32),
}

fn pick_service_user(
    run_as: Option<&str>,
    sudo_uid: Option<&str>,
    uid: u32,
) -> Result<ServiceUser> {
    if let Some(run_as) = run_as {
        return Ok(match run_as.parse() {
            Ok(id) => ServiceUser::Uid(id),
            Err(_) => ServiceUser::Name(run_as.to_owned()),
        });
    }
    if let Some(sudo_uid) = sudo_uid.filter(|s| !s.is_empty()) {
        let id: u32 = sudo_uid
            .parse()
            .with_context(|| format!("SUDO_UID is not a user id: {sudo_uid:?}"))?;
        if id != 0 {
            return Ok(ServiceUser::Uid(id));
        }
    }
    if uid != 0 {
        return Ok(ServiceUser::Uid(uid));
    }
    bail!(
        "refusing to register a system service that runs swing as root: run `sudo swing service install --system` from the account that should run swing, or pass --run-as <user>"
    )
}

fn passwd_name(user: &ServiceUser) -> Result<String> {
    let entry = match user {
        ServiceUser::Name(name) => {
            let c_name = CString::new(name.as_str())
                .with_context(|| format!("invalid user name {name:?}"))?;
            unsafe { libc::getpwnam(c_name.as_ptr()) }
        }
        ServiceUser::Uid(uid) => unsafe { libc::getpwuid(*uid) },
    };
    if entry.is_null() {
        match user {
            ServiceUser::Name(name) => bail!("no such user: {name}"),
            ServiceUser::Uid(uid) => bail!("no user with uid {uid}"),
        }
    }
    let name = unsafe { CStr::from_ptr((*entry).pw_name) };
    name.to_str()
        .map(str::to_owned)
        .context("the user name is not valid UTF-8")
}

fn service_user(run_as: Option<&str>) -> Result<String> {
    let sudo_uid = std::env::var("SUDO_UID").ok();
    let uid = unsafe { libc::getuid() };
    passwd_name(&pick_service_user(run_as, sudo_uid.as_deref(), uid)?)
}

pub fn start(system: bool) -> Result<()> {
    systemctl(system, &["start", "swing"])?;
    println!("Started swing with systemd.");
    Ok(())
}

pub fn is_installed(system: bool) -> Option<bool> {
    Some(unit_path(system).is_ok_and(|p| p.exists()))
}

fn writable_paths(config: &Path) -> Vec<PathBuf> {
    match crate::config::Config::load(Some(config)) {
        Ok(config) => [config.agent.state_dir, config.kubo.repo]
            .into_iter()
            .filter(|p| p.is_absolute())
            .collect(),
        Err(e) => {
            println!(
                "Warning: could not read {} ({e:#}); the unit only lets swing write under the config file's directory, so keep [agent].state_dir and [kubo].repo there.",
                config.display()
            );
            Vec::new()
        }
    }
}

pub fn install(exe: &Path, config: &Path, workdir: &Path, opts: &InstallOptions<'_>) -> Result<()> {
    let system = opts.system;
    let user = if system {
        Some(service_user(opts.run_as)?)
    } else {
        None
    };
    let writable = if system {
        writable_paths(config)
    } else {
        Vec::new()
    };
    let scope = match &user {
        Some(user) => SystemdScope::System {
            user,
            writable: &writable,
        },
        None => SystemdScope::User,
    };
    let unit = systemd_unit(exe, config, workdir, &scope)?;
    let path = unit_path(system)?;
    ensure_parent_dir(&path)?;
    write_service_file(&path, unit)?;
    println!("Wrote systemd unit to {}.", path.display());
    if let Some(user) = &user {
        println!("The service runs as user {user}.");
    }

    systemctl(system, &["daemon-reload"])?;
    if opts.no_start {
        systemctl(system, &["enable", "swing"])?;
    } else {
        systemctl(system, &["enable", "--now", "swing"])?;
    }
    println!(
        "Registered swing with systemd ({}).",
        if system { "system" } else { "user" }
    );

    if !system {
        let uid = unsafe { libc::getuid() }.to_string();
        let linger = Command::new("loginctl")
            .args(["enable-linger", &uid])
            .output();
        match linger {
            Ok(out) if out.status.success() => {
                println!("Enabled linger so swing keeps running while logged out.");
            }
            _ => {
                println!(
                    "Warning: could not run `loginctl enable-linger`. Run it yourself so swing keeps running while you are logged out."
                );
            }
        }
    }

    if opts.no_start {
        println!(
            "Service installed but not started (--no-start). Start it with `systemctl {}start swing`.",
            if system { "" } else { "--user " }
        );
    } else {
        println!(
            "Follow logs with `journalctl {}-u swing -f`.",
            if system { "" } else { "--user " }
        );
    }
    Ok(())
}

pub async fn stop(system: bool) -> Result<()> {
    systemctl(system, &["stop", "swing"])?;
    println!("Stopped swing.");
    Ok(())
}

pub fn registrations(system: bool) -> Result<Vec<Registration>> {
    let path = unit_path(system)?;
    let unit = match std::fs::read_to_string(&path) {
        Ok(unit) => unit,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("reading {}", path.display())));
        }
    };
    Ok(vec![Registration {
        part: Part::Service,
        what: format!("The systemd unit {}", path.display()),
        exe: exe_from_systemd_unit(&unit),
    }])
}

pub async fn uninstall_parts(system: bool, service: bool, _tray: bool) -> Result<()> {
    if !service {
        return Ok(());
    }
    let path = unit_path(system)?;
    if let Ok(out) = systemctl_command(system, &["disable", "--now", "swing"]).output()
        && !out.status.success()
    {
        println!(
            "Note: `systemctl disable --now swing` did not succeed (it may not have been loaded)."
        );
    }
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        println!("Removed {}.", path.display());
    } else {
        println!("No unit file found at {}.", path.display());
    }
    systemctl(system, &["daemon-reload"])?;
    println!("Uninstalled swing from systemd.");
    Ok(())
}

pub fn status(system: bool) -> Result<()> {
    let path = unit_path(system)?;
    if !path.exists() {
        println!("not installed");
        return Ok(());
    }
    let _ = systemctl_command(system, &["status", "swing", "--no-pager"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_user_prefers_run_as_then_the_sudo_caller_then_the_current_user() {
        assert_eq!(
            pick_service_user(Some("swing"), Some("1000"), 0).unwrap(),
            ServiceUser::Name("swing".into())
        );
        assert_eq!(
            pick_service_user(Some("1001"), None, 0).unwrap(),
            ServiceUser::Uid(1001)
        );
        assert_eq!(
            pick_service_user(Some("root"), None, 0).unwrap(),
            ServiceUser::Name("root".into())
        );
        assert_eq!(
            pick_service_user(None, Some("1000"), 0).unwrap(),
            ServiceUser::Uid(1000)
        );
        assert_eq!(
            pick_service_user(None, None, 1000).unwrap(),
            ServiceUser::Uid(1000)
        );
    }

    #[test]
    fn service_user_refuses_root_without_run_as() {
        for sudo_uid in [None, Some(""), Some("0")] {
            let err = pick_service_user(None, sudo_uid, 0).unwrap_err();
            assert!(err.to_string().contains("--run-as"), "{err:#}");
        }
        assert!(pick_service_user(None, Some("abc"), 0).is_err());
    }

    #[test]
    fn passwd_name_resolves_existing_users_only() {
        assert_eq!(passwd_name(&ServiceUser::Uid(0)).unwrap(), "root");
        assert_eq!(
            passwd_name(&ServiceUser::Name("root".into())).unwrap(),
            "root"
        );
        assert!(passwd_name(&ServiceUser::Name("swing-no-such-user-x".into())).is_err());
        assert!(passwd_name(&ServiceUser::Name("a\0b".into())).is_err());
    }
}
