use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Attr {
    pub name: String,
    pub value: String,
    pub certain: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Tag {
    pub name: String,
    pub attrs: Vec<Attr>,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attr_with_certainty(name).map(|(v, _)| v)
    }

    pub fn attr_with_certainty(&self, name: &str) -> Option<(&str, bool)> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| (a.value.as_str(), a.certain))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Node<'a> {
    Tag(Tag),
    Script(&'a str),
    Style(&'a str),
}

const RAW_TEXT: [&str; 8] = [
    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes",
];

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

pub(super) fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn tag_name_at(b: &[u8], at: usize, name: &str) -> bool {
    let end = at + name.len();
    end <= b.len()
        && b[at..end].eq_ignore_ascii_case(name.as_bytes())
        && (end == b.len() || is_space(b[end]) || b[end] == b'/' || b[end] == b'>')
}

fn find_end_tag(b: &[u8], from: usize, name: &str) -> usize {
    let mut i = from;
    while let Some(p) = find(b, i, b"</") {
        if tag_name_at(b, p + 2, name) {
            return p;
        }
        i = p + 2;
    }
    b.len()
}

// Follows the script data escaped and double-escaped states, so `<!-- <script></script> -->` inside a script does not end it.
fn find_script_end(b: &[u8], from: usize) -> usize {
    #[derive(PartialEq)]
    enum State {
        Data,
        Escaped,
        DoubleEscaped,
    }
    let mut state = State::Data;
    let mut i = from;
    while i < b.len() {
        match state {
            State::Data => {
                if b[i..].starts_with(b"<!--") {
                    state = State::Escaped;
                    i += 4;
                    continue;
                }
                if b[i..].starts_with(b"</") && tag_name_at(b, i + 2, "script") {
                    return i;
                }
            }
            State::Escaped => {
                if b[i..].starts_with(b"-->") {
                    state = State::Data;
                    i += 3;
                    continue;
                }
                if b[i..].starts_with(b"</") && tag_name_at(b, i + 2, "script") {
                    return i;
                }
                if b[i] == b'<' && tag_name_at(b, i + 1, "script") {
                    state = State::DoubleEscaped;
                    i += 7;
                    continue;
                }
            }
            State::DoubleEscaped => {
                if b[i..].starts_with(b"-->") {
                    state = State::Data;
                    i += 3;
                    continue;
                }
                if b[i..].starts_with(b"</") && tag_name_at(b, i + 2, "script") {
                    state = State::Escaped;
                    i += 8;
                    continue;
                }
            }
        }
        i += 1;
    }
    b.len()
}

fn comment_end(b: &[u8], from: usize) -> usize {
    let mut i = from;
    while let Some(p) = find(b, i, b"--") {
        let mut q = p;
        while b.get(q) == Some(&b'-') {
            q += 1;
        }
        match b.get(q) {
            Some(b'>') => return q + 1,
            Some(b'!') if b.get(q + 1) == Some(&b'>') => return q + 2,
            _ => i = q.max(p + 1),
        }
    }
    b.len()
}

pub(super) fn parse(s: &str) -> Vec<Node<'_>> {
    let b = s.as_bytes();
    let len = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = b[i..].iter().position(|&c| c == b'<') {
        i += p;
        if b[i..].starts_with(b"<!--") {
            i = if b[i + 4..].starts_with(b">") {
                i + 5
            } else if b[i + 4..].starts_with(b"->") {
                i + 6
            } else {
                comment_end(b, i + 4)
            };
            continue;
        }
        let Some(&next) = b.get(i + 1) else {
            break;
        };
        if next == b'/' && b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) {
            i = parse_tag(s, i + 2).1;
            continue;
        }
        if matches!(next, b'!' | b'?' | b'/') {
            i = b[i + 1..]
                .iter()
                .position(|&c| c == b'>')
                .map_or(len, |p| i + 1 + p + 1);
            continue;
        }
        if !next.is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let (tag, after) = parse_tag(s, i + 1);
        i = after;
        if RAW_TEXT.contains(&tag.name.as_str()) {
            let end = if tag.name == "script" {
                find_script_end(b, i)
            } else {
                find_end_tag(b, i, &tag.name)
            };
            let text = &s[i..end];
            let name = tag.name.clone();
            out.push(Node::Tag(tag));
            match name.as_str() {
                "script" => out.push(Node::Script(text)),
                "style" => out.push(Node::Style(text)),
                _ => {}
            }
            i = end;
        } else {
            out.push(Node::Tag(tag));
        }
    }
    out
}

fn parse_tag(s: &str, from: usize) -> (Tag, usize) {
    let b = s.as_bytes();
    let len = b.len();
    let mut j = from;
    while j < len && !is_space(b[j]) && b[j] != b'/' && b[j] != b'>' {
        j += 1;
    }
    let name = s[from..j].to_ascii_lowercase();
    let mut attrs: Vec<Attr> = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    loop {
        while j < len && (is_space(b[j]) || b[j] == b'/') {
            j += 1;
        }
        if j >= len {
            break;
        }
        if b[j] == b'>' {
            j += 1;
            break;
        }
        let start = j;
        j += 1;
        while j < len && !is_space(b[j]) && !matches!(b[j], b'/' | b'>' | b'=') {
            j += 1;
        }
        let attr_name = s[start..j].to_ascii_lowercase();
        let mut k = j;
        while k < len && is_space(b[k]) {
            k += 1;
        }
        let mut value = "";
        if k < len && b[k] == b'=' {
            k += 1;
            while k < len && is_space(b[k]) {
                k += 1;
            }
            if k < len && (b[k] == b'"' || b[k] == b'\'') {
                let quote = b[k];
                let vstart = k + 1;
                let vend = b[vstart..]
                    .iter()
                    .position(|&c| c == quote)
                    .map_or(len, |p| vstart + p);
                value = &s[vstart..vend];
                k = (vend + 1).min(len);
            } else {
                let vstart = k;
                while k < len && !is_space(b[k]) && b[k] != b'>' {
                    k += 1;
                }
                value = &s[vstart..k];
            }
            j = k;
        }
        if names.insert(attr_name.clone()) {
            let (value, certain) = decode_attribute(value);
            attrs.push(Attr {
                name: attr_name,
                value,
                certain,
            });
        }
    }
    (Tag { name, attrs }, j)
}

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" | "AMP" => '&',
        "lt" | "LT" => '<',
        "gt" | "GT" => '>',
        "quot" | "QUOT" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "colon" => ':',
        "sol" => '/',
        "period" => '.',
        "num" => '#',
        "quest" => '?',
        "equals" => '=',
        "percnt" => '%',
        "commat" => '@',
        "lpar" => '(',
        "rpar" => ')',
        "Tab" => '\t',
        "NewLine" => '\n',
        _ => return None,
    })
}

const LEGACY_WITHOUT_SEMICOLON: [&str; 8] = ["amp", "AMP", "lt", "LT", "gt", "GT", "quot", "QUOT"];

const C1_REPLACEMENTS: [u32; 32] = [
    0x20ac, 0x81, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039,
    0x0152, 0x8d, 0x017d, 0x8f, 0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014,
    0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x9d, 0x017e, 0x0178,
];

pub(super) fn decode_attribute(s: &str) -> (String, bool) {
    if !s.contains('&') {
        return (s.to_string(), true);
    }
    let mut out = String::with_capacity(s.len());
    let mut certain = true;
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let tail = &rest[p + 1..];
        match decode_one(tail) {
            Decoded::Char(c, used) => {
                out.push(c);
                rest = &tail[used..];
            }
            Decoded::Literal => {
                out.push('&');
                rest = tail;
            }
            Decoded::Unknown => {
                certain = false;
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    (out, certain)
}

enum Decoded {
    Char(char, usize),
    Literal,
    Unknown,
}

fn numeric(code: u32) -> char {
    let code = match code {
        0x80..=0x9f => C1_REPLACEMENTS[(code - 0x80) as usize],
        0 => 0xfffd,
        _ => code,
    };
    char::from_u32(code).unwrap_or('\u{fffd}')
}

fn decode_one(tail: &str) -> Decoded {
    let b = tail.as_bytes();
    if b.first() == Some(&b'#') {
        let (radix, start) = match b.get(1) {
            Some(b'x' | b'X') => (16, 2),
            _ => (10, 1),
        };
        let digits = b[start..]
            .iter()
            .take_while(|c| c.is_ascii_hexdigit() && (radix == 16 || c.is_ascii_digit()))
            .count();
        if digits == 0 {
            return Decoded::Literal;
        }
        let code = u32::from_str_radix(&tail[start..start + digits], radix).unwrap_or(0xfffd);
        let mut used = start + digits;
        if b.get(used) == Some(&b';') {
            used += 1;
        }
        return Decoded::Char(numeric(code), used);
    }
    let n = b.iter().take_while(|c| c.is_ascii_alphanumeric()).count();
    if n == 0 {
        return Decoded::Literal;
    }
    let name = &tail[..n];
    let next = b.get(n).copied();
    if next == Some(b';') {
        return match named_entity(name) {
            Some(c) => Decoded::Char(c, n + 1),
            None => Decoded::Unknown,
        };
    }
    if next == Some(b'=') {
        return Decoded::Literal;
    }
    if let Some(prefix) = LEGACY_WITHOUT_SEMICOLON
        .iter()
        .find(|legacy| name.starts_with(**legacy))
    {
        if name.len() == prefix.len() {
            return Decoded::Char(named_entity(prefix).unwrap_or('&'), n);
        }
        return Decoded::Literal;
    }
    // Some legacy names (e.g. `&copy`) decode without `;` even in attributes, and the full table is not kept here.
    Decoded::Unknown
}
