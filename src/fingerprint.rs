use std::sync::LazyLock;

use nostr_sdk::prelude::PublicKey;
use sha2::{Digest, Sha256};

pub const LENGTH: usize = 4;

const LIST: &str = include_str!("../web/emoji/list.txt");

static EMOJI: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    LIST.lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect()
});

pub fn fingerprint(pk: &PublicKey) -> Vec<String> {
    let digest = Sha256::digest(pk.to_bytes());
    digest[..LENGTH]
        .iter()
        .map(|b| EMOJI[usize::from(*b)].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn list_has_256_distinct_single_codepoints() {
        assert_eq!(EMOJI.len(), 256);
        assert_eq!(EMOJI.iter().collect::<HashSet<_>>().len(), 256);
        for id in EMOJI.iter() {
            let cp = u32::from_str_radix(id, 16).unwrap();
            assert!(char::from_u32(cp).is_some(), "{id}");
            assert_eq!(*id, format!("{cp:x}"));
        }
    }

    #[test]
    fn every_emoji_has_a_sprite_symbol() {
        let sprite = include_str!("../web/emoji.svg");
        assert_eq!(sprite.matches("<symbol ").count(), EMOJI.len());
        for id in EMOJI.iter() {
            assert!(sprite.contains(&format!("<symbol id=\"e-{id}\" ")), "{id}");
        }
    }

    const SPEC: &str = include_str!("../docs/fingerprint-spec.md");

    fn spec_rows() -> impl Iterator<Item = Vec<&'static str>> {
        SPEC.lines()
            .filter_map(|line| line.strip_prefix("| ")?.strip_suffix(" |"))
            .map(|row| row.split(" | ").collect())
    }

    fn codepoint(cell: &str) -> String {
        cell.strip_prefix("U+").unwrap().to_lowercase()
    }

    #[test]
    fn list_matches_the_spec_table() {
        let table: Vec<String> = spec_rows()
            .filter(|cells| cells[0].parse::<usize>().is_ok())
            .enumerate()
            .map(|(i, cells)| {
                assert_eq!(cells[0], i.to_string());
                codepoint(cells[1])
            })
            .collect();
        assert_eq!(table, *EMOJI);
    }

    #[test]
    fn fingerprint_matches_the_spec_vectors() {
        let vectors: Vec<_> = spec_rows()
            .filter_map(|cells| Some((cells[0].strip_prefix('`')?.strip_suffix('`')?, cells[2])))
            .collect();
        assert_eq!(vectors.len(), 3);
        for (hex, expected) in vectors {
            let pk = PublicKey::from_hex(hex).unwrap();
            let expected: Vec<String> = expected.split(' ').map(codepoint).collect();
            assert_eq!(fingerprint(&pk), expected, "{hex}");
        }
    }
}
