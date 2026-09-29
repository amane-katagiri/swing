use std::io::Read;

#[cfg(unix)]
mod unix {
    use std::ffi::CString;
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    pub struct Dir(File);

    fn describe(err: io::Error) -> String {
        match err.raw_os_error() {
            Some(libc::ELOOP) => "is a symlink".to_string(),
            Some(libc::ENOTDIR) => "is a symlink or not a directory".to_string(),
            _ => format!("cannot open: {err}"),
        }
    }

    impl Dir {
        pub fn open_root(path: &Path) -> io::Result<Self> {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY)
                .open(path)
                .map(Self)
        }

        pub fn open_dir(&self, name: &str) -> Result<Self, String> {
            self.open_at(name, libc::O_DIRECTORY).map(Self)
        }

        pub fn open_file(&self, name: &str) -> Result<File, String> {
            self.open_at(name, 0)
        }

        fn open_at(&self, name: &str, flags: libc::c_int) -> Result<File, String> {
            let c_name = CString::new(name).map_err(|_| "name contains NUL".to_string())?;
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC | flags,
                )
            };
            if fd < 0 {
                return Err(describe(io::Error::last_os_error()));
            }
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    pub struct Dir(PathBuf);

    fn final_path(file: &File) -> io::Result<PathBuf> {
        let mut buf = vec![0u16; 512];
        loop {
            let len = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            } as usize;
            if len == 0 {
                return Err(io::Error::last_os_error());
            }
            if len < buf.len() {
                buf.truncate(len);
                return Ok(PathBuf::from(OsString::from_wide(&buf)));
            }
            buf.resize(len, 0);
        }
    }

    impl Dir {
        pub fn open_root(path: &Path) -> io::Result<Self> {
            let canonical = std::fs::canonicalize(path)?;
            if !std::fs::metadata(&canonical)?.is_dir() {
                return Err(io::Error::other("not a directory"));
            }
            Ok(Self(canonical))
        }

        pub fn open_dir(&self, name: &str) -> Result<Self, String> {
            let dir = self.open_at(name, FILE_FLAG_BACKUP_SEMANTICS)?;
            let meta = dir
                .metadata()
                .map_err(|err| format!("cannot stat: {err}"))?;
            if !meta.is_dir() {
                return Err("is not a directory".to_string());
            }
            let path = final_path(&dir).map_err(|err| format!("cannot resolve: {err}"))?;
            Ok(Self(path))
        }

        pub fn open_file(&self, name: &str) -> Result<File, String> {
            self.open_at(name, 0)
        }

        fn open_at(&self, name: &str, flags: u32) -> Result<File, String> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | flags)
                .open(self.0.join(name))
                .map_err(|err| format!("cannot open: {err}"))?;
            let meta = file
                .metadata()
                .map_err(|err| format!("cannot stat: {err}"))?;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err("is a symlink or reparse point".to_string());
            }
            let opened = final_path(&file).map_err(|err| format!("cannot resolve: {err}"))?;
            if opened.parent() != Some(self.0.as_path()) {
                return Err("was moved outside its directory while opening".to_string());
            }
            Ok(file)
        }
    }
}

#[cfg(unix)]
pub use unix::Dir;
#[cfg(windows)]
pub use windows::Dir;

pub fn read_limited(dir: &Dir, name: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let file = dir.open_file(name)?;
    let meta = file
        .metadata()
        .map_err(|err| format!("cannot stat: {err}"))?;
    if !meta.is_file() {
        return Err("is not a regular file".to_string());
    }
    #[cfg(unix)]
    if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
        return Err("has more than one hard link".to_string());
    }
    if meta.len() > max_bytes {
        return Err(format!(
            "is {} bytes, over the {max_bytes} byte limit",
            meta.len()
        ));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("cannot read: {err}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!(
            "grew past the {max_bytes} byte limit while reading"
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_limited_rejects_a_file_over_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), [0u8; 11]).unwrap();
        std::fs::write(dir.path().join("g"), [0u8; 10]).unwrap();
        let root = Dir::open_root(dir.path()).unwrap();
        assert!(read_limited(&root, "f", 10).is_err());
        assert_eq!(read_limited(&root, "g", 10).unwrap().len(), 10);
    }

    #[cfg(unix)]
    #[test]
    fn opening_through_a_symlink_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("file"), b"x").unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("dir-link")).unwrap();
        std::os::unix::fs::symlink(real.join("file"), dir.path().join("file-link")).unwrap();
        let root = Dir::open_root(dir.path()).unwrap();
        assert!(root.open_dir("dir-link").is_err());
        assert_eq!(
            read_limited(&root, "file-link", 10).unwrap_err(),
            "is a symlink"
        );
        assert!(root.open_dir("real").is_ok());
    }
}
