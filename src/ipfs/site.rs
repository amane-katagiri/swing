use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt, stream};

use super::percent_encode_relative_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl FileId {
    #[cfg_attr(not(unix), allow(unused_variables))]
    fn of(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                dev: metadata.dev(),
                ino: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        Self {}
    }
}

#[derive(Debug)]
struct ListedFile {
    path: PathBuf,
    size: u64,
    id: FileId,
}

#[derive(Debug)]
enum Entry {
    Dir(String),
    File(String, ListedFile),
}

struct Walk<'a> {
    root: &'a Path,
    out: Vec<Entry>,
    dir_ancestors: HashSet<PathBuf>,
    linked_dirs: HashSet<PathBuf>,
}

impl Walk<'_> {
    fn dir(&mut self, current: &Path, rel_prefix: &str) -> Result<()> {
        let mut children: Vec<std::fs::DirEntry> = std::fs::read_dir(current)
            .with_context(|| format!("reading dir {}", current.display()))?
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("reading dir {}", current.display()))?;
        children.sort_by_key(|e| e.file_name());

        for entry in children {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = if rel_prefix.is_empty() {
                name.clone()
            } else {
                format!("{rel_prefix}/{name}")
            };
            let canon = std::fs::canonicalize(&path)
                .with_context(|| format!("resolving path {}", path.display()))?;
            if !canon.starts_with(self.root) {
                bail!(
                    "{} is a symlink to {}, outside the site directory; copy what it points to into the site instead",
                    path.display(),
                    canon.display()
                );
            }
            let metadata = std::fs::metadata(&canon)
                .with_context(|| format!("reading metadata for {}", path.display()))?;
            if metadata.is_dir() {
                if !self.dir_ancestors.insert(canon.clone()) {
                    bail!("symlink loop detected at {}", path.display());
                }
                // Each linked directory is expanded once, so links to links cannot multiply the listing exponentially.
                let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
                if is_link && !self.linked_dirs.insert(canon.clone()) {
                    bail!(
                        "{} links to {}, which another symlink in the site already links to; link each directory only once or copy it",
                        path.display(),
                        canon.display()
                    );
                }
                self.out.push(Entry::Dir(rel.clone()));
                self.dir(&path, &rel)?;
                self.dir_ancestors.remove(&canon);
            } else if metadata.is_file() {
                self.out.push(Entry::File(
                    rel,
                    ListedFile {
                        size: metadata.len(),
                        id: FileId::of(&metadata),
                        path: canon,
                    },
                ));
            }
        }
        Ok(())
    }
}

fn walk_root(dir: &Path) -> Result<(PathBuf, Vec<Entry>)> {
    let canon_root =
        std::fs::canonicalize(dir).with_context(|| format!("resolving path {}", dir.display()))?;
    let mut walk = Walk {
        root: &canon_root,
        out: Vec::new(),
        dir_ancestors: HashSet::from([canon_root.clone()]),
        linked_dirs: HashSet::new(),
    };
    walk.dir(dir, "")?;
    let entries = walk.out;
    Ok((canon_root, entries))
}

// Walks down from the root without following symlinks, so a directory swapped for a link after the listing cannot lead outside the site.
#[cfg(unix)]
fn open_below(root: &Path, path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};

    let rel = path.strip_prefix(root).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "not inside the site")
    })?;
    let names: Vec<&std::ffi::OsStr> = rel
        .components()
        .map(|c| match c {
            std::path::Component::Normal(name) => Ok(name),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a plain path inside the site",
            )),
        })
        .collect::<std::io::Result<_>>()?;
    let mut current = std::fs::File::open(root)?;
    for (i, name) in names.iter().enumerate() {
        let last = i + 1 == names.len();
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if last {
                libc::O_NONBLOCK
            } else {
                libc::O_DIRECTORY
            };
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        current = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Ok(current)
}

#[cfg(not(unix))]
fn open_below(_root: &Path, path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

fn open_listed(root: &Path, file: &ListedFile) -> std::io::Result<std::fs::File> {
    let opened = open_below(root, &file.path)?;
    if FileId::of(&opened.metadata()?) != file.id {
        return Err(std::io::Error::other(
            "it changed after the site was listed; publish again",
        ));
    }
    Ok(opened)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteEntry {
    pub path: String,
    pub size: Option<u64>,
}

#[derive(Debug)]
pub struct SiteListing {
    dir: PathBuf,
    root: PathBuf,
    root_name: String,
    entries: Vec<Entry>,
}

impl SiteListing {
    pub fn read(dir: &Path) -> Result<Self> {
        if !dir.is_dir() {
            bail!("not a directory: {}", dir.display());
        }
        let root_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "root".to_string());
        let (root, entries) = walk_root(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            root,
            root_name,
            entries,
        })
    }

    pub async fn read_async(dir: &Path) -> Result<Self> {
        let dir = dir.to_path_buf();
        tokio::task::spawn_blocking(move || Self::read(&dir))
            .await
            .context("listing task panicked")?
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> Vec<SiteEntry> {
        self.entries
            .iter()
            .map(|entry| match entry {
                Entry::Dir(path) => SiteEntry {
                    path: path.clone(),
                    size: None,
                },
                Entry::File(path, file) => SiteEntry {
                    path: path.clone(),
                    size: Some(file.size),
                },
            })
            .collect()
    }

    pub(super) fn into_multipart(
        self,
        boundary: String,
    ) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
        multipart_body(self.root, self.root_name, self.entries, boundary)
    }
}

// reqwest's Form chains one stream per part, so polling it recurses once per file and overflows the stack on large sites.
fn multipart_body(
    root: PathBuf,
    root_name: String,
    entries: Vec<Entry>,
    boundary: String,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
    let end = Bytes::from(format!("--{boundary}--\r\n"));
    let root = std::sync::Arc::new(root);
    stream::iter(entries)
        .flat_map(move |entry| {
            let (rel, mime, file) = match entry {
                Entry::Dir(rel) => (rel, "application/x-directory", None),
                Entry::File(rel, file) => (rel, "application/octet-stream", Some(file)),
            };
            let root = std::sync::Arc::clone(&root);
            // Kubo infers the wrapping directory node from a shared filename prefix, so the root name must be included or files land as unlinked blobs.
            let header = Bytes::from(format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {mime}\r\n\r\n",
                percent_encode_relative_path(&format!("{root_name}/{rel}"))
            ));
            let content = stream::iter(file)
                .then(move |file| {
                    let root = std::sync::Arc::clone(&root);
                    async move {
                        let path = file.path.clone();
                        tokio::task::spawn_blocking(move || open_listed(&root, &file))
                            .await
                            .map_err(std::io::Error::other)
                            .and_then(|opened| opened)
                            .map(tokio::fs::File::from_std)
                            .map_err(|e| {
                                std::io::Error::new(
                                    e.kind(),
                                    format!("opening {}: {e}", path.display()),
                                )
                            })
                    }
                })
                .map_ok(tokio_util::io::ReaderStream::new)
                .try_flatten();
            stream::once(std::future::ready(Ok(header)))
                .chain(content)
                .chain(stream::once(std::future::ready(Ok(Bytes::from_static(
                    b"\r\n",
                )))))
        })
        .chain(stream::once(std::future::ready(Ok(end))))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collect_body(
        root: &Path,
        entries: Vec<Entry>,
        boundary: &str,
    ) -> std::io::Result<Vec<u8>> {
        let chunks: Vec<Bytes> = multipart_body(
            root.to_path_buf(),
            "root".to_string(),
            entries,
            boundary.to_string(),
        )
        .try_collect()
        .await?;
        Ok(chunks.concat())
    }

    fn listed(path: PathBuf) -> ListedFile {
        let metadata = std::fs::metadata(&path).unwrap();
        ListedFile {
            size: metadata.len(),
            id: FileId::of(&metadata),
            path,
        }
    }

    #[tokio::test]
    async fn multipart_body_encodes_dirs_and_files_like_reqwest() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let file = root.join("a b.css");
        std::fs::write(&file, b"body{}").unwrap();
        let body = collect_body(
            &root,
            vec![
                Entry::Dir("css".into()),
                Entry::File("css/a b.css".into(), listed(file)),
            ],
            "XYZ",
        )
        .await
        .unwrap();
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "--XYZ\r\nContent-Disposition: form-data; name=\"file\"; filename=\"root/css\"\r\nContent-Type: application/x-directory\r\n\r\n\r\n\
             --XYZ\r\nContent-Disposition: form-data; name=\"file\"; filename=\"root/css/a%20b.css\"\r\nContent-Type: application/octet-stream\r\n\r\nbody{}\r\n\
             --XYZ--\r\n"
        );
    }

    #[tokio::test]
    async fn a_listing_sends_only_the_files_it_listed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
        let site = SiteListing::read(dir.path()).unwrap();
        std::fs::write(dir.path().join("late.txt"), b"late").unwrap();
        let chunks: Vec<Bytes> = site
            .into_multipart("B".to_string())
            .try_collect()
            .await
            .unwrap();
        let body = String::from_utf8(chunks.concat()).unwrap();
        assert!(body.contains("/index.html\""), "{body}");
        assert!(!body.contains("late.txt"), "{body}");
    }

    #[tokio::test]
    async fn multipart_body_names_a_file_it_cannot_open() {
        let missing = ListedFile {
            path: PathBuf::from("/nonexistent/swing-test"),
            size: 0,
            id: FileId::of(&std::fs::metadata(".").unwrap()),
        };
        let err = collect_body(
            Path::new("/nonexistent"),
            vec![Entry::File("x".into(), missing)],
            "B",
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("opening /nonexistent/swing-test"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn multipart_body_does_not_nest_per_part() {
        let entries = (0..100_000).map(|i| Entry::Dir(format!("d{i}"))).collect();
        let body = collect_body(Path::new("/"), entries, "B").await.unwrap();
        assert!(body.ends_with(b"--B--\r\n"));
    }

    fn entry_names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Dir(r) => format!("dir:{r}"),
                Entry::File(r, _) => format!("file:{r}"),
            })
            .collect()
    }

    #[test]
    fn walk_collects_files_and_dirs_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"hi").unwrap();
        std::fs::create_dir(dir.path().join("css")).unwrap();
        std::fs::write(dir.path().join("css/style.css"), b"body{}").unwrap();
        std::fs::create_dir(dir.path().join("empty")).unwrap();

        let (_, entries) = walk_root(dir.path()).unwrap();
        let names = entry_names(&entries);
        assert!(names.contains(&"file:index.html".to_string()));
        assert!(names.contains(&"dir:css".to_string()));
        assert!(names.contains(&"file:css/style.css".to_string()));
        assert!(names.contains(&"dir:empty".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn walk_follows_symlinked_files_and_dirs_inside_the_site() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("real_dir")).unwrap();
        std::fs::write(dir.path().join("real_dir/real.txt"), b"real").unwrap();
        std::fs::write(dir.path().join("real_file.txt"), b"hi").unwrap();

        symlink(
            dir.path().join("real_file.txt"),
            dir.path().join("link_file.txt"),
        )
        .unwrap();
        symlink("real_dir", dir.path().join("link_dir")).unwrap();

        let (_, entries) = walk_root(dir.path()).unwrap();
        let names = entry_names(&entries);
        assert!(names.contains(&"file:link_file.txt".to_string()));
        assert!(names.contains(&"dir:link_dir".to_string()));
        assert!(names.contains(&"file:link_dir/real.txt".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn walk_records_the_resolved_path_it_checked() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), b"hi").unwrap();
        symlink("real.txt", dir.path().join("link.txt")).unwrap();

        let (_, entries) = walk_root(dir.path()).unwrap();
        let real = std::fs::canonicalize(dir.path().join("real.txt")).unwrap();
        let opened: Vec<(&str, &Path)> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::File(rel, file) => Some((rel.as_str(), file.path.as_path())),
                Entry::Dir(_) => None,
            })
            .collect();
        assert_eq!(
            opened,
            vec![("link.txt", real.as_path()), ("real.txt", real.as_path())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_refuses_symlinks_leaving_the_site() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("id_ed25519"), b"secret").unwrap();

        let dir = tempfile::tempdir().unwrap();
        symlink(
            outside.path().join("id_ed25519"),
            dir.path().join("key.txt"),
        )
        .unwrap();
        let err = walk_root(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("outside the site directory"),
            "{err}"
        );

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        symlink(outside.path(), dir.path().join("sub/linked")).unwrap();
        let err = walk_root(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("outside the site directory"),
            "{err}"
        );

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        symlink("../escape", dir.path().join("sub/up")).unwrap();
        std::fs::create_dir(dir.path().join("escape")).unwrap();
        let err = walk_root(&dir.path().join("sub")).unwrap_err();
        assert!(
            err.to_string().contains("outside the site directory"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_expands_each_linked_directory_once() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        for level in 0..3 {
            std::fs::create_dir(dir.path().join(format!("d{level}"))).unwrap();
        }
        for level in 0..2 {
            for fan in ["a", "b"] {
                symlink(
                    format!("../d{}", level + 1),
                    dir.path().join(format!("d{level}/{fan}")),
                )
                .unwrap();
            }
        }
        let err = walk_root(dir.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("another symlink in the site already links to"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_directory_swapped_for_a_link_after_listing_is_not_followed() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(outside.path().join("sub")).unwrap();
        std::fs::write(outside.path().join("sub/page.html"), b"secret").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/page.html"), b"public").unwrap();
        let site = SiteListing::read(dir.path()).unwrap();
        std::fs::rename(dir.path().join("sub"), dir.path().join("old")).unwrap();
        symlink(outside.path().join("sub"), dir.path().join("sub")).unwrap();

        let result: std::io::Result<Vec<Bytes>> =
            site.into_multipart("B".to_string()).try_collect().await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("page.html"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_file_replaced_after_listing_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"old").unwrap();
        let site = SiteListing::read(dir.path()).unwrap();
        std::fs::write(dir.path().join("new.html"), b"new").unwrap();
        std::fs::rename(dir.path().join("new.html"), dir.path().join("index.html")).unwrap();

        let result: std::io::Result<Vec<Bytes>> =
            site.into_multipart("B".to_string()).try_collect().await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("changed after"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn walk_rejects_symlink_loop() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        symlink(dir.path(), dir.path().join("self_loop")).unwrap();

        let err = walk_root(dir.path()).unwrap_err();
        assert!(err.to_string().contains("symlink loop"));
    }
}
