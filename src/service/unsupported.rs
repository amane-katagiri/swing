use std::path::Path;

use anyhow::{Result, bail};

use super::InstallOptions;
use super::ownership::Registration;

pub fn install(
    _exe: &Path,
    _config: &Path,
    _workdir: &Path,
    _opts: &InstallOptions<'_>,
) -> Result<()> {
    bail!("service management is not supported on this OS")
}

pub fn registrations(_system: bool) -> Result<Vec<Registration>> {
    bail!("service management is not supported on this OS")
}

pub async fn uninstall_parts(_system: bool, _service: bool, _tray: bool) -> Result<()> {
    bail!("service management is not supported on this OS")
}

pub async fn stop(_system: bool) -> Result<()> {
    bail!("service management is not supported on this OS")
}

pub fn start(_system: bool) -> Result<()> {
    bail!("service management is not supported on this OS")
}

pub fn is_installed(_system: bool) -> Option<bool> {
    Some(false)
}

pub fn status(_system: bool) -> Result<()> {
    bail!("service management is not supported on this OS")
}
