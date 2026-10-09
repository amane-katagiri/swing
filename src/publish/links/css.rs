fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n')
}

fn skip_space_and_comments(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && is_space(b[i]) {
            i += 1;
        }
        if b[i..].starts_with(b"/*") {
            i = super::html::find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
        } else {
            return i;
        }
    }
}

fn preprocess(s: &str) -> String {
    s.replace("\r\n", "\n").replace(['\r', '\x0c'], "\n")
}

fn escape_at(s: &str, i: usize) -> (Option<char>, usize) {
    let rest = &s[i + 1..];
    let hex: usize = rest
        .bytes()
        .take(6)
        .take_while(u8::is_ascii_hexdigit)
        .count();
    if hex > 0 {
        let code = u32::from_str_radix(&rest[..hex], 16).unwrap_or(0xfffd);
        let c = char::from_u32(code)
            .filter(|&c| c != '\0')
            .unwrap_or('\u{fffd}');
        let mut end = i + 1 + hex;
        if s.as_bytes().get(end).is_some_and(|b| is_space(*b)) {
            end += 1;
        }
        return (Some(c), end);
    }
    match rest.chars().next() {
        Some('\n') => (None, i + 2),
        Some(c) => (Some(c), i + 1 + c.len_utf8()),
        None => (None, i + 1),
    }
}

fn starts_name(s: &str, i: usize) -> bool {
    let b = s.as_bytes();
    match b.get(i) {
        Some(b'\\') => b.get(i + 1).is_some_and(|&n| n != b'\n'),
        Some(_) => s[i..].chars().next().is_some_and(is_name),
        None => false,
    }
}

fn name_at(s: &str, mut i: usize) -> (String, usize) {
    let b = s.as_bytes();
    let mut out = String::new();
    while i < b.len() {
        if b[i] == b'\\' {
            if b.get(i + 1) == Some(&b'\n') {
                break;
            }
            let (c, next) = escape_at(s, i);
            out.extend(c);
            i = next;
            continue;
        }
        let Some(c) = s[i..].chars().next().filter(|&c| is_name(c)) else {
            break;
        };
        out.push(c);
        i += c.len_utf8();
    }
    (out, i)
}

pub(super) fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw.as_bytes()[i] == b'\\' {
            let (c, next) = escape_at(raw, i);
            out.extend(c);
            i = next;
            continue;
        }
        let c = raw[i..].chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn string_at(s: &str, i: usize) -> (String, usize) {
    let b = s.as_bytes();
    let quote = b[i];
    let mut j = i + 1;
    while j < b.len() && b[j] != quote && b[j] != b'\n' {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    let end = j.min(b.len());
    let end = (0..=end)
        .rev()
        .find(|&e| s.is_char_boundary(e))
        .unwrap_or(i + 1);
    (unescape(&s[i + 1..end]), (end + 1).min(b.len()))
}

pub(super) fn urls(source: &str) -> Vec<String> {
    let text = preprocess(source);
    let s = text.as_str();
    let b = s.as_bytes();
    let len = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < len {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = super::html::find(b, i + 2, b"*/").map_or(len, |p| p + 2);
            }
            b'"' | b'\'' => i = string_at(s, i).1,
            b'@' if starts_name(s, i + 1) => {
                let (name, after) = name_at(s, i + 1);
                i = after;
                if name.eq_ignore_ascii_case("import") {
                    let j = skip_space_and_comments(b, after);
                    if j < len && (b[j] == b'"' || b[j] == b'\'') {
                        let (value, next) = string_at(s, j);
                        out.push(value);
                        i = next;
                    } else {
                        i = j;
                    }
                }
            }
            _ if starts_name(s, i) => {
                let (name, after) = name_at(s, i);
                i = after;
                if b.get(after) == Some(&b'(') && name.eq_ignore_ascii_case("url") {
                    let mut j = after + 1;
                    while j < len && is_space(b[j]) {
                        j += 1;
                    }
                    if j < len && (b[j] == b'"' || b[j] == b'\'') {
                        let (value, next) = string_at(s, j);
                        out.push(value);
                        i = next;
                    } else {
                        let start = j;
                        while j < len && b[j] != b')' {
                            j += if b[j] == b'\\' { 2 } else { 1 };
                        }
                        let end = j.min(len);
                        out.push(unescape(s[start..end].trim()));
                        i = end;
                    }
                }
            }
            _ => i += s[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}
