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

struct Account {
    name: String,
    uid: u32,
}

fn passwd_entry(user: &ServiceUser) -> Result<Account> {
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
    let (name, uid) = unsafe { (CStr::from_ptr((*entry).pw_name), (*entry).pw_uid) };
    let name = name
        .to_str()
        .map(str::to_owned)
        .context("the user name is not valid UTF-8")?;
    Ok(Account { name, uid })
}

fn service_user(run_as: Option<&str>, allow_root: bool) -> Result<Account> {
    let sudo_uid = std::env::var("SUDO_UID").ok();
    let uid = unsafe { libc::getuid() };
    let account = passwd_entry(&pick_service_user(run_as, sudo_uid.as_deref(), uid)?)?;
    if account.uid == 0 && !allow_root {
        bail!(
            "refusing to register a system service that runs swing as {} (uid 0); pass --allow-root if that is intended",
            account.name
        );
    }
    Ok(account)
}

fn is_private_group(owner: u32, gid: u32) -> bool {
    unsafe {
        let user = libc::getpwuid(owner);
        let group = libc::getgrgid(gid);
        if user.is_null() || group.is_null() {
            return false;
        }
        let name = CStr::from_ptr((*user).pw_name);
        if CStr::from_ptr((*group).gr_name) != name {
            return false;
        }
        let mut member = (*group).gr_mem;
        while !(*member).is_null() {
            if CStr::from_ptr(*member) != name {
                return false;
            }
            member = member.add(1);
        }
        true
    }
}

fn entry_problem(
    owner: u32,
    mode: u32,
    trusted: u32,
    ancestor: bool,
    private_group: impl FnOnce() -> bool,
) -> Option<String> {
    let kind = mode & libc::S_IFMT;
    if ancestor && kind == libc::S_IFDIR && mode & libc::S_ISVTX != 0 && owner == 0 {
        return None;
    }
    if owner != 0 && owner != trusted {
        return Some(format!("is owned by uid {owner}"));
    }
    let loose = match mode & 0o022 {
        0 => false,
        0o020 => !(ancestor && owner == 0) && !private_group(),
        _ => true,
    };
    if kind != libc::S_IFLNK && loose {
        return Some(format!(
            "is writable by its group or others (mode {:o})",
            mode & 0o7777
        ));
    }
    None
}

fn path_problems(path: &Path, trusted: u32, strict_depth: usize) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let mut chains = vec![path.to_path_buf()];
    if let Ok(real) = path.canonicalize()
        && real != path
    {
        chains.push(real);
    }
    let mut problems = Vec::new();
    for chain in &chains {
        for (i, entry) in chain.ancestors().enumerate() {
            let problem = match std::fs::symlink_metadata(entry) {
                Ok(meta) => {
                    entry_problem(meta.uid(), meta.mode(), trusted, i > strict_depth, || {
                        is_private_group(meta.uid(), meta.gid())
                    })
                }
                Err(e) => Some(format!("cannot be inspected ({e})")),
            };
            if let Some(problem) = problem {
                problems.push(format!("{} {problem}", entry.display()));
            }
        }
    }
    problems
}

fn require_protected(paths: &[(&Path, usize)], user: &Account) -> Result<()> {
    let mut problems: Vec<String> = Vec::new();
    for problem in paths
        .iter()
        .flat_map(|&(path, strict_depth)| path_problems(path, user.uid, strict_depth))
    {
        if !problems.contains(&problem) {
            problems.push(problem);
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "refusing to register a system service that runs as {name}: only root and {name} may be able to change what it runs, but\n  {}\nInstall swing into a root-owned directory (install.sh --prefix /usr/local) and keep the config in a directory such as /etc/swing.",
        problems.join("\n  "),
        name = user.name,
    )
}

pub fn start(system: bool) -> Result<()> {
    systemctl(system, &["start", "swing"])?;
    println!("Started swing with systemd.");
    Ok(())
}

pub fn is_installed(system: bool) -> Option<bool> {
    Some(unit_path(system).is_ok_and(|p| p.exists()))
}

fn load_config(path: &Path) -> Option<crate::config::Config> {
    match crate::config::Config::load(Some(path)) {
        Ok(config) => Some(config),
        Err(e) => {
            println!(
                "Warning: could not read {} ({e:#}); the unit only lets swing write under the config file's directory, so keep [agent].state_dir and [kubo].repo there.",
                path.display()
            );
            None
        }
    }
}

fn writable_paths(config: Option<&crate::config::Config>) -> Vec<PathBuf> {
    config
        .map(|config| {
            [&config.agent.state_dir, &config.kubo.repo]
                .into_iter()
                .filter(|p| p.is_absolute())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn kubo_binary(config: Option<&crate::config::Config>) -> Option<PathBuf> {
    let kubo = &config?.kubo;
    if !kubo.managed {
        return None;
    }
    match &kubo.binary {
        Some(path) => Some(path.clone()),
        None => crate::kubo::locate_binary(None).ok(),
    }
}

pub fn install(exe: &Path, config: &Path, workdir: &Path, opts: &InstallOptions<'_>) -> Result<()> {
    let system = opts.system;
    let user = if system {
        Some(service_user(opts.run_as, opts.allow_root)?)
    } else {
        None
    };
    let loaded = if system { load_config(config) } else { None };
    if let Some(user) = &user {
        let kubo = kubo_binary(loaded.as_ref());
        let mut paths = vec![(exe, 1), (config, 0), (workdir, 0)];
        paths.extend(kubo.as_deref().map(|kubo| (kubo, 1)));
        require_protected(&paths, user)?;
    }
    let writable = writable_paths(loaded.as_ref());
    let scope = match &user {
        Some(user) => SystemdScope::System {
            user: &user.name,
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
        println!("The service runs as user {}.", user.name);
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
    fn passwd_entry_resolves_existing_users_only() {
        assert_eq!(passwd_entry(&ServiceUser::Uid(0)).unwrap().name, "root");
        assert_eq!(
            passwd_entry(&ServiceUser::Name("root".into())).unwrap().uid,
            0
        );
        assert!(passwd_entry(&ServiceUser::Name("swing-no-such-user-x".into())).is_err());
        assert!(passwd_entry(&ServiceUser::Name("a\0b".into())).is_err());
    }

    #[test]
    fn running_as_root_needs_allow_root() {
        for run_as in ["root", "0"] {
            let err = service_user(Some(run_as), false).err().unwrap();
            assert!(err.to_string().contains("--allow-root"), "{err:#}");
        }
        assert_eq!(service_user(Some("root"), true).unwrap().uid, 0);
    }

    #[test]
    fn entries_must_belong_to_root_or_the_user_and_not_be_shared_writable() {
        let dir = libc::S_IFDIR;
        let file = libc::S_IFREG;
        let check = |owner, mode, ancestor, private| {
            entry_problem(owner, mode, 1000, ancestor, || private).is_none()
        };
        assert!(check(0, file | 0o755, false, false));
        assert!(check(1000, file | 0o700, false, false));
        assert!(check(1000, dir | 0o755, true, false));
        assert!(check(0, dir | 0o1777, true, false));
        assert!(check(1000, libc::S_IFLNK | 0o777, false, false));
        assert!(check(1000, file | 0o775, false, true));
        assert!(check(1000, dir | 0o2775, true, true));
        assert!(!check(1001, file | 0o755, false, false));
        assert!(!check(1000, file | 0o775, false, false));
        assert!(!check(1000, file | 0o757, false, true));
        assert!(!check(0, dir | 0o757, true, false));
        assert!(check(0, dir | 0o2775, true, false));
        assert!(!check(0, dir | 0o2775, false, false));
        assert!(!check(0, file | 0o775, false, false));
        assert!(!check(1000, dir | 0o2775, true, false));
        assert!(!check(0, dir | 0o1777, false, false));
        assert!(!check(1001, dir | 0o1777, true, false));
    }

    #[test]
    fn the_directory_holding_a_binary_gets_no_ancestor_exemption() {
        use std::os::unix::fs::MetadataExt;
        let shared = Path::new("/tmp");
        let Ok(meta) = std::fs::symlink_metadata(shared) else {
            return;
        };
        if meta.uid() != 0 || meta.mode() & 0o1777 != 0o1777 {
            return;
        }
        let exe = shared.join("swing-binary-check");
        let names = |problems: Vec<String>| {
            problems
                .iter()
                .any(|p| p.starts_with(&format!("{} ", shared.display())))
        };
        assert!(!names(path_problems(&exe, 1000, 0)));
        assert!(names(path_problems(&exe, 1000, 1)));
        assert!(!names(path_problems(
            &shared.join("a").join("swing"),
            1000,
            1
        )));
    }

    #[test]
    fn path_problems_follow_symlinks_and_name_loose_parents() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let tmp = tempfile::tempdir().unwrap();
        let me = std::fs::metadata(tmp.path()).unwrap().uid();
        let dir = tmp.path().join("lib");
        let exe = dir.join("swing");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(&exe, b"").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        let mentions = |problems: Vec<String>, path: &Path| {
            let prefix = format!("{} ", path.display());
            problems.iter().any(|p| p.starts_with(&prefix))
        };
        assert!(!mentions(path_problems(&link, me, 0), &dir));

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o757)).unwrap();
        assert!(mentions(path_problems(&link, me, 0), &dir));
        assert!(path_problems(&exe, me, 0).contains(&format!(
            "{} is writable by its group or others (mode 757)",
            dir.display()
        )));
        if me != 0 {
            assert!(mentions(path_problems(&exe, me + 1, 0), &exe));
        }
        assert!(mentions(
            path_problems(&dir.join("missing"), me, 0),
            &dir.join("missing")
        ));
    }
}
