use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

pub fn locate_binary(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if !path.is_file() {
            bail!("Kubo binary not found: {}", path.display());
        }
        return Ok(path.to_path_buf());
    }

    let exe_name = if cfg!(windows) { "ipfs.exe" } else { "ipfs" };

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            tracing::info!(
                path = %candidate.display(),
                "using Kubo binary found next to the swing executable"
            );
            return Ok(candidate);
        }
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(exe_name);
            if candidate.is_file() {
                tracing::info!(
                    path = %candidate.display(),
                    "using Kubo binary found on PATH"
                );
                return Ok(candidate);
            }
        }
    }

    bail!(
        "Kubo binary not found: no [kubo].binary configured, no {exe_name} next to the swing executable, and no {exe_name} on PATH"
    );
}

pub(super) async fn run(bin: &Path, repo: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new(bin);
    command.args(args).stdin(Stdio::null());
    if let Some(repo) = repo {
        command.env("IPFS_PATH", repo);
    }
    let output = command
        .output()
        .await
        .with_context(|| format!("running `{} {}`", bin.display(), args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "`{} {}` failed ({}): {}",
            bin.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub async fn version(bin: &Path) -> Result<String> {
    Ok(run(bin, None, &["version", "--number"])
        .await?
        .trim()
        .to_string())
}

pub async fn ensure_repo(bin: &Path, repo: &Path) -> Result<bool> {
    std::fs::create_dir_all(repo)
        .with_context(|| format!("creating Kubo repo dir {}", repo.display()))?;
    if repo.join("config").is_file() {
        return Ok(false);
    }
    run(bin, Some(repo), &["init"]).await?;
    Ok(true)
}
