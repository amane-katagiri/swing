#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct JsFindings {
    pub workers: Vec<String>,
    pub insecure: Vec<String>,
}

const REGEX_AFTER_WORDS: [&str; 14] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
];

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn skip_space(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn word_at(s: &str, i: usize) -> (&str, usize) {
    let b = s.as_bytes();
    let mut j = i;
    while j < b.len() && is_word(b[j]) {
        j += 1;
    }
    (&s[i..j], j)
}

enum TemplateStop {
    End(usize),
    Expr(usize),
}

fn template_part(b: &[u8], from: usize) -> (usize, TemplateStop) {
    let mut j = from;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'`' => return (j, TemplateStop::End(j + 1)),
            b'$' if b.get(j + 1) == Some(&b'{') => return (j, TemplateStop::Expr(j + 2)),
            _ => j += 1,
        }
    }
    (b.len(), TemplateStop::End(b.len()))
}

pub(super) fn scan(s: &str) -> JsFindings {
    let b = s.as_bytes();
    let len = b.len();
    let mut findings = JsFindings::default();
    let mut strings: Vec<&str> = Vec::new();
    let mut templates: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut last: u8 = 0;
    let mut last_word = "";
    let mut i = 0;
    let mut template_from: Option<usize> = None;
    loop {
        if let Some(from) = template_from.take() {
            let (end, stop) = template_part(b, from);
            strings.push(&s[from.min(len)..end.min(len)]);
            match stop {
                TemplateStop::End(next) => {
                    i = next;
                    last = b'"';
                }
                TemplateStop::Expr(next) => {
                    templates.push(depth);
                    i = next;
                    last = b'{';
                }
            }
        }
        if i >= len {
            break;
        }
        let c = b[i];
        match c {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = b[i..]
                    .iter()
                    .position(|&c| c == b'\n')
                    .map_or(len, |p| i + p);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = super::html::find(b, i + 2, b"*/").map_or(len, |p| p + 2);
            }
            b'/' => {
                let regex = matches!(
                    last,
                    0 | b'('
                        | b','
                        | b'='
                        | b':'
                        | b'['
                        | b'!'
                        | b'&'
                        | b'|'
                        | b'?'
                        | b'{'
                        | b'}'
                        | b';'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'%'
                        | b'<'
                        | b'>'
                        | b'~'
                        | b'^'
                ) || (last == b'a' && REGEX_AFTER_WORDS.contains(&last_word));
                if regex {
                    let mut j = i + 1;
                    let mut class = false;
                    while j < len {
                        match b[j] {
                            b'\\' => j += 1,
                            b'[' => class = true,
                            b']' => class = false,
                            b'/' if !class => break,
                            b'\n' => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    i = (j + 1).min(len);
                    last = b'"';
                } else {
                    i += 1;
                    last = b'/';
                }
            }
            b'\'' | b'"' => {
                let mut j = i + 1;
                while j < len && b[j] != c && b[j] != b'\n' && b[j] != b'\r' {
                    j += if b[j] != b'\\' {
                        1
                    } else if b[j + 1..].starts_with(b"\r\n") {
                        3
                    } else {
                        2
                    };
                }
                let end = j.min(len);
                strings.push(&s[i + 1..end]);
                i = (end + 1).min(len);
                last = b'"';
            }
            b'`' => template_from = Some(i + 1),
            b'{' => {
                depth += 1;
                i += 1;
                last = b'{';
            }
            b'}' => {
                if templates.last() == Some(&depth) {
                    templates.pop();
                    template_from = Some(i + 1);
                } else {
                    depth = depth.saturating_sub(1);
                    i += 1;
                    last = b'}';
                }
            }
            c if is_word(c) => {
                let (word, next) = word_at(s, i);
                detect_worker(s, word, next, &mut findings.workers);
                last = b'a';
                last_word = word;
                i = next;
            }
            c if c.is_ascii_whitespace() => i += 1,
            c => {
                last = c;
                i += 1;
            }
        }
    }
    for literal in strings {
        insecure_urls(literal, &mut findings.insecure);
    }
    findings
}

fn detect_worker(s: &str, word: &str, next: usize, out: &mut Vec<String>) {
    let b = s.as_bytes();
    match word {
        "new" => {
            let j = skip_space(b, next);
            let (name, after) = word_at(s, j);
            if matches!(name, "Worker" | "SharedWorker") {
                let k = skip_space(b, after);
                if b.get(k) == Some(&b'(') {
                    out.push(format!("new {name}("));
                }
            }
        }
        "serviceWorker" => {
            let mut j = skip_space(b, next);
            if b.get(j) == Some(&b'?') {
                j += 1;
            }
            if b.get(j) != Some(&b'.') {
                return;
            }
            let j = skip_space(b, j + 1);
            if word_at(s, j).0 == "register" {
                out.push("serviceWorker.register".to_string());
            }
        }
        _ => {}
    }
}

const URL_END: &[char] = &['"', '\'', '`', '<', '>', ')', '(', '\\', ' ', '{', '}'];

pub(super) fn insecure_urls(literal: &str, out: &mut Vec<String>) {
    let text = literal.replace("\\/", "/");
    let lower = text.to_ascii_lowercase();
    for scheme in ["http://", "ws://"] {
        let mut from = 0;
        while let Some(p) = lower[from..].find(scheme) {
            let at = from + p;
            from = at + scheme.len();
            if at > 0 && lower.as_bytes()[at - 1].is_ascii_alphanumeric() {
                continue;
            }
            let host = lower.as_bytes().get(from);
            if !host.is_some_and(|h| h.is_ascii_alphanumeric() || *h == b'[') {
                continue;
            }
            if lower[at..].starts_with("http://www.w3.org/") {
                continue;
            }
            let rest = &text[at..];
            let end = rest
                .find(|c: char| c.is_whitespace() || URL_END.contains(&c))
                .unwrap_or(rest.len());
            out.push(rest[..end].to_string());
            from = at + end.max(scheme.len());
        }
    }
}
