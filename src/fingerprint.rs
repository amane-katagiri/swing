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

    #[test]
    fn fingerprint_hashes_the_x_only_pubkey() {
        let pk =
            PublicKey::from_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();
        assert_eq!(fingerprint(&pk), ["1f43c", "1f421", "1f33f", "1f52e"]);
    }
}
