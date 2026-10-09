use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::config::{ConfigOrigin, locate_config};
use crate::format::Sanitized;

mod ownership;
mod process;
mod templates;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use unsupported as platform;
#[cfg(windows)]
use windows as platform;

pub use ownership::{Part, Placement, Registration};
pub use templates::{
    SystemdScope, launchd_plist, launchd_tray_plist, schtasks_xml, systemd_unit, tray_run_command,
};

pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(90);

pub const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(60);

const MACOS_BUNDLE_ID: &str = "jp.ne.ama.swing";
const MACOS_LABEL: &str = MACOS_BUNDLE_ID;
const MACOS_TRAY_LABEL: &str = "jp.ne.ama.swing-tray";

#[derive(Debug, Default, Clone, Copy)]
pub struct InstallOptions<'a> {
    pub system: bool,
    pub run_as: Option<&'a str>,
    pub allow_root: bool,
    pub no_start: bool,
    pub no_tray: bool,
}

fn resolve_service_paths(
    config_path: Option<&Path>,
    system: bool,
) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let (config, origin) = locate_config(config_path);
    if !config.exists() {
        match origin {
            ConfigOrigin::UserDefault if !system => create_empty_config(&config)?,
            ConfigOrigin::Explicit => bail!("config file not found: {}", config.display()),
            ConfigOrigin::Cwd | ConfigOrigin::UserDefault => bail!(
                "service install needs a config file (swing.toml): pass --config or set SWING_CONFIG"
            ),
        }
    }
    let config = config
        .canonicalize()
        .with_context(|| format!("resolving config path {}", config.display()))?;
    let workdir = config
        .parent()
        .map(Path::to_path_buf)
        .context("config file has no parent directory")?;
    // Not canonicalized: resolving Homebrew's opt symlink would pin the service to a keg that `brew upgrade` removes.
    let exe = std::env::current_exe()
        .and_then(std::path::absolute)
        .context("resolving current executable path")?;
    Ok((config, workdir, exe))
}

fn create_empty_config(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        crate::auth::create_private_dir_all(dir)?;
    }
    match crate::auth::private_file_options().open(path) {
        Ok(_) => {
            println!("Created an empty config file at {}", path.display());
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(anyhow::Error::new(e).context(format!("creating {}", path.display()))),
    }
}

fn require_system_supported(system: bool) -> Result<()> {
    if system && !cfg!(target_os = "linux") {
        bail!("--system is only supported on Linux");
    }
    Ok(())
}

// Command::new searches the application directory before System32.
pub(crate) fn windows_system_tool(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root).join("System32").join(name)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_service_file(path: &Path, content: impl AsRef<[u8]>) -> Result<()> {
    write_atomic(path, content.as_ref()).with_context(|| format!("writing {}", path.display()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&tmp)?;
        file.write_all(content)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

const TRAY_RELATIVE_PATH: &str = if cfg!(windows) {
    "swing-tray.exe"
} else if cfg!(target_os = "macos") {
    "SWING.app/Contents/MacOS/swing-tray"
} else {
    "swing-tray"
};

pub fn tray_exe_path(exe: &Path) -> Option<PathBuf> {
    let next_to = |exe: &Path| {
        let path = exe.parent()?.join(TRAY_RELATIVE_PATH);
        path.is_file().then_some(path)
    };
    next_to(exe).or_else(|| next_to(&exe.canonicalize().ok()?))
}

#[cfg(any(windows, target_os = "macos"))]
fn install_tray(
    exe: &Path,
    config: &Path,
    workdir: &Path,
    opts: &InstallOptions<'_>,
) -> Result<()> {
    if opts.no_tray {
        return platform::unregister_tray();
    }
    let Some(tray) = tray_exe_path(exe) else {
        println!(
            "{TRAY_RELATIVE_PATH} was not found next to {}; skipping the tray icon.",
            exe.display()
        );
        return Ok(());
    };
    platform::register_tray(&tray, config, workdir, opts.no_start)
}

pub fn install(config_path: Option<&Path>, opts: &InstallOptions<'_>) -> Result<()> {
    require_system_supported(opts.system)?;
    if opts.run_as.is_some() && !opts.system {
        bail!("--run-as is only valid with --system");
    }
    let (config, workdir, exe) = resolve_service_paths(config_path, opts.system)?;
    platform::install(&exe, &config, &workdir, opts)
}

pub async fn uninstall(system: bool, only_from: Option<&Path>) -> Result<()> {
    require_system_supported(system)?;
    let Some(dir) = only_from else {
        return platform::uninstall_parts(system, true, true).await;
    };
    let dir = absolute_dir(dir)?;
    let registrations = platform::registrations(system)?;
    if registrations.is_empty() {
        println!("swing is not registered; nothing to uninstall.");
        return Ok(());
    }
    let mut service = false;
    let mut tray = false;
    for registration in &registrations {
        if registration_points_into(registration, &dir) {
            match registration.part {
                Part::Service => service = true,
                Part::Tray => tray = true,
            }
        } else {
            println!(
                "{}; left it as is.",
                Sanitized(describe_outside(registration, &dir))
            );
        }
    }
    if service || tray {
        platform::uninstall_parts(system, service, tray).await?;
    }
    Ok(())
}

pub fn placement(system: bool, dir: &Path) -> Result<Placement> {
    require_system_supported(system)?;
    let dir = absolute_dir(dir)?;
    let registrations = platform::registrations(system)?;
    if registrations.is_empty() {
        println!("not installed");
    }
    let inside: Vec<bool> = registrations
        .iter()
        .map(|registration| {
            let inside = registration_points_into(registration, &dir);
            if inside {
                println!(
                    "{} runs {}, which is under {}.",
                    Sanitized(&registration.what),
                    Sanitized(registration.exe.as_deref().unwrap_or_default()),
                    Sanitized(dir.display())
                );
            } else {
                println!("{}.", Sanitized(describe_outside(registration, &dir)));
            }
            inside
        })
        .collect();
    Ok(ownership::summarize(&inside))
}

fn absolute_dir(dir: &Path) -> Result<PathBuf> {
    std::path::absolute(dir).with_context(|| format!("resolving {}", dir.display()))
}

fn registration_points_into(registration: &Registration, dir: &Path) -> bool {
    registration
        .exe
        .as_deref()
        .is_some_and(|exe| ownership::exe_points_into(exe, dir))
}

fn describe_outside(registration: &Registration, dir: &Path) -> String {
    match &registration.exe {
        Some(exe) => format!(
            "{} runs {}, which is not under {}",
            registration.what,
            exe,
            dir.display()
        ),
        None => format!(
            "{} could not be read to tell which swing it runs, so it is treated as not under {}",
            registration.what,
            dir.display()
        ),
    }
}

pub async fn stop(system: bool) -> Result<()> {
    require_system_supported(system)?;
    platform::stop(system).await
}

pub fn start(system: bool) -> Result<()> {
    require_system_supported(system)?;
    platform::start(system)
}

pub fn is_installed(system: bool) -> Option<bool> {
    platform::is_installed(system)
}

pub fn status(system: bool) -> Result<()> {
    require_system_supported(system)?;
    platform::status(system)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn service_file_is_replaced_whole_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swing.service");
        write_service_file(&path, "old contents that are longer").unwrap();
        write_service_file(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn tray_exe_is_found_only_next_to_swing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("swing");
        assert_eq!(tray_exe_path(&exe), None);
        let tray = dir.path().join(TRAY_RELATIVE_PATH);
        std::fs::create_dir_all(tray.parent().unwrap()).unwrap();
        std::fs::write(&tray, b"").unwrap();
        assert_eq!(tray_exe_path(&exe), Some(tray));
    }

    #[cfg(unix)]
    #[test]
    fn tray_exe_is_found_next_to_the_symlink_before_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        std::fs::write(real.join("swing"), b"").unwrap();
        let tray = real.join(TRAY_RELATIVE_PATH);
        std::fs::create_dir_all(tray.parent().unwrap()).unwrap();
        std::fs::write(&tray, b"").unwrap();
        assert_eq!(
            tray_exe_path(&link.join("swing")),
            Some(link.join(TRAY_RELATIVE_PATH))
        );

        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(real.join("swing"), bin.join("swing")).unwrap();
        assert_eq!(
            tray_exe_path(&bin.join("swing")),
            Some(std::fs::canonicalize(&tray).unwrap())
        );
    }

    #[test]
    fn empty_config_is_created_with_its_directories_and_kept_if_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("swing").join("swing.toml");
        create_empty_config(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert!(crate::config::Config::load(Some(&path)).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }
        std::fs::write(&path, "[nostr]\n").unwrap();
        create_empty_config(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[nostr]\n");
    }

    #[test]
    fn run_as_needs_system() {
        let err = install(
            None,
            &InstallOptions {
                run_as: Some("swing"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("--run-as"), "{err:#}");
    }
}
