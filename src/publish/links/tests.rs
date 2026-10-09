use super::*;

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
fn srcset_candidates_are_each_checked() {
    let report = html(
        &["a.png", "b.png"],
        "index.html",
        r##"<img srcset="a.png 1x, b.png 2x,c.png 3x">
           <picture><source srcset="a.png, /d.png 100w"></picture>
           <img srcset="data:image/png;base64,AAA= 1x, b.png 2x">"##,
        None,
    );
    assert_eq!(refs(&report, LinkKind::Broken), ["c.png", "/d.png"]);
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/d.png"]);
}

#[test]
fn css_urls_with_and_without_quotes_and_imports() {
    let mut s = scanner(&["css/site.css", "img/a.png", "css/b.css"], None);
    s.text(
        FileType::Css,
        "css/site.css",
        r##"@import "b.css"; @import url(missing.css);
           /* url(commented.png) */
           body { background: url(../img/a.png) }
           .a { background: url( "../img/missing.png" ) }
           .b { background: URL('/img/a.png') }
           .c { content: "url(not-a-url.png)" }
           .d { background: url(data:image/png;base64,AAA) }
           .e { filter: url(#f) }
           .f { background-image: url(../img/a\2e png) }"##,
    );
    assert_eq!(
        refs(&s.report, LinkKind::Broken),
        ["missing.css", "../img/missing.png"]
    );
    assert_eq!(refs(&s.report, LinkKind::RootRelative), ["/img/a.png"]);
}

#[test]
fn style_attributes_and_elements_are_read_as_css() {
    let report = html(
        &["index.html"],
        "index.html",
        r##"<style>body { background: url("/bg.png") }</style>
           <div style="background: url(&quot;x.png&quot;)"></div>"##,
        None,
    );
    assert_eq!(refs(&report, LinkKind::Broken), ["/bg.png", "x.png"]);
}

#[test]
fn comments_and_raw_text_are_not_markup() {
    let report = html(
        &["index.html"],
        "index.html",
        r##"<!-- <a href="/commented">x</a> -->
           <script>document.write('<a href="/in-script">x</a>');</script>
           <textarea><img src="/in-textarea.png"></textarea>
           <title><a href="/in-title"></title>
           <!--> <a href="kept.html">after an abrupt comment</a>"##,
        None,
    );
    assert!(refs(&report, LinkKind::RootRelative).is_empty());
    assert_eq!(refs(&report, LinkKind::Broken), ["kept.html"]);
}

#[test]
fn attributes_quoted_unquoted_and_with_character_references() {
    let report = html(
        &["a&b.html", "q.html"],
        "index.html",
        "<a href=a&amp;b.html>1</a><a href='q.html?x=1&amp;y=2'>2</a>\
         <a\nhref = \"/x&#47;y\">3</a><A HREF=missing.html>4</A>\
         <a href=\"java&#x73;cript:alert(1)\">5</a><a href=\"mailto:a@example.com\">6</a>\
         <a href=\"&#x2F;z\">7</a>",
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/x/y", "/z"]);
    assert_eq!(
        refs(&report, LinkKind::Broken),
        ["/x/y", "missing.html", "/z"]
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
fn a_base_element_changes_resolution_and_a_root_base_is_flagged() {
    let report = html(
        &["index.html", "sub/a.html"],
        "index.html",
        r##"<base href="sub/"><a href="a.html">a</a><a href="b.html">b</a>"##,
        None,
    );
    assert_eq!(refs(&report, LinkKind::Broken), ["b.html"]);
    let report = html(
        &["index.html", "blog/a.html"],
        "blog/index.html",
        r##"<base href="/"><a href="blog/a.html">a</a>"##,
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/"]);
    assert!(refs(&report, LinkKind::Broken).is_empty());
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
fn insecure_scripts_break_and_external_resources_only_warn() {
    let report = html(
        &[],
        "index.html",
        r##"<script src="http://cdn.example/a.js"></script>
           <script src="https://cdn.example/b.js"></script>
           <link rel="stylesheet" href="//fonts.example/css">
           <link rel="preconnect" href="https://fonts.example">
           <iframe src="https://video.example/embed/1"></iframe>
           <a href="https://elsewhere.example/">a plain link</a>"##,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::InsecureScript),
        ["http://cdn.example/a.js"]
    );
    assert_eq!(
        refs(&report, LinkKind::External),
        [
            "http://cdn.example/a.js",
            "https://cdn.example/b.js",
            "//fonts.example/css",
            "https://video.example/embed/1"
        ]
    );
    assert_eq!(report.blocking(), 1);
}

#[test]
fn forms_that_do_not_get_and_insecure_actions_are_warnings() {
    let report = html(
        &[],
        "index.html",
        r##"<form method="POST" action="http://api.example/send"></form>
           <form method="get"></form><form method="dialog"></form><form></form>
           <button formmethod="post">send</button>"##,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::PostForm),
        ["<form method=\"post\">", "<button formmethod=\"post\">"]
    );
    assert_eq!(
        refs(&report, LinkKind::InsecureRequest),
        ["http://api.example/send"]
    );
    assert_eq!(report.blocking(), 0);
}

#[test]
fn inline_scripts_and_js_files_report_workers_and_insecure_requests() {
    let report = html(
        &[],
        "index.html",
        r##"<script>
             navigator.serviceWorker.register('sw.js');
             const w = new Worker ('w.js');
             fetch("http://api.example/data");
             const svg = "http://www.w3.org/2000/svg";
             // fetch("http://commented.example/")
             const prefix = "http://";
           </script>
           <script type="application/ld+json">{"@context": "http://schema.org"}</script>"##,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::Worker),
        ["serviceWorker.register", "new Worker("]
    );
    assert_eq!(
        refs(&report, LinkKind::InsecureRequest),
        ["http://api.example/data"]
    );

    let mut s = scanner(&[], None);
    s.text(
        FileType::Js,
        "app.mjs",
        r##"const re = /"http:\/\/regex.example"/;
           const t = `ws://socket.example/${path}/x ${ {a: "http://nested.example"}.a }`;
           new SharedWorker(url); x = a / b / c; s = 'http:\/\/escaped.example/a';"##,
    );
    assert_eq!(refs(&s.report, LinkKind::Worker), ["new SharedWorker("]);
    assert_eq!(
        refs(&s.report, LinkKind::InsecureRequest),
        [
            "ws://socket.example/",
            "http://nested.example",
            "http://escaped.example/a"
        ]
    );
}

#[test]
fn own_site_links_are_reported_except_canonical_and_meta() {
    let report = html(
        &[],
        "index.html",
        r##"<link rel="canonical" href="https://example.com/">
           <meta property="og:url" content="https://example.com/">
           <a href="https://Example.COM/about/">about</a>
           <img src="https://example.com/logo.png">
           <a href="https://other.example/">other</a>"##,
        Some("https://example.com/"),
    );
    assert_eq!(
        refs(&report, LinkKind::OwnSite),
        ["https://Example.COM/about/", "https://example.com/logo.png"]
    );
    assert!(refs(&report, LinkKind::External).is_empty());
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
fn an_external_base_keeps_root_relative_references_off_the_site() {
    let report = html(
        &["index.html"],
        "index.html",
        r#"<base href="https://cdn.example/assets/"><img src="/logo.png"><a href="x.html">x</a>"#,
        None,
    );
    assert!(refs(&report, LinkKind::RootRelative).is_empty());
    assert!(refs(&report, LinkKind::Broken).is_empty());
    assert_eq!(refs(&report, LinkKind::External), ["/logo.png"]);
}

#[test]
fn character_references_follow_the_attribute_rules() {
    let report = html(
        &["a&amp=b.html", "a&b.html", "\u{20ac}.html", "a&ampx.html"],
        "index.html",
        r#"<a href="a&amp=b.html">1</a><a href="a&AMP;b.html">2</a>
           <a href="&#128;.html">3</a><a href="a&ampx.html">4</a>"#,
        None,
    );
    assert_eq!(report.total(), 0, "{report:?}");
}

#[test]
fn unknown_character_references_never_stop_a_publish() {
    let report = html(
        &[],
        "index.html",
        r#"<a href="/x&hellip;y.html">1</a><img src="/x&copy.png"><a href="a&b.html">2</a>
           <script src="http://cdn.example/a&foo;.js"></script>
           <base href="sub&unknown;/">"#,
        None,
    );
    assert_eq!(report.blocking(), 0, "{report:?}");
    assert_eq!(
        refs(&report, LinkKind::External),
        ["http://cdn.example/a&foo;.js"]
    );
}

#[test]
fn tabs_and_newlines_inside_a_url_are_removed() {
    let report = html(
        &["img.png"],
        "index.html",
        "<img src=\"im&#10;g.png\"><a href=\"java&#9;script:void(0)\">x</a><img src=\"im\ng.png\">",
        None,
    );
    assert_eq!(report.total(), 0, "{report:?}");
}

#[test]
fn a_double_escaped_script_keeps_inner_tags_as_text() {
    let report = html(
        &[],
        "index.html",
        "<script><!--\nconst s = \"<script></script><img src='/ghost.png'>\";\n//-->\n</script><img src=\"/after.png\">",
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/after.png"]);
}

#[test]
fn quoted_attributes_on_end_tags_are_consumed() {
    let report = html(
        &[],
        "index.html",
        r#"<div></div data-x="><img src='/ghost.png'>"><style></style data-y="><img src='/g2.png'>">"#,
        None,
    );
    assert_eq!(report.total(), 0, "{report:?}");
}

#[test]
fn comments_closed_with_a_bang_end_there() {
    let report = html(
        &[],
        "index.html",
        r#"<!-- x --!><img src="/missing.png"><!----><img src="/b.png">"#,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::RootRelative),
        ["/missing.png", "/b.png"]
    );
}

#[test]
fn css_line_continuations_in_any_newline_form() {
    for newline in ["\n", "\r\n", "\r", "\x0c"] {
        let mut s = scanner(&["css/a.css", "css/images/foo.png"], None);
        s.text(
            FileType::Css,
            "css/a.css",
            &format!("a {{ background: url(\"images/fo\\{newline}o.png\") }} b {{ background: url(\"missing.png\") }}"),
        );
        assert_eq!(
            refs(&s.report, LinkKind::Broken),
            ["missing.png"],
            "{newline:?}"
        );
    }
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
fn css_imports_after_comments_and_escaped_names_are_read() {
    let mut s = scanner(&["a.css"], None);
    s.text(
        FileType::Css,
        "a.css",
        r#"@import/**/"/theme.css"; @import /* license */ "missing.css";
           a{background:u\72l(/image.png)} @\69mport "/other.css";
           b{background-image: myurl(/not-a-url.png)}"#,
    );
    assert_eq!(
        refs(&s.report, LinkKind::RootRelative),
        ["/theme.css", "/image.png", "/other.css"]
    );
    assert_eq!(
        refs(&s.report, LinkKind::Broken),
        ["/theme.css", "missing.css", "/image.png", "/other.css"]
    );
}

#[test]
fn invalid_srcset_candidates_are_dropped() {
    let report = html(
        &["ok.png"],
        "index.html",
        r#"<img src="ok.png" srcset="/unused.png -1x, ok.png 1x, /twice.png 1x 2x, /mixed.png 100w 1x, /h.png 10h, /w.png 0w, /wh.png 10w 20h">"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/wh.png"]);
}

#[test]
fn js_string_continuations_in_any_newline_form_stay_strings() {
    for newline in ["\n", "\r\n", "\r"] {
        let mut s = scanner(&[], None);
        s.text(
            FileType::Js,
            "a.js",
            &format!("const s = \"hello\\{newline}new Worker(foo)\";\nnew SharedWorker(x);"),
        );
        assert_eq!(
            refs(&s.report, LinkKind::Worker),
            ["new SharedWorker("],
            "{newline:?}"
        );
    }
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
fn many_distinct_attributes_scan_in_linear_time() {
    let mut text = String::from("<div");
    let mut i = 0;
    while text.len() < LINK_SCAN_MAX_FILE as usize - 16 {
        text.push_str(&format!(" a{i:06}"));
        i += 1;
    }
    text.push_str("><img src=\"/x.png\">");
    let report = html(&[], "index.html", &text, None);
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/x.png"]);
}

#[test]
fn a_long_string_of_urls_scans_in_linear_time() {
    let mut text = String::from("const s = \"");
    while text.len() < LINK_SCAN_MAX_FILE as usize - 16 {
        text.push_str("http://a/");
    }
    text.push_str("\";");
    let mut s = scanner(&[], None);
    s.text(FileType::Js, "a.js", &text);
    let found = s.report.of(LinkKind::InsecureRequest);
    assert_eq!(found.len(), 1);
    assert!(found[0].reference.ends_with('\u{2026}'));
}
