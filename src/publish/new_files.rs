use std::collections::HashSet;

use crate::ipfs::{IpfsClient, SiteEntry};
use crate::nostr::SiteEvent;

const LISTED_NEW_FILES: usize = 50;
const MAX_PREVIOUS_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviousFiles {
    NoPrevious,
    Unknown(String),
    Listed {
        cid: String,
        created_at: u64,
        paths: Vec<String>,
    },
}

impl PreviousFiles {
    pub async fn load(ipfs: &IpfsClient, previous: &Result<Option<SiteEvent>, String>) -> Self {
        match previous {
            Err(e) => Self::Unknown(format!("could not fetch it from the relays: {e}")),
            Ok(None) => Self::NoPrevious,
            Ok(Some(site)) => match ipfs.list_files_local(&site.cid, MAX_PREVIOUS_ENTRIES).await {
                Ok(paths) => Self::Listed {
                    cid: site.cid.clone(),
                    created_at: site.created_at,
                    paths,
                },
                Err(e) => Self::Unknown(format!(
                    "could not list {} in the local Kubo: {e:#}",
                    site.cid
                )),
            },
        }
    }

    pub fn new_files(&self, entries: &[SiteEntry]) -> Vec<String> {
        let known: HashSet<&str> = match self {
            Self::Listed { paths, .. } => paths.iter().map(String::as_str).collect(),
            _ => HashSet::new(),
        };
        entries
            .iter()
            .filter(|e| e.size.is_some() && !known.contains(e.path.as_str()))
            .map(|e| e.path.clone())
            .collect()
    }

    pub fn lines(&self, new_files: &[String]) -> Vec<String> {
        let mut lines = vec![match self {
            Self::Listed { cid, .. } => {
                format!("compared with your latest version on the relays ({cid})")
            }
            Self::NoPrevious => {
                "no previous version on the relays; every file counts as new".to_string()
            }
            Self::Unknown(reason) => {
                format!("! previous version unavailable ({reason}); every file counts as new")
            }
        }];
        if new_files.is_empty() {
            lines.push("\u{2713} no new files".to_string());
            return lines;
        }
        lines.push(format!("! {}", count_new_files(new_files.len())));
        let mut listed = 0;
        for (folder, names) in group_by_folder(new_files) {
            if listed == LISTED_NEW_FILES {
                break;
            }
            let indent = if folder.is_empty() {
                "    "
            } else {
                lines.push(format!("    {folder}/"));
                "      "
            };
            for name in names.iter().take(LISTED_NEW_FILES - listed) {
                lines.push(format!("{indent}{name}"));
                listed += 1;
            }
        }
        if new_files.len() > LISTED_NEW_FILES {
            lines.push(format!(
                "    \u{2026} and {} more",
                new_files.len() - LISTED_NEW_FILES
            ));
        }
        lines
    }
}

fn group_by_folder(paths: &[String]) -> Vec<(&str, Vec<&str>)> {
    let mut split: Vec<(&str, &str)> = paths
        .iter()
        .map(|path| path.rsplit_once('/').unwrap_or(("", path)))
        .collect();
    split.sort();
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for (folder, name) in split {
        match groups.last_mut() {
            Some((last, names)) if *last == folder => names.push(name),
            _ => groups.push((folder, vec![name])),
        }
    }
    groups
}

pub(super) fn count_new_files(n: usize) -> String {
    if n == 1 {
        "1 new file".to_string()
    } else {
        format!("{n} new files")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(paths: &[(&str, Option<u64>)]) -> Vec<SiteEntry> {
        paths
            .iter()
            .map(|(path, size)| SiteEntry {
                path: path.to_string(),
                size: *size,
            })
            .collect()
    }

    #[test]
    fn new_files_are_the_files_missing_from_the_previous_version() {
        let site = entries(&[
            ("index.html", Some(1)),
            ("keys", None),
            ("keys/id_ed25519", Some(1)),
            ("secrets.json", Some(1)),
        ]);
        let previous = PreviousFiles::Listed {
            cid: "bafy0".into(),
            created_at: 1,
            paths: vec!["index.html".into(), "old.html".into()],
        };
        assert_eq!(
            previous.new_files(&site),
            vec!["keys/id_ed25519", "secrets.json"]
        );
    }

    #[test]
    fn without_a_previous_listing_every_file_is_new() {
        let site = entries(&[("a", None), ("a/b", Some(1)), ("c", Some(0))]);
        for previous in [
            PreviousFiles::NoPrevious,
            PreviousFiles::Unknown("offline".into()),
        ] {
            assert_eq!(previous.new_files(&site), vec!["a/b", "c"]);
        }
    }

    #[test]
    fn new_files_are_listed_by_folder_with_the_root_first() {
        let paths: Vec<String> = [
            "testekey",
            "sss/index.html",
            "index.html",
            "a/b/c.txt",
            "testekey.pub",
            "a/z.txt",
        ]
        .iter()
        .map(|p| p.to_string())
        .collect();
        assert_eq!(
            group_by_folder(&paths),
            vec![
                ("", vec!["index.html", "testekey", "testekey.pub"]),
                ("a", vec!["z.txt"]),
                ("a/b", vec!["c.txt"]),
                ("sss", vec!["index.html"]),
            ]
        );
        assert_eq!(
            PreviousFiles::NoPrevious.lines(&paths)[2..],
            [
                "    index.html",
                "    testekey",
                "    testekey.pub",
                "    a/",
                "      z.txt",
                "    a/b/",
                "      c.txt",
                "    sss/",
                "      index.html",
            ]
        );
    }

    #[test]
    fn lines_cap_the_listing() {
        let many: Vec<String> = (0..LISTED_NEW_FILES + 2).map(|i| format!("f{i}")).collect();
        let lines = PreviousFiles::NoPrevious.lines(&many);
        assert_eq!(lines[1], format!("! {} new files", LISTED_NEW_FILES + 2));
        assert_eq!(
            PreviousFiles::NoPrevious.lines(&many[..1])[1],
            "! 1 new file"
        );
        assert_eq!(lines.len(), 2 + LISTED_NEW_FILES + 1);
        assert_eq!(lines.last().unwrap(), "    \u{2026} and 2 more");
        let unknown = PreviousFiles::Unknown("boom".into()).lines(&[]);
        assert!(unknown[0].contains("boom"), "{unknown:?}");
        assert_eq!(unknown[1], "\u{2713} no new files");
    }
}
