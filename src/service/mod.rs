use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::config::{ConfigOrigin, locate_config};

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
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
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
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}

const TRAY_RELATIVE_PATH: &str = if cfg!(windows) {
    "swing-tray.exe"
} else if cfg!(target_os = "macos") {
    "SWING.app/Contents/MacOS/swing-tray"
} else {
    "swing-tray"
};

pub fn tray_exe_path(exe: &Path) -> Option<PathBuf> {
    let path = exe.parent()?.join(TRAY_RELATIVE_PATH);
    path.is_file().then_some(path)
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

pub async fn uninstall(system: bool) -> Result<()> {
    require_system_supported(system)?;
    platform::uninstall(system).await
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
