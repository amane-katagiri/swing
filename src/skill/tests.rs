use std::path::Path;

use super::*;
use crate::publish::dry_run::FORMAT;
use crate::publish::links::SITE_GUIDE_URL;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn frontmatter(text: &str) -> Vec<(String, String)> {
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("---"));
    lines
        .take_while(|l| *l != "---")
        .map(|l| {
            let (k, v) = l.split_once(':').expect("key: value");
            (k.to_string(), v.trim().to_string())
        })
        .collect()
}

#[test]
fn frontmatter_names_the_skill_and_says_when_to_use_it() {
    let fields = frontmatter(&render());
    let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["name", "description"]);
    assert_eq!(fields[0].1, NAME);
    let description = &fields[1].1;
    assert!(description.contains("Use when"));
    assert!(description.len() <= 1024);
    assert!(names_this_skill(&render()));
}

#[test]
fn render_fills_every_version_placeholder() {
    assert!(SOURCE.contains(VERSION_PLACEHOLDER));
    let text = render();
    assert!(!text.contains("{{"));
    assert!(text.contains(&format!("swing {VERSION}")));
}

#[test]
fn links_point_to_existing_docs_of_this_version() {
    let text = render();
    let prefix = format!("https://github.com/amane-katagiri/swing/blob/v{VERSION}/");
    let mut found = 0;
    for (i, _) in text.match_indices("https://github.com/") {
        let url = text[i..]
            .split(|c: char| c.is_whitespace() || c == ')')
            .next()
            .unwrap();
        let path = url.strip_prefix(&prefix).unwrap_or_else(|| panic!("{url}"));
        let path = path.split('#').next().unwrap();
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(path).is_file(),
            "{path}"
        );
        found += 1;
    }
    assert!(found >= 3);
    assert!(SITE_GUIDE_URL.starts_with(&prefix));
}

#[test]
fn states_the_json_format_of_the_dry_run() {
    assert!(render().contains(&format!("check that `format` is `{FORMAT}`")));
}

#[test]
fn recognizes_the_skill_by_its_frontmatter_name() {
    assert!(names_this_skill(
        "---\r\nname: \"swing-publish\"\r\ndescription: old\r\n---\r\nbody\r\n"
    ));
    assert!(names_this_skill("\u{feff}---\nname: swing-publish\n---\n"));
    assert!(!names_this_skill(
        "---\nname: other\n---\nname: swing-publish\n"
    ));
    assert!(!names_this_skill("name: swing-publish\n"));
    assert!(!names_this_skill("---\nname: swing-publish-extra\n---\n"));
}

#[test]
fn install_creates_the_skill_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = tmp.path().join("nested").join("skills");
    let (path, installed) = install_into(&skills, false).unwrap();
    assert_eq!(installed, Installed::Created);
    assert_eq!(path, skills.join(NAME).join(FILE_NAME));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), render());
}

#[test]
fn install_leaves_an_identical_file_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _) = install_into(tmp.path(), false).unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let (_, installed) = install_into(tmp.path(), false).unwrap();
    assert_eq!(installed, Installed::UpToDate);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
}

#[test]
fn install_updates_an_older_copy_of_the_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(NAME).join(FILE_NAME);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "---\nname: swing-publish\ndescription: old\n---\nold\n",
    )
    .unwrap();
    let (_, installed) = install_into(tmp.path(), false).unwrap();
    assert_eq!(installed, Installed::Updated);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), render());
}

#[test]
fn install_stops_at_another_file_unless_forced() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(NAME).join(FILE_NAME);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "my own notes\n").unwrap();
    let err = install_into(tmp.path(), false).unwrap_err();
    assert!(err.to_string().contains("--force"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "my own notes\n");
    let (_, installed) = install_into(tmp.path(), true).unwrap();
    assert_eq!(installed, Installed::Replaced);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), render());
}

#[test]
fn default_dir_is_under_the_home_directory() {
    let home = if cfg!(windows) {
        r"C:\Users\u"
    } else {
        "/home/u"
    };
    let dir = skills_dir_from(|name| (name == HOME_VAR).then(|| home.to_string())).unwrap();
    assert_eq!(dir, Path::new(home).join(".claude").join("skills"));
    assert!(skills_dir_from(|_| None).is_err());
    assert!(skills_dir_from(|_| Some("relative".to_string())).is_err());
}
