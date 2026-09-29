use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug)]
pub struct InstanceLock {
    _file: std::fs::File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub enum TryAcquire {
    Acquired(InstanceLock),
    Held { pid: Option<String> },
}

pub fn acquire(state_dir: &Path) -> Result<InstanceLock> {
    match try_acquire(state_dir, "swing.lock")? {
        TryAcquire::Acquired(lock) => Ok(lock),
        TryAcquire::Held { pid: Some(pid) } => bail!(
            "another swing instance is already running on {} (pid {pid})",
            state_dir.display()
        ),
        TryAcquire::Held { pid: None } => bail!(
            "another swing instance is already running on {}",
            state_dir.display()
        ),
    }
}

pub fn try_acquire(state_dir: &Path, file_name: &str) -> Result<TryAcquire> {
    crate::auth::create_private_dir_all(state_dir)?;
    let path = state_dir.join(file_name);
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
            return Ok(TryAcquire::Held { pid });
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

    Ok(TryAcquire::Acquired(InstanceLock { _file: file, path }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_with_first_pid_then_succeeds_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).unwrap();

        let err = acquire(dir.path()).unwrap_err().to_string();
        #[cfg(unix)]
        assert!(err.contains(&std::process::id().to_string()), "{err}");
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

    #[test]
    fn try_acquire_uses_the_given_file_name_and_reports_a_held_lock() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("state");
        let TryAcquire::Acquired(first) = try_acquire(&state_dir, "swing-tray.lock").unwrap()
        else {
            panic!("first lock should be acquired");
        };
        assert_eq!(first.path(), state_dir.join("swing-tray.lock"));
        assert!(matches!(
            try_acquire(&state_dir, "swing-tray.lock").unwrap(),
            TryAcquire::Held { .. }
        ));
        let _other = acquire(&state_dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn acquire_creates_state_dir_as_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("nested").join("state");
        let _lock = acquire(&state_dir).unwrap();
        let mode = std::fs::metadata(&state_dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
