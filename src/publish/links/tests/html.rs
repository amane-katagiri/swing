use super::*;

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
fn markup_inside_templates_never_stops_a_publish() {
    let report = html(
        &[],
        "index.html",
        r#"<template><img src="{{src}}"><template><a href="/x/${u}">x</a></template>
           <div style="background:url(/t.png)"></div><style>a{background:url(/s.png)}</style>
           <base href="https://cdn.example/"></template><img src="/after.png">"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/after.png"]);
    assert_eq!(refs(&report, LinkKind::Broken), ["/after.png"]);
}

#[test]
fn only_css_typed_style_elements_are_read() {
    let report = html(
        &[],
        "index.html",
        r#"<style type="text/x-template">a{background:url(/tpl.png)}</style>
           <style type="TEXT/CSS">a{background:url(/css.png)}</style>
           <style type="">a{background:url(/empty.png)}</style>"#,
        None,
    );
    assert_eq!(
        refs(&report, LinkKind::RootRelative),
        ["/css.png", "/empty.png"]
    );
}

#[test]
fn unfinished_tags_cdata_and_plaintext_follow_the_browser() {
    let report = html(
        &[],
        "index.html",
        r#"<img src="/ok.png"><a href="/cut"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/ok.png"]);
    let report = html(
        &[],
        "index.html",
        r#"<a href="/x.png">x</a><img src="/y.png"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/x.png"]);
    let report = html(
        &[],
        "index.html",
        r#"<svg><![CDATA[ a > b <img src="/in-cdata.png"> ]]></svg><plaintext><img src="/in-plaintext.png">"#,
        None,
    );
    assert_eq!(report.total(), 0, "{report:?}");
}

#[test]
fn short_comments_inside_a_script_leave_the_escaped_state() {
    for open in ["<!-->", "<!--->"] {
        let report = html(
            &[],
            "index.html",
            &format!("<script>{open}<script></script><img src=\"/after.png\">"),
            None,
        );
        assert_eq!(
            refs(&report, LinkKind::RootRelative),
            ["/after.png"],
            "{open}"
        );
    }
}

#[test]
fn srcset_units_are_lower_case_without_a_plus() {
    let report = html(
        &["ok.png"],
        "index.html",
        r#"<img srcset="/plus.png +1x, /upper.png 2X, /wide.png 100W, /ok.png 3x, ok.png 1e1x">"#,
        None,
    );
    assert_eq!(refs(&report, LinkKind::RootRelative), ["/ok.png"]);
}
