use super::*;

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
fn css_bad_strings_and_bad_urls_are_dropped() {
    let mut s = scanner(&["a.css"], None);
    s.text(
        FileType::Css,
        "a.css",
        "@import \"missing.css\n;\na{background:url(\"bad\n.png\")}\nb{background:url(a b.png)}\nc{background:url(a\"b.png)}\nd{background:url(a(b.png)}\ne{background:url(  /ok.png  )}\n@namespace svg url(/ns);",
    );
    assert_eq!(refs(&s.report, LinkKind::RootRelative), ["/ok.png"]);
}
