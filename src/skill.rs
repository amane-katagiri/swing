use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub const NAME: &str = "swing-publish";
pub const FILE_NAME: &str = "SKILL.md";
const SOURCE: &str = include_str!("../skills/swing-publish/SKILL.md");
const VERSION_PLACEHOLDER: &str = "{{version}}";

#[cfg(windows)]
const HOME_VAR: &str = "USERPROFILE";
#[cfg(not(windows))]
const HOME_VAR: &str = "HOME";

pub fn render() -> String {
    SOURCE.replace(VERSION_PLACEHOLDER, env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    Created,
    UpToDate,
    Updated,
    Replaced,
}

pub fn default_skills_dir() -> Result<PathBuf> {
    skills_dir_from(|name| std::env::var(name).ok())
}

fn skills_dir_from(env: impl Fn(&str) -> Option<String>) -> Result<PathBuf> {
    let home = env(HOME_VAR)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .with_context(|| {
            format!("cannot determine the home directory ({HOME_VAR} is not an absolute path); pass --dir")
        })?;
    Ok(home.join(".claude").join("skills"))
}

pub fn install_into(skills_dir: &Path, force: bool) -> Result<(PathBuf, Installed)> {
    let dir = skills_dir.join(NAME);
    let path = dir.join(FILE_NAME);
    let content = render();
    let installed = match std::fs::read(&path) {
        Ok(existing) if existing == content.as_bytes() => return Ok((path, Installed::UpToDate)),
        Ok(existing) if names_this_skill(&String::from_utf8_lossy(&existing)) => Installed::Updated,
        Ok(_) if force => Installed::Replaced,
        Ok(_) => bail!(
            "{} exists and is not the {NAME} skill; rerun with --force to overwrite it",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Installed::Created,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok((path, installed))
}

pub fn install(skills_dir: Option<&Path>, force: bool) -> Result<()> {
    let skills_dir = match skills_dir {
        Some(dir) => dir.to_path_buf(),
        None => default_skills_dir()?,
    };
    let (path, installed) = install_into(&skills_dir, force)?;
    let version = env!("CARGO_PKG_VERSION");
    let path = path.display();
    match installed {
        Installed::Created => println!("installed the {NAME} skill for swing {version} at {path}"),
        Installed::UpToDate => println!("{path} is up to date (swing {version})"),
        Installed::Updated => println!("updated {path} to swing {version}"),
        Installed::Replaced => {
            println!("replaced {path} with the {NAME} skill for swing {version}")
        }
    }
    Ok(())
}

fn names_this_skill(text: &str) -> bool {
    let mut lines = text.lines().map(str::trim_end);
    if lines.next().map(|l| l.trim_start_matches('\u{feff}')) != Some("---") {
        return false;
    }
    lines
        .take_while(|l| *l != "---")
        .filter_map(|l| l.strip_prefix("name:"))
        .any(|v| v.trim().trim_matches(['"', '\'']) == NAME)
}

#[cfg(test)]
mod tests;
