use std::collections::{BTreeMap, HashSet};
use std::io::Read;

use anyhow::{Context, Result};

use crate::format::format_bytes;
use crate::ipfs::SiteListing;

mod css;
mod html;
mod js;
#[cfg(test)]
mod tests;

pub const LINK_SCAN_MAX_FILE: u64 = 4 << 20;
pub const LISTED_LINKS: usize = 5;
pub const SITE_GUIDE_URL: &str =
    "https://github.com/amane-katagiri/swing/blob/main/docs/site-guide.md";

const MAX_SHOWN: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkKind {
    Reserved,
    RootRelative,
    Broken,
    InsecureScript,
    PostForm,
    Worker,
    InsecureRequest,
    External,
    OwnSite,
}

impl LinkKind {
    pub fn name(self) -> &'static str {
        match self {
            LinkKind::Reserved => "reserved",
            LinkKind::RootRelative => "root_relative",
            LinkKind::Broken => "broken",
            LinkKind::InsecureScript => "insecure_script",
            LinkKind::PostForm => "post_form",
            LinkKind::Worker => "worker",
            LinkKind::InsecureRequest => "insecure_request",
            LinkKind::External => "external",
            LinkKind::OwnSite => "own_site",
        }
    }

    pub fn breaks(self) -> bool {
        matches!(
            self,
            LinkKind::Reserved
                | LinkKind::RootRelative
                | LinkKind::Broken
                | LinkKind::InsecureScript
        )
    }

    fn label(self, redirects: bool) -> &'static str {
        match self {
            LinkKind::Reserved => "top-level ipfs/ipns name: 404 on swing's gateway",
            LinkKind::RootRelative => "starts with /: breaks on path gateways",
            LinkKind::Broken if redirects => "not in the site; _redirects may cover it",
            LinkKind::Broken => "not in the site",
            LinkKind::InsecureScript => "http:// script: blocked by the gateway's CSP",
            LinkKind::PostForm => "non-GET form: 405 on swing's gateway",
            LinkKind::Worker => "worker: blocked by the gateway's CSP",
            LinkKind::InsecureRequest => {
                "http:// or ws:// request: may be blocked by the gateway's CSP"
            }
            LinkKind::External => "loads from another host",
            LinkKind::OwnSite => "points to the original site",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinkFinding {
    pub file: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkReport {
    found: BTreeMap<LinkKind, Vec<LinkFinding>>,
    pub redirects: bool,
    pub skipped: Vec<String>,
}

impl LinkReport {
    pub fn kinds(&self) -> impl Iterator<Item = (LinkKind, &[LinkFinding])> {
        self.found.iter().map(|(k, v)| (*k, v.as_slice()))
    }

    pub fn of(&self, kind: LinkKind) -> &[LinkFinding] {
        self.found.get(&kind).map_or(&[], Vec::as_slice)
    }

    pub fn total(&self) -> usize {
        self.found.values().map(Vec::len).sum()
    }

    pub fn blocks(&self, kind: LinkKind) -> bool {
        kind.breaks() && !(kind == LinkKind::Broken && self.redirects)
    }

    pub fn blocking(&self) -> usize {
        self.kinds()
            .filter(|(k, _)| self.blocks(*k))
            .map(|(_, v)| v.len())
            .sum()
    }

    pub fn lines(&self) -> Vec<String> {
        let skipped = (!self.skipped.is_empty()).then(|| {
            format!(
                "{} not read (over {})",
                count_files(self.skipped.len()),
                format_bytes(LINK_SCAN_MAX_FILE)
            )
        });
        let total = self.total();
        if total == 0 {
            return vec![match skipped {
                Some(s) => format!("\u{2713} links: no problems found; {s}"),
                None => "\u{2713} links: no problems found".to_string(),
            }];
        }
        let blocking = self.blocking();
        let mut lines = vec![if blocking > 0 {
            format!("! links: {total} found ({blocking} break on gateways)")
        } else {
            format!("! links: {total} found (may break on some gateways)")
        }];
        for (kind, findings) in self.kinds() {
            let label = kind.label(self.redirects);
            for f in findings.iter().take(LISTED_LINKS) {
                lines.push(if f.reference.is_empty() {
                    format!("    {} ({label})", f.file)
                } else {
                    format!("    {}: {} ({label})", f.file, f.reference)
                });
            }
            if findings.len() > LISTED_LINKS {
                lines.push(format!(
                    "    \u{2026} and {} more ({})",
                    findings.len() - LISTED_LINKS,
                    kind.name()
                ));
            }
        }
        if let Some(s) = skipped {
            lines.push(format!("    {s}"));
        }
        lines.push(format!("    see {SITE_GUIDE_URL}"));
        lines
    }

    pub fn abort_reason(&self) -> Option<String> {
        (self.blocking() > 0).then(|| {
            "links that break on gateways found: rename top-level ipfs/ipns entries, make links inside the site relative and point them at files in it, load scripts over https, or set --check-links / [publish].check_links to warn or off".to_string()
        })
    }

    fn push(
        &mut self,
        seen: &mut HashSet<(LinkKind, String, String)>,
        kind: LinkKind,
        file: &str,
        reference: &str,
    ) {
        if seen.insert((kind, file.to_string(), reference.to_string())) {
            self.found.entry(kind).or_default().push(LinkFinding {
                file: file.to_string(),
                reference: shorten(reference, MAX_SHOWN),
            });
        }
    }
}

fn count_files(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

fn shorten(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((cut, _)) => format!("{}\u{2026}", &s[..cut]),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileType {
    Html,
    Css,
    Js,
}

fn file_type(path: &str) -> Option<FileType> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" | "xhtml" => Some(FileType::Html),
        "css" => Some(FileType::Css),
        "js" | "mjs" => Some(FileType::Js),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Navigate,
    Resource,
    Script,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BaseDir {
    Dir(String),
    Elsewhere(Option<String>),
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn has_scheme(u: &str) -> bool {
    let Some((scheme, _)) = u.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

pub(crate) fn absolute_host(u: &str) -> Option<String> {
    let rest = if let Some(rest) = u.strip_prefix("//") {
        rest
    } else if starts_with_ci(u, "http://") {
        &u[7..]
    } else if starts_with_ci(u, "https://") {
        &u[8..]
    } else {
        return None;
    };
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next().unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
        {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'%'));
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, PartialEq, Eq)]
enum Resolved {
    Current,
    Escapes,
    Missing,
    Path { path: String, dir_only: bool },
}

fn resolve(reference: &str, dir: &str) -> Resolved {
    let end = reference.find(['?', '#']).unwrap_or(reference.len());
    let p = &reference[..end];
    if p.is_empty() {
        return Resolved::Current;
    }
    let rooted = p.starts_with('/');
    let mut segs: Vec<String> = if rooted {
        Vec::new()
    } else {
        dir.split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };
    let parts: Vec<&str> = p.split('/').collect();
    let mut dir_only = false;
    for (idx, raw) in parts.iter().enumerate() {
        if rooted && idx == 0 {
            continue;
        }
        let last = idx + 1 == parts.len();
        let seg = percent_decode(raw);
        if seg.contains('/') {
            return Resolved::Missing;
        }
        match seg.as_str() {
            "." => dir_only |= last,
            ".." => {
                if segs.pop().is_none() {
                    return Resolved::Escapes;
                }
                dir_only |= last;
            }
            "" if last => dir_only = true,
            _ => segs.push(seg),
        }
    }
    Resolved::Path {
        path: segs.join("/"),
        dir_only,
    }
}

struct Index {
    files: HashSet<String>,
    dirs: HashSet<String>,
}

impl Index {
    fn exists(&self, resolved: &Resolved) -> bool {
        match resolved {
            Resolved::Current => true,
            Resolved::Escapes | Resolved::Missing => false,
            Resolved::Path { path, .. } if path.is_empty() => true,
            Resolved::Path {
                path,
                dir_only: true,
            } => self.dirs.contains(path),
            Resolved::Path {
                path,
                dir_only: false,
            } => self.files.contains(path) || self.dirs.contains(path),
        }
    }
}

struct Scanner {
    index: Index,
    own_host: Option<String>,
    report: LinkReport,
    seen: HashSet<(LinkKind, String, String)>,
}

impl Scanner {
    fn new<'a>(paths: impl IntoIterator<Item = (&'a str, bool)>, own_url: Option<&str>) -> Self {
        let mut files = HashSet::new();
        let mut dirs = HashSet::new();
        for (path, is_dir) in paths {
            if is_dir {
                dirs.insert(path.to_string());
            } else {
                files.insert(path.to_string());
            }
        }
        let mut report = LinkReport {
            redirects: files.contains("_redirects"),
            ..Default::default()
        };
        let mut seen = HashSet::new();
        let mut top: Vec<&String> = files
            .iter()
            .chain(dirs.iter())
            .filter(|p| {
                !p.contains('/') && matches!(p.to_ascii_lowercase().as_str(), "ipfs" | "ipns")
            })
            .collect();
        top.sort();
        for name in top {
            report.push(&mut seen, LinkKind::Reserved, name, "");
        }
        Self {
            index: Index { files, dirs },
            own_host: own_url.and_then(absolute_host),
            report,
            seen,
        }
    }

    fn add(&mut self, kind: LinkKind, file: &str, reference: &str) {
        self.report.push(&mut self.seen, kind, file, reference);
    }

    // Breaking kinds are only reported when the scanner is sure of the URL, so an unknown character reference never stops a publish.
    fn reference(&mut self, role: Role, raw: &str, file: &str, base: &BaseDir, certain: bool) {
        let shown = raw.trim_matches(|c: char| c.is_ascii_whitespace());
        let cleaned: String = shown
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        let u = cleaned.as_str();
        if u.is_empty() || u.starts_with('#') {
            return;
        }
        if certain && role == Role::Script && starts_with_ci(u, "http://") {
            self.add(LinkKind::InsecureScript, file, shown);
        }
        if let Some(host) = absolute_host(u) {
            self.elsewhere(role, Some(&host), file, shown);
            return;
        }
        if has_scheme(u) {
            return;
        }
        let dir = match base {
            BaseDir::Elsewhere(host) => {
                self.elsewhere(role, host.as_deref(), file, shown);
                return;
            }
            BaseDir::Dir(dir) => dir.as_str(),
        };
        if !certain {
            return;
        }
        let rooted = u.starts_with('/');
        if rooted {
            self.add(LinkKind::RootRelative, file, shown);
        }
        if !self
            .index
            .exists(&resolve(u, if rooted { "" } else { dir }))
        {
            self.add(LinkKind::Broken, file, shown);
        }
    }

    fn elsewhere(&mut self, role: Role, host: Option<&str>, file: &str, shown: &str) {
        let Some(host) = host else {
            return;
        };
        let own = self.own_host.as_deref() == Some(host);
        match role {
            Role::Navigate | Role::Resource | Role::Script if own => {
                self.add(LinkKind::OwnSite, file, shown)
            }
            Role::Resource | Role::Script => self.add(LinkKind::External, file, shown),
            _ => {}
        }
    }

    fn css(&mut self, text: &str, file: &str, base: &BaseDir, certain: bool) {
        for url in css::urls(text) {
            self.reference(Role::Resource, &url, file, base, certain);
        }
    }

    fn js(&mut self, text: &str, file: &str) {
        let found = js::scan(text);
        for w in found.workers {
            self.add(LinkKind::Worker, file, &w);
        }
        for u in found.insecure {
            self.add(LinkKind::InsecureRequest, file, &u);
        }
    }

    fn base_dir(&mut self, nodes: &[html::Node<'_>], file: &str) -> BaseDir {
        let doc_dir = BaseDir::Dir(parent(file).to_string());
        let Some((href, certain)) = nodes.iter().find_map(|n| match n {
            html::Node::Tag(t) if t.name == "base" => t.attr_with_certainty("href"),
            _ => None,
        }) else {
            return doc_dir;
        };
        let shown = href.trim_matches(|c: char| c.is_ascii_whitespace());
        let cleaned: String = shown
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        let href = cleaned.as_str();
        if href.is_empty() {
            return doc_dir;
        }
        if let Some(host) = absolute_host(href) {
            self.reference(Role::Navigate, shown, file, &doc_dir, certain);
            return BaseDir::Elsewhere(Some(host));
        }
        if has_scheme(href) || !certain {
            return BaseDir::Elsewhere(None);
        }
        let rooted = href.starts_with('/');
        if rooted {
            self.add(LinkKind::RootRelative, file, shown);
        }
        let BaseDir::Dir(dir) = &doc_dir else {
            unreachable!()
        };
        let from = if rooted { "" } else { dir.as_str() };
        match resolve(href, from) {
            Resolved::Current => doc_dir,
            Resolved::Escapes => {
                self.add(LinkKind::Broken, file, shown);
                BaseDir::Elsewhere(None)
            }
            Resolved::Missing => BaseDir::Elsewhere(None),
            Resolved::Path { path, dir_only } => BaseDir::Dir(if dir_only {
                path
            } else {
                parent(&path).to_string()
            }),
        }
    }

    fn html(&mut self, text: &str, file: &str) {
        let nodes = html::parse(text);
        let base = self.base_dir(&nodes, file);
        let mut script_is_js = false;
        for node in &nodes {
            match node {
                html::Node::Tag(tag) => {
                    if tag.name == "script" {
                        script_is_js = is_js_type(tag.attr("type"));
                    }
                    self.tag(tag, file, &base);
                }
                html::Node::Script(text) if script_is_js => self.js(text, file),
                html::Node::Script(_) => {}
                html::Node::Style(text) => self.css(text, file, &base, true),
            }
        }
    }

    fn tag(&mut self, tag: &html::Tag, file: &str, base: &BaseDir) {
        if let Some((style, certain)) = tag.attr_with_certainty("style") {
            self.css(style, file, base, certain);
        }
        let name = tag.name.as_str();
        let check = |this: &mut Self, role: Role, attr: &str| {
            if let Some((value, certain)) = tag.attr_with_certainty(attr) {
                this.reference(role, value, file, base, certain);
            }
        };
        match name {
            "a" | "area" => check(self, Role::Navigate, "href"),
            "link" => {
                if let Some(role) = link_role(tag.attr("rel")) {
                    check(self, role, "href");
                }
            }
            "script" => check(self, Role::Script, "src"),
            "img" | "iframe" | "frame" | "video" | "audio" | "source" | "track" | "embed" => {
                check(self, Role::Resource, "src")
            }
            "input"
                if tag
                    .attr("type")
                    .is_some_and(|t| t.trim().eq_ignore_ascii_case("image")) =>
            {
                check(self, Role::Resource, "src")
            }
            "object" => check(self, Role::Resource, "data"),
            _ => {}
        }
        if matches!(name, "img" | "source")
            && let Some((srcset, certain)) = tag.attr_with_certainty("srcset")
        {
            for url in srcset_urls(srcset) {
                self.reference(Role::Resource, url, file, base, certain);
            }
        }
        if name == "video" {
            check(self, Role::Resource, "poster");
        }
        let (method, action) = match name {
            "form" => ("method", "action"),
            "button" | "input" => ("formmethod", "formaction"),
            _ => return,
        };
        if let Some(m) = tag.attr(method) {
            let m = m.trim().to_ascii_lowercase();
            if !m.is_empty() && m != "get" && m != "dialog" {
                self.add(
                    LinkKind::PostForm,
                    file,
                    &format!("<{name} {method}=\"{m}\">"),
                );
            }
        }
        if let Some(a) = tag.attr(action) {
            let a = a.trim();
            if starts_with_ci(a, "http://") {
                self.add(LinkKind::InsecureRequest, file, a);
            }
        }
    }
}

fn is_js_type(t: Option<&str>) -> bool {
    let Some(t) = t else {
        return true;
    };
    let t = t
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    t.is_empty() || t == "module" || t.contains("javascript") || t.contains("ecmascript")
}

const RESOURCE_RELS: [&str; 10] = [
    "stylesheet",
    "icon",
    "apple-touch-icon",
    "apple-touch-icon-precomposed",
    "mask-icon",
    "manifest",
    "preload",
    "modulepreload",
    "prefetch",
    "prerender",
];

fn link_role(rel: Option<&str>) -> Option<Role> {
    let rel = rel.unwrap_or_default().to_ascii_lowercase();
    let tokens: Vec<&str> = rel.split_ascii_whitespace().collect();
    if tokens.contains(&"canonical") {
        return None;
    }
    Some(if tokens.iter().any(|t| RESOURCE_RELS.contains(t)) {
        Role::Resource
    } else {
        Role::Other
    })
}

fn srcset_urls(srcset: &str) -> Vec<&str> {
    let b = srcset.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut url = &srcset[start..i];
        if url.is_empty() {
            break;
        }
        let mut descriptors = "";
        if url.ends_with(',') {
            url = url.trim_end_matches(',');
        } else {
            let desc_start = i;
            let mut in_parens = false;
            while i < b.len() {
                match b[i] {
                    b'(' => in_parens = true,
                    b')' => in_parens = false,
                    b',' if !in_parens => break,
                    _ => {}
                }
                i += 1;
            }
            descriptors = &srcset[desc_start..i];
        }
        if valid_descriptors(descriptors) {
            out.push(url);
        }
    }
    out
}

fn valid_descriptors(descriptors: &str) -> bool {
    let (mut w, mut x, mut h) = (false, false, false);
    for token in descriptors.split_ascii_whitespace() {
        let Some(unit) = token.chars().last() else {
            continue;
        };
        let number = &token[..token.len() - unit.len_utf8()];
        let integer = !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit());
        match unit.to_ascii_lowercase() {
            'w' if !w && !x && integer && number.parse::<u64>().is_ok_and(|n| n > 0) => w = true,
            'h' if !h && !x && integer && number.parse::<u64>().is_ok_and(|n| n > 0) => h = true,
            'x' if !x
                && !w
                && !h
                && !number.is_empty()
                && number.bytes().all(|c| {
                    c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')
                })
                && number.parse::<f64>().is_ok_and(|d| d >= 0.0) =>
            {
                x = true
            }
            _ => return false,
        }
    }
    !h || w
}

fn read_limited(file: &crate::ipfs::SiteFile<'_>) -> Result<Option<String>> {
    let opened = file
        .open()
        .with_context(|| format!("reading {} for the link check", file.path()))?;
    let mut buf = Vec::new();
    opened
        .take(LINK_SCAN_MAX_FILE + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading {} for the link check", file.path()))?;
    if buf.len() as u64 > LINK_SCAN_MAX_FILE {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

pub fn scan(site: &SiteListing, own_url: Option<&str>) -> Result<LinkReport> {
    let entries = site.entries();
    let mut scanner = Scanner::new(
        entries.iter().map(|e| (e.path.as_str(), e.size.is_none())),
        own_url,
    );
    for file in site.files() {
        let Some(kind) = file_type(file.path()) else {
            continue;
        };
        if file.size() > LINK_SCAN_MAX_FILE {
            scanner.report.skipped.push(file.path().to_string());
            continue;
        }
        let Some(text) = read_limited(&file)? else {
            scanner.report.skipped.push(file.path().to_string());
            continue;
        };
        scanner.text(kind, file.path(), &text);
    }
    Ok(scanner.report)
}

impl Scanner {
    fn text(&mut self, kind: FileType, path: &str, text: &str) {
        match kind {
            FileType::Html => self.html(text, path),
            FileType::Css => {
                let base = BaseDir::Dir(parent(path).to_string());
                self.css(text, path, &base, true);
            }
            FileType::Js => self.js(text, path),
        }
    }
}

pub async fn scan_async(
    site: SiteListing,
    own_url: Option<String>,
) -> Result<(SiteListing, LinkReport)> {
    tokio::task::spawn_blocking(move || {
        let report = scan(&site, own_url.as_deref());
        report.map(|r| (site, r))
    })
    .await
    .context("link check task panicked")?
}
