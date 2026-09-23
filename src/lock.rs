use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug)]
pub struct InstanceLock {
    // Never read; kept alive so the OS releases the flock only when this is dropped.
    #[allow(dead_code)]
    file: std::fs::File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn acquire(state_dir: &Path) -> Result<InstanceLock> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("creating state dir {}", state_dir.display()))?;
    let path = state_dir.join("swing.lock");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;

    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            let mut existing = String::new();
            let pid = file
                .read_to_string(&mut existing)
                .ok()
                .map(|_| existing.trim().to_string())
                .filter(|s| !s.is_empty());
            match pid {
                Some(pid) => bail!(
                    "another swing instance is already running on {} (pid {pid})",
                    state_dir.display()
                ),
                None => bail!(
                    "another swing instance is already running on {}",
                    state_dir.display()
                ),
            }
        }
        Err(std::fs::TryLockError::Error(e)) => {
            return Err(e).with_context(|| format!("locking {}", path.display()));
        }
    }

    file.set_len(0)
        .with_context(|| format!("truncating {}", path.display()))?;
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("seeking {}", path.display()))?;
    write!(file, "{}", std::process::id())
        .with_context(|| format!("writing pid to {}", path.display()))?;
    file.flush()
        .with_context(|| format!("flushing {}", path.display()))?;

    Ok(InstanceLock { file, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_with_first_pid_then_succeeds_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).unwrap();

        let err = acquire(dir.path()).unwrap_err().to_string();
        let pid = std::process::id().to_string();
        assert!(err.contains(&pid), "{err}");
        assert!(err.contains("already running"), "{err}");

        drop(first);
        // Other tests fork child processes concurrently; a fork in flight shares the flock until it execs.
        let second = (0..50)
            .find_map(|_| {
                acquire(dir.path()).ok().or_else(|| {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    None
                })
            })
            .expect("lock should be released after drop");
        drop(second);
    }

    #[test]
    fn acquire_creates_state_dir_and_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("nested").join("state");
        let lock = acquire(&state_dir).unwrap();
        assert_eq!(lock.path(), state_dir.join("swing.lock"));
        assert!(lock.path().is_file());
    }
}
