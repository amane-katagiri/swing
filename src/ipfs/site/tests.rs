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
    assert_eq!(
        names,
        vec![
            "dir:css".to_string(),
            "file:css/style.css".to_string(),
            "dir:empty".to_string(),
            "file:index.html".to_string(),
        ]
    );
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
