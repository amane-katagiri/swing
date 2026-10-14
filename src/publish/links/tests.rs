use super::*;

mod css;
mod html;
mod js;

fn scanner(paths: &[&str], own_url: Option<&str>) -> Scanner {
    let mut entries: Vec<(String, bool)> = Vec::new();
    for path in paths {
        let mut dir = String::new();
        let segments: Vec<&str> = path.split('/').collect();
        for seg in &segments[..segments.len() - 1] {
            if !dir.is_empty() {
                dir.push('/');
            }
            dir.push_str(seg);
            if !entries.iter().any(|(p, d)| *d && *p == dir) {
                entries.push((dir.clone(), true));
            }
        }
        entries.push((path.to_string(), false));
    }
    Scanner::new(entries.iter().map(|(p, d)| (p.as_str(), *d)), own_url)
}

fn html(paths: &[&str], file: &str, text: &str, own_url: Option<&str>) -> LinkReport {
    let mut s = scanner(paths, own_url);
    s.html(text, file);
    s.report
}

fn refs(report: &LinkReport, kind: LinkKind) -> Vec<&str> {
    report
        .of(kind)
        .iter()
        .map(|f| f.reference.as_str())
        .collect()
}

#[test]
fn relative_references_resolve_from_the_referring_file() {
    let paths = [
        "index.html",
        "blog/post.html",
        "blog/img/a.png",
        "css/site.css",
        "about/index.html",
    ];
    let report = html(
        &paths,
        "blog/post.html",
        r##"<a href="../index.html">home</a>
           <img src="img/a.png"><img src="./img/a.png?v=1#x">
           <link rel=stylesheet href="../css/site.css">
           <a href="../about/">about</a><a href="../about">about</a>
           <a href="">self</a><a href="?page=2">next</a><a href="#top">top</a>
           <img src="img/missing.png"><a href="../../outside.html">x</a>
           <a href="./">dir</a><a href="post.html/">not a dir</a>"##,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::Broken),
        ["img/missing.png", "../../outside.html", "post.html/"]
    );
    assert!(refs(&report, LinkKind::RootRelative).is_empty());
}

#[test]
fn root_relative_references_are_flagged_and_resolved_from_the_root() {
    let report = html(
        &["index.html", "css/site.css"],
        "index.html",
        r##"<link rel="stylesheet" href="/css/site.css"><script src="/js/app.js"></script>
           <a href="//cdn.example/x.js">protocol relative</a>"##,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::RootRelative),
        ["/css/site.css", "/js/app.js"]
    );
    assert_eq!(refs(&report, LinkKind::Broken), ["/js/app.js"]);
    assert!(report.blocking() > 0);
}

#[test]
fn percent_encoding_is_decoded_and_case_matters() {
    let report = html(
        &["a b.html", "日本.html", "Image.PNG"],
        "index.html",
        r##"<a href="a%20b.html">1</a><a href="%E6%97%A5%E6%9C%AC.html">2</a>
           <img src="image.png"><img src="Image.PNG"><a href="a%2Fb.html">3</a>
           <a href="%2e%2e/x">4</a>"##,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::Broken),
        ["image.png", "a%2Fb.html", "%2e%2e/x"]
    );
}

#[test]
fn schemes_fragments_and_empty_references_are_skipped() {
    let report = html(
        &[],
        "index.html",
        r##"<a href="mailto:x@example.com"></a><a href="tel:123"></a><a href="#x"></a>
           <a href=""></a><img src="data:image/gif;base64,R0"><a href="javascript:void(0)"></a>
           <img src="blob:abc">"##,
        None,
    );
    assert_eq!(report.total(), 0);
}

#[test]
fn reserved_top_level_names_are_flagged() {
    let s = scanner(
        &["IPFS/x.html", "ipns", "blog/ipfs/y.html", "ipfsx/z"],
        None,
    );
    assert_eq!(
        s.report
            .of(LinkKind::Reserved)
            .iter()
            .map(|f| f.file.as_str())
            .collect::<Vec<_>>(),
        ["IPFS", "ipns"]
    );
}

#[test]
fn redirects_turn_broken_into_a_warning() {
    let report = html(
        &["_redirects"],
        "index.html",
        r##"<a href="missing/">x</a><a href="/also-missing">y</a>"##,
        None,
    );
    assert!(report.redirects);
    assert_eq!(report.of(LinkKind::Broken).len(), 2);
    assert!(!report.blocks(LinkKind::Broken));
    assert_eq!(report.blocking(), 1);
    assert!(
        report
            .lines()
            .iter()
            .any(|l| l.contains("_redirects may cover it"))
    );
}

#[test]
fn findings_are_listed_once_per_file_and_capped_per_kind() {
    let mut text = String::new();
    for i in 0..8 {
        text.push_str(&format!("<img src=\"m{i}.png\"><img src=\"m{i}.png\">"));
    }
    let report = html(&[], "index.html", &text, None);
    assert_eq!(report.of(LinkKind::Broken).len(), 8);
    let lines = report.lines();
    assert_eq!(lines[0], "! links: 8 found (8 break on gateways)");
    assert_eq!(lines[1], "    index.html: m0.png (not in the site)");
    assert_eq!(lines[LISTED_LINKS + 1], "    \u{2026} and 3 more (broken)");
    assert_eq!(lines.last().unwrap(), &format!("    see {SITE_GUIDE_URL}"));
    assert!(report.abort_reason().unwrap().contains("--check-links"));
}

#[test]
fn skipped_files_and_one_guide_link_are_listed() {
    let mut report = html(
        &[],
        "index.html",
        r##"<img src="/a.png"><img src="https://cdn.example/b.png">"##,
        None,
    );
    report.skipped.push("big.html".into());
    let lines = report.lines();
    assert!(
        lines.contains(&"    1 file not read (over 4 MiB)".to_string()),
        "{lines:?}"
    );
    let sees: Vec<&String> = lines.iter().filter(|l| l.starts_with("    see ")).collect();
    assert_eq!(sees, [&format!("    see {SITE_GUIDE_URL}")]);
}

#[test]
fn a_clean_site_reports_no_problems() {
    let report = html(
        &["index.html", "a.html"],
        "index.html",
        r##"<a href="a.html">a</a>"##,
        None,
    );
    assert_eq!(report.lines(), ["\u{2713} links: no problems found"]);
    assert!(report.abort_reason().is_none());
}

#[test]
fn scan_reads_only_listed_text_files_within_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), br##"<img src="/a.png">"##).unwrap();
    std::fs::write(dir.path().join("PAGE.HTM"), br##"<img src="b.png">"##).unwrap();
    std::fs::write(dir.path().join("notes.txt"), br##"<img src="/c.png">"##).unwrap();
    let mut big = b"<img src=\"/d.png\">".to_vec();
    big.resize(LINK_SCAN_MAX_FILE as usize + 1, b' ');
    std::fs::write(dir.path().join("big.html"), &big).unwrap();
    let site = SiteListing::read(dir.path()).unwrap();
    std::fs::write(dir.path().join("late.html"), br##"<img src="/e.png">"##).unwrap();

    let report = scan(&site, None).unwrap();
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/a.png"]);
    assert_eq!(refs(&report, LinkKind::Broken), ["b.png", "/a.png"]);
    assert_eq!(report.skipped, ["big.html"]);
}

#[cfg(unix)]
#[test]
fn scan_refuses_a_file_replaced_after_listing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"<p>old</p>").unwrap();
    let site = SiteListing::read(dir.path()).unwrap();
    std::fs::write(dir.path().join("new.html"), b"new").unwrap();
    std::fs::rename(dir.path().join("new.html"), dir.path().join("index.html")).unwrap();
    let err = scan(&site, None).unwrap_err();
    assert!(format!("{err:#}").contains("changed after"), "{err:#}");
}

#[test]
fn resolve_handles_dots_and_trailing_slashes() {
    assert_eq!(resolve("?x", "a"), Resolved::Current);
    assert_eq!(resolve("../..", "a"), Resolved::Escapes);
    assert_eq!(
        resolve("../b/./c/", "a"),
        Resolved::Path {
            path: "b/c".into(),
            dir_only: true
        }
    );
    assert_eq!(
        resolve("/x/../y", "a"),
        Resolved::Path {
            path: "y".into(),
            dir_only: false
        }
    );
}

#[test]
fn absolute_host_strips_userinfo_ports_and_case() {
    assert_eq!(
        absolute_host("HTTPS://user@Example.com:8443/x").as_deref(),
        Some("example.com")
    );
    assert_eq!(absolute_host("//[::1]:80/").as_deref(), Some("::1"));
    assert_eq!(absolute_host("ftp://example.com"), None);
    assert_eq!(absolute_host("/x"), None);
}

#[test]
fn an_encoded_slash_does_not_match_a_nested_file() {
    let report = html(
        &["a/b.html"],
        "index.html",
        r#"<a href="a%2Fb.html">x</a><a href="a/b.html">y</a>"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::Broken), ["a%2Fb.html"]);
}

#[test]
fn long_references_are_compared_before_shortening() {
    let a = format!("{}1.png", "a".repeat(MAX_SHOWN));
    let b = format!("{}2.png", "a".repeat(MAX_SHOWN));
    let report = html(
        &[],
        "index.html",
        &format!(r#"<img src="{a}"><img src="{b}">"#),
        None,
    );
    let broken = report.of(LinkKind::Broken);
    assert_eq!(broken.len(), 2);
    assert!(broken[0].reference.ends_with('\u{2026}'));
}

#[test]
fn backslashes_are_read_as_slashes() {
    let report = html(
        &["images/a.png"],
        "index.html",
        r#"<img src="images\a.png"><img src="\\cdn.example\x.png"><a href="sub\missing.html">x</a>"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::Broken), ["sub\\missing.html"]);
    assert_eq!(
        refs(&report, LinkKind::External),
        ["\\\\cdn.example\\x.png"]
    );
}

#[test]
fn empty_segments_in_the_middle_are_skipped() {
    let report = html(
        &["images/a.png"],
        "index.html",
        r#"<img src="images//a.png"><img src="images///b.png">"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::Broken), ["images///b.png"]);
}

#[test]
fn xhtml_files_are_not_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("page.xhtml"),
        br#"<script src="a.js"/><img src="/x.png"/>"#,
    )
    .unwrap();
    let site = SiteListing::read(dir.path()).unwrap();
    assert_eq!(scan(&site, None).unwrap().total(), 0);
}

#[test]
fn external_references_are_grouped_across_files() {
    let mut s = scanner(&[], Some("https://example.com/"));
    for file in ["a.html", "b.html", "c.html"] {
        s.html(
            r#"<link rel="stylesheet" href="https://cdn.example/a.css"><a href="https://example.com/">home</a>"#,
            file,
        );
    }
    let external = s.report.of(LinkKind::External);
    assert_eq!(external.len(), 1);
    assert_eq!(external[0].file, "a.html");
    assert_eq!(external[0].files, 3);
    assert_eq!(s.report.of(LinkKind::OwnSite)[0].files, 3);
    assert!(
        s.report.lines().contains(
            &"    a.html and 2 other files: https://cdn.example/a.css (loads from another host)"
                .to_string()
        )
    );
}

#[test]
fn control_characters_are_shown_escaped() {
    let report = html(
        &[],
        "index.html",
        "<img src=\"/a&#27;b.png\"><img src=\"/c&#11;d.png\">",
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::RootRelative),
        ["/a\\u{1b}b.png", "/c\\u{b}d.png"]
    );
}

#[test]
fn findings_are_ordered_like_the_new_files_list() {
    let dir = tempfile::tempdir().unwrap();
    for (path, body) in [
        (
            "blog/2026/post.html",
            r#"<img src="/b26-1.png"><img src="/b26-2.png">"#,
        ),
        ("blog/index.html", r#"<img src="/b-1.png">"#),
        (
            "css/a.css",
            "a{background:url(/c-1.png)} b{background:url(/c-2.png)}",
        ),
        ("index.html", r#"<img src="/i-2.png"><img src="/i-1.png">"#),
        ("about.html", r#"<img src="/a-1.png">"#),
        (
            "x.html",
            r#"<link rel="stylesheet" href="https://cdn.example/s.css">"#,
        ),
        (
            "blog/x.html",
            r#"<link rel="stylesheet" href="https://cdn.example/s.css">"#,
        ),
    ] {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    let site = SiteListing::read(dir.path()).unwrap();
    let report = scan(&site, None).unwrap();
    let order: Vec<(&str, &str)> = report
        .of(LinkKind::RootRelative)
        .iter()
        .map(|f| (f.file.as_str(), f.reference.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("about.html", "/a-1.png"),
            ("index.html", "/i-2.png"),
            ("index.html", "/i-1.png"),
            ("blog/index.html", "/b-1.png"),
            ("blog/2026/post.html", "/b26-1.png"),
            ("blog/2026/post.html", "/b26-2.png"),
            ("css/a.css", "/c-1.png"),
            ("css/a.css", "/c-2.png"),
        ]
    );
    let lines = report.lines();
    let first = lines
        .iter()
        .position(|l| l.contains("starts with /"))
        .unwrap();
    assert_eq!(
        lines[first + LISTED_LINKS - 1],
        "    blog/2026/post.html: /b26-1.png (starts with /: breaks on path gateways)"
    );
    assert_eq!(
        lines[first + LISTED_LINKS],
        "    \u{2026} and 3 more (root_relative)"
    );
    let external = report.of(LinkKind::External);
    assert_eq!(external[0].file, "x.html");
    assert_eq!(external[0].files, 2);
}
