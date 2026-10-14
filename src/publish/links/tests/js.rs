use super::*;

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
fn a_long_string_of_urls_scans_in_linear_time() {
    let mut text = String::from("const s = \"");
    while text.len() < LINK_SCAN_MAX_FILE as usize - 16 {
        text.push_str("http://a.b/");
    }
    text.push_str("\";");
    let mut s = scanner(&[], None);
    s.text(FileType::Js, "a.js", &text);
    let found = s.report.of(LinkKind::InsecureRequest);
    assert_eq!(found.len(), 1);
    assert!(found[0].reference.ends_with('\u{2026}'));
}

#[test]
fn namespace_and_placeholder_urls_are_not_requests() {
    let mut s = scanner(&[], None);
    s.text(
        FileType::Js,
        "a.js",
        r#"const a = "http://json-schema.org/draft-07/schema#"; new URL(p, "http://n");
           fetch("http://localhost:3000/api"); fetch("http://api.example/x");"#,
    );
    assert_eq!(
        refs(&s.report, LinkKind::InsecureRequest),
        ["http://localhost:3000/api", "http://api.example/x"]
    );
}

#[test]
fn workers_behind_global_objects_are_found() {
    let mut s = scanner(&[], None);
    s.text(
        FileType::Js,
        "a.js",
        "new self.Worker(u); new window . SharedWorker(u); new globalThis.Worker(u); new foo.Worker(u);",
    );
    assert_eq!(
        refs(&s.report, LinkKind::Worker),
        ["new Worker(", "new SharedWorker("]
    );
}
