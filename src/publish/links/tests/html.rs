use std::time::Instant;

use super::*;

#[test]
fn srcset_candidates_are_each_checked() {
    use LinkKind::*;
    assert_html_cases(&[
        (
            "every candidate",
            &["a.png", "b.png"],
            r##"<img srcset="a.png 1x, b.png 2x,c.png 3x">
           <picture><source srcset="a.png, /d.png 100w"></picture>
           <img srcset="data:image/png;base64,AAA= 1x, b.png 2x">"##,
            &[(Broken, &["c.png", "/d.png"]), (RootRelative, &["/d.png"])],
        ),
        (
            "invalid candidates are dropped",
            &["ok.png"],
            r#"<img src="ok.png" srcset="/unused.png -1x, ok.png 1x, /twice.png 1x 2x, /mixed.png 100w 1x, /h.png 10h, /w.png 0w, /wh.png 10w 20h">"#,
            &[(RootRelative, &["/wh.png"])],
        ),
        (
            "units are lower case without a plus",
            &["ok.png"],
            r#"<img srcset="/plus.png +1x, /upper.png 2X, /wide.png 100W, /ok.png 3x, ok.png 1e1x">"#,
            &[(RootRelative, &["/ok.png"])],
        ),
    ]);
}

#[test]
fn html_is_tokenized_like_a_browser() {
    use LinkKind::*;
    let short_comment =
        |open: &str| format!("<script>{open}<script></script><img src=\"/after.png\">");
    let (short, shorter) = (short_comment("<!-->"), short_comment("<!--->"));
    assert_html_cases(&[
        (
            "style attributes and elements are read as css",
            &["index.html"],
            r##"<style>body { background: url("/bg.png") }</style>
           <div style="background: url(&quot;x.png&quot;)"></div>"##,
            &[(Broken, &["/bg.png", "x.png"])],
        ),
        (
            "comments and raw text are not markup",
            &["index.html"],
            r##"<!-- <a href="/commented">x</a> -->
           <script>document.write('<a href="/in-script">x</a>');</script>
           <textarea><img src="/in-textarea.png"></textarea>
           <title><a href="/in-title"></title>
           <!--> <a href="kept.html">after an abrupt comment</a>"##,
            &[(RootRelative, &[]), (Broken, &["kept.html"])],
        ),
        (
            "attributes quoted, unquoted and with character references",
            &["a&b.html", "q.html"],
            "<a href=a&amp;b.html>1</a><a href='q.html?x=1&amp;y=2'>2</a>\
             <a\nhref = \"/x&#47;y\">3</a><A HREF=missing.html>4</A>\
             <a href=\"java&#x73;cript:alert(1)\">5</a><a href=\"mailto:a@example.com\">6</a>\
             <a href=\"&#x2F;z\">7</a>",
            &[
                (RootRelative, &["/x/y", "/z"]),
                (Broken, &["/x/y", "missing.html", "/z"]),
            ],
        ),
        (
            "an external base keeps root-relative references off the site",
            &["index.html"],
            r#"<base href="https://cdn.example/assets/"><img src="/logo.png"><a href="x.html">x</a>"#,
            &[
                (RootRelative, &[]),
                (Broken, &[]),
                (External, &["/logo.png"]),
            ],
        ),
        (
            "character references follow the attribute rules",
            &["a&amp=b.html", "a&b.html", "\u{20ac}.html", "a&ampx.html"],
            r#"<a href="a&amp=b.html">1</a><a href="a&AMP;b.html">2</a>
           <a href="&#128;.html">3</a><a href="a&ampx.html">4</a>"#,
            &[],
        ),
        (
            "tabs and newlines inside a url are removed",
            &["img.png"],
            "<img src=\"im&#10;g.png\"><a href=\"java&#9;script:void(0)\">x</a><img src=\"im\ng.png\">",
            &[],
        ),
        (
            "a double-escaped script keeps inner tags as text",
            &[],
            "<script><!--\nconst s = \"<script></script><img src='/ghost.png'>\";\n//-->\n</script><img src=\"/after.png\">",
            &[(RootRelative, &["/after.png"])],
        ),
        (
            "quoted attributes on end tags are consumed",
            &[],
            r#"<div></div data-x="><img src='/ghost.png'>"><style></style data-y="><img src='/g2.png'>">"#,
            &[],
        ),
        (
            "comments closed with a bang end there",
            &[],
            r#"<!-- x --!><img src="/missing.png"><!----><img src="/b.png">"#,
            &[(RootRelative, &["/missing.png", "/b.png"])],
        ),
        (
            "markup inside templates never stops a publish",
            &[],
            r#"<template><img src="{{src}}"><template><a href="/x/${u}">x</a></template>
           <div style="background:url(/t.png)"></div><style>a{background:url(/s.png)}</style>
           <base href="https://cdn.example/"></template><img src="/after.png">"#,
            &[(RootRelative, &["/after.png"]), (Broken, &["/after.png"])],
        ),
        (
            "only css-typed style elements are read",
            &[],
            r#"<style type="text/x-template">a{background:url(/tpl.png)}</style>
           <style type="TEXT/CSS">a{background:url(/css.png)}</style>
           <style type="">a{background:url(/empty.png)}</style>"#,
            &[(RootRelative, &["/css.png", "/empty.png"])],
        ),
        (
            "an unfinished attribute value is dropped",
            &[],
            r#"<img src="/ok.png"><a href="/cut"#,
            &[(RootRelative, &["/ok.png"])],
        ),
        (
            "an unfinished tag is dropped",
            &[],
            r#"<a href="/x.png">x</a><img src="/y.png"#,
            &[(RootRelative, &["/x.png"])],
        ),
        (
            "cdata and plaintext are text",
            &[],
            r#"<svg><![CDATA[ a > b <img src="/in-cdata.png"> ]]></svg><plaintext><img src="/in-plaintext.png">"#,
            &[],
        ),
        (
            "<!--> inside a script leaves the escaped state",
            &[],
            &short,
            &[(RootRelative, &["/after.png"])],
        ),
        (
            "<!---> inside a script leaves the escaped state",
            &[],
            &shorter,
            &[(RootRelative, &["/after.png"])],
        ),
    ]);
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
fn many_distinct_attributes_scan_in_linear_time() {
    let mut text = String::from("<div");
    let mut i = 0;
    while text.len() < LINK_SCAN_MAX_FILE as usize - 16 {
        text.push_str(&format!(" a{i:06}"));
        i += 1;
    }
    text.push_str("><img src=\"/x.png\">");
    let started = Instant::now();
    let report = html(&[], "index.html", &text, None);
    assert!(
        started.elapsed() < LINEAR_SCAN_BUDGET,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/x.png"]);
}
