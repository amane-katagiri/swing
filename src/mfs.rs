use crate::ipfs::percent_encode_segment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfsLayout {
    root: String,
}

impl MfsLayout {
    pub fn new(root: impl Into<String>) -> Self {
        Self { root: root.into() }
    }

    pub fn agent_root(&self) -> String {
        format!("{}/agent", self.root)
    }

    pub fn agent_account(&self, pubkey_hex: &str) -> String {
        format!("{}/{pubkey_hex}", self.agent_root())
    }

    pub fn agent_site(&self, pubkey_hex: &str, d: &str) -> String {
        format!("{}/{}", self.agent_account(pubkey_hex), site_name(d))
    }

    pub fn agent_version(&self, pubkey_hex: &str, d: &str, created_at: u64) -> String {
        format!("{}/{created_at}", self.agent_site(pubkey_hex, d))
    }

    pub fn publish_account(&self, pubkey_hex: &str) -> String {
        format!("{}/publish/{pubkey_hex}", self.root)
    }

    pub fn publish_site(&self, pubkey_hex: &str, d: &str) -> String {
        format!("{}/{}", self.publish_account(pubkey_hex), site_name(d))
    }

    pub fn publish_version(&self, pubkey_hex: &str, d: &str, created_at: u64) -> String {
        format!("{}/{created_at}", self.publish_site(pubkey_hex, d))
    }
}

fn site_name(d: &str) -> String {
    match d {
        "." | ".." => d.replace('.', "%2E"),
        _ => percent_encode_segment(d),
    }
}

pub fn site_from_name(name: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(name.len());
    let mut rest = name.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        if b == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(b);
            rest = tail;
        }
    }
    let d = String::from_utf8(bytes).ok()?;
    (site_name(&d) == name).then_some(d)
}

pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_namespaced_and_encode_the_site() {
        let layout = MfsLayout::new("/swing");
        assert_eq!(layout.agent_root(), "/swing/agent");
        assert_eq!(layout.agent_account("ab"), "/swing/agent/ab");
        assert_eq!(
            layout.agent_version("ab", "example.com", 100),
            "/swing/agent/ab/example.com/100"
        );
        assert_eq!(
            layout.agent_site("ab", "a/b %c"),
            "/swing/agent/ab/a%2Fb%20%25c"
        );
        assert_eq!(
            layout.publish_version("ab", "example.com", 7),
            "/swing/publish/ab/example.com/7"
        );
        assert_eq!(layout.publish_account("ab"), "/swing/publish/ab");
        assert_eq!(parent("/swing/agent/ab"), "/swing/agent");
    }

    #[test]
    fn site_name_never_yields_a_path_segment_with_special_meaning() {
        assert_eq!(site_name("a/b"), "a%2Fb");
        assert_eq!(site_name("."), "%2E");
        assert_eq!(site_name(".."), "%2E%2E");
        assert_eq!(site_name("..."), "...");
    }

    #[test]
    fn site_from_name_inverts_site_name_and_rejects_other_spellings() {
        for d in ["example.com", "a/b %c", ".", "..", "日本.example"] {
            assert_eq!(site_from_name(&site_name(d)).as_deref(), Some(d));
        }
        for name in ["a%2fb", "%2", "%zz", "a b", "%FF", "."] {
            assert_eq!(site_from_name(name), None, "{name}");
        }
    }
}
