use super::*;

#[test]
fn css_urls_and_imports_are_read_like_a_browser() {
    use LinkKind::*;
    let continuation = |newline: &str| {
        format!(
            "a {{ background: url(\"images/fo\\{newline}o.png\") }} b {{ background: url(\"missing.png\") }}"
        )
    };
    let continued: Vec<(String, String)> = [
        ("\\n", "\n"),
        ("\\r\\n", "\r\n"),
        ("\\r", "\r"),
        ("\\f", "\x0c"),
    ]
    .iter()
    .map(|(name, newline)| {
        (
            format!("line continuation with {name}"),
            continuation(newline),
        )
    })
    .collect();
    let mut cases: Vec<(&str, &[&str], &str, &str, Expect)> = vec![
        (
            "urls with and without quotes and imports",
            &["css/site.css", "img/a.png", "css/b.css"],
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
            &[
                (Broken, &["missing.css", "../img/missing.png"]),
                (RootRelative, &["/img/a.png"]),
            ],
        ),
        (
            "imports after comments and escaped names",
            &["a.css"],
            "a.css",
            r#"@import/**/"/theme.css"; @import /* license */ "missing.css";
           a{background:u\72l(/image.png)} @\69mport "/other.css";
           b{background-image: myurl(/not-a-url.png)}"#,
            &[
                (RootRelative, &["/theme.css", "/image.png", "/other.css"]),
                (
                    Broken,
                    &["/theme.css", "missing.css", "/image.png", "/other.css"],
                ),
            ],
        ),
        (
            "bad strings and bad urls are dropped",
            &["a.css"],
            "a.css",
            "@import \"missing.css\n;\na{background:url(\"bad\n.png\")}\nb{background:url(a b.png)}\nc{background:url(a\"b.png)}\nd{background:url(a(b.png)}\ne{background:url(  /ok.png  )}\n@namespace svg url(/ns);",
            &[(RootRelative, &["/ok.png"])],
        ),
    ];
    for (name, text) in &continued {
        cases.push((
            name,
            &["css/a.css", "css/images/foo.png"],
            "css/a.css",
            text,
            &[(Broken, &["missing.png"])],
        ));
    }
    for (case, paths, file, text, expect) in cases {
        let mut s = scanner(paths, None);
        s.text(FileType::Css, file, text);
        assert_finds(case, &s.report, expect);
    }
}
