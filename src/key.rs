use anyhow::Result;
use nostr_sdk::prelude::*;
use zeroize::Zeroizing;

pub struct GeneratedKeys {
    pub nsec: Zeroizing<String>,
    pub npub: String,
    pub secret_hex: Zeroizing<String>,
    pub public_hex: String,
}

fn describe(keys: &Keys) -> GeneratedKeys {
    GeneratedKeys {
        nsec: keys
            .secret_key()
            .to_bech32()
            .expect("nsec encoding is infallible")
            .into(),
        npub: keys
            .public_key()
            .to_bech32()
            .expect("npub encoding is infallible"),
        secret_hex: keys.secret_key().to_secret_hex().into(),
        public_hex: keys.public_key().to_hex(),
    }
}

pub fn generate() -> Result<()> {
    let generated = describe(&Keys::generate());
    println!("nsec: {}", *generated.nsec);
    println!("npub: {}", generated.npub);
    println!("hex (secret): {}", *generated.secret_hex);
    println!("hex (public): {}", generated.public_hex);
    println!();
    println!(
        "Put the nsec (or the secret hex) into SWING_NOSTR_SECRET_KEY in .env. Keep it private."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nsec_roundtrips_to_same_npub() {
        let keys = Keys::generate();
        let generated = describe(&keys);
        let parsed = Keys::parse(&generated.nsec).unwrap();
        assert_eq!(parsed.public_key().to_bech32().unwrap(), generated.npub);
    }

    #[test]
    fn secret_hex_roundtrips_to_same_public_hex() {
        let keys = Keys::generate();
        let generated = describe(&keys);
        let parsed = Keys::parse(&generated.secret_hex).unwrap();
        assert_eq!(parsed.public_key().to_hex(), generated.public_hex);
    }
}
