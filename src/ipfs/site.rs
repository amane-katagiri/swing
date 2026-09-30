use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt, stream};

use super::percent_encode_relative_path;

#[derive(Debug)]
enum Entry {
    Dir(String),
    File(String, PathBuf, u64),
}

fn walk(
    current: &Path,
    rel_prefix: &str,
    root: &Path,
    out: &mut Vec<Entry>,
    dir_ancestors: &mut HashSet<PathBuf>,
) -> Result<()> {
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
        if !canon.starts_with(root) {
            bail!(
                "{} is a symlink to {}, outside the site directory; copy what it points to into the site instead",
                path.display(),
                canon.display()
            );
        }
        let metadata = std::fs::metadata(&canon)
            .with_context(|| format!("reading metadata for {}", path.display()))?;
        if metadata.is_dir() {
            if !dir_ancestors.insert(canon.clone()) {
                bail!("symlink loop detected at {}", path.display());
            }
            out.push(Entry::Dir(rel.clone()));
            walk(&path, &rel, root, out, dir_ancestors)?;
            dir_ancestors.remove(&canon);
        } else if metadata.is_file() {
            out.push(Entry::File(rel, canon, metadata.len()));
        }
    }
    Ok(())
}

fn walk_root(dir: &Path) -> Result<Vec<Entry>> {
    let canon_root =
        std::fs::canonicalize(dir).with_context(|| format!("resolving path {}", dir.display()))?;
    let mut dir_ancestors = HashSet::new();
    dir_ancestors.insert(canon_root.clone());
    let mut entries = Vec::new();
    walk(dir, "", &canon_root, &mut entries, &mut dir_ancestors)?;
    Ok(entries)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteEntry {
    pub path: String,
    pub size: Option<u64>,
}

#[derive(Debug)]
pub struct SiteListing {
    dir: PathBuf,
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
        Ok(Self {
            dir: dir.to_path_buf(),
            root_name,
            entries: walk_root(dir)?,
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
                Entry::File(path, _, size) => SiteEntry {
                    path: path.clone(),
                    size: Some(*size),
                },
            })
            .collect()
    }

    pub(super) fn into_multipart(
        self,
        boundary: String,
    ) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
        multipart_body(self.root_name, self.entries, boundary)
    }
}

// reqwest's Form chains one stream per part, so polling it recurses once per file and overflows the stack on large sites.
fn multipart_body(
    root_name: String,
    entries: Vec<Entry>,
    boundary: String,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
    let end = Bytes::from(format!("--{boundary}--\r\n"));
    stream::iter(entries)
        .flat_map(move |entry| {
            let (rel, mime, path) = match entry {
                Entry::Dir(rel) => (rel, "application/x-directory", None),
                Entry::File(rel, path, _) => (rel, "application/octet-stream", Some(path)),
            };
            // Kubo infers the wrapping directory node from a shared filename prefix, so the root name must be included or files land as unlinked blobs.
            let header = Bytes::from(format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {mime}\r\n\r\n",
                percent_encode_relative_path(&format!("{root_name}/{rel}"))
            ));
            let content = stream::iter(path)
                .then(|path| async move {
                    tokio::fs::File::open(&path).await.map_err(|e| {
                        std::io::Error::new(e.kind(), format!("opening {}: {e}", path.display()))
                    })
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

    async fn collect_body(entries: Vec<Entry>, boundary: &str) -> std::io::Result<Vec<u8>> {
        let chunks: Vec<Bytes> = multipart_body("root".to_string(), entries, boundary.to_string())
            .try_collect()
            .await?;
        Ok(chunks.concat())
    }

    #[tokio::test]
    async fn multipart_body_encodes_dirs_and_files_like_reqwest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a b.css");
        std::fs::write(&file, b"body{}").unwrap();
        let body = collect_body(
            vec![
                Entry::Dir("css".into()),
                Entry::File("css/a b.css".into(), file, 6),
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
        let missing = std::path::PathBuf::from("/nonexistent/swing-test");
        let err = collect_body(vec![Entry::File("x".into(), missing, 0)], "B")
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
        let body = collect_body(entries, "B").await.unwrap();
        assert!(body.ends_with(b"--B--\r\n"));
    }

    fn entry_names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Dir(r) => format!("dir:{r}"),
                Entry::File(r, _, _) => format!("file:{r}"),
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

        let entries = walk_root(dir.path()).unwrap();
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

        let entries = walk_root(dir.path()).unwrap();
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

        let entries = walk_root(dir.path()).unwrap();
        let real = std::fs::canonicalize(dir.path().join("real.txt")).unwrap();
        let opened: Vec<(&str, &Path)> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::File(rel, path, _) => Some((rel.as_str(), path.as_path())),
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
    fn walk_rejects_symlink_loop() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        symlink(dir.path(), dir.path().join("self_loop")).unwrap();

        let err = walk_root(dir.path()).unwrap_err();
        assert!(err.to_string().contains("symlink loop"));
    }
}
