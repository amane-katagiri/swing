use std::time::Instant;

use super::*;

#[test]
fn inline_scripts_report_workers_and_insecure_requests() {
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
    assert_finds(
        "inline script",
        &report,
        &[
            (LinkKind::Worker, &["serviceWorker.register", "new Worker("]),
            (LinkKind::InsecureRequest, &["http://api.example/data"]),
        ],
    );
}

#[test]
fn js_files_report_workers_and_insecure_requests() {
    use LinkKind::*;
    let continuation = |newline: &str| {
        format!("const s = \"hello\\{newline}new Worker(foo)\";\nnew SharedWorker(x);")
    };
    let (lf, crlf, cr) = (continuation("\n"), continuation("\r\n"), continuation("\r"));
    let cases: &[(&str, &str, Expect)] = &[
        (
            "regex, template and escaped strings",
            r##"const re = /"http:\/\/regex.example"/;
           const t = `ws://socket.example/${path}/x ${ {a: "http://nested.example"}.a }`;
           new SharedWorker(url); x = a / b / c; s = 'http:\/\/escaped.example/a';"##,
            &[
                (Worker, &["new SharedWorker("]),
                (
                    InsecureRequest,
                    &[
                        "ws://socket.example/",
                        "http://nested.example",
                        "http://escaped.example/a",
                    ],
                ),
            ],
        ),
        (
            "string continuation with \\n",
            &lf,
            &[(Worker, &["new SharedWorker("])],
        ),
        (
            "string continuation with \\r\\n",
            &crlf,
            &[(Worker, &["new SharedWorker("])],
        ),
        (
            "string continuation with \\r",
            &cr,
            &[(Worker, &["new SharedWorker("])],
        ),
        (
            "namespace and placeholder urls are not requests",
            r#"const a = "http://json-schema.org/draft-07/schema#"; new URL(p, "http://n");
           fetch("http://localhost:3000/api"); fetch("http://api.example/x");"#,
            &[(
                InsecureRequest,
                &["http://localhost:3000/api", "http://api.example/x"],
            )],
        ),
        (
            "workers behind global objects",
            "new self.Worker(u); new window . SharedWorker(u); new globalThis.Worker(u); new foo.Worker(u);",
            &[(Worker, &["new Worker(", "new SharedWorker("])],
        ),
    ];
    for (case, text, expect) in cases {
        let mut s = scanner(&[], None);
        s.text(FileType::Js, "a.js", text);
        assert_finds(case, &s.report, expect);
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
    let started = Instant::now();
    s.text(FileType::Js, "a.js", &text);
    assert!(
        started.elapsed() < LINEAR_SCAN_BUDGET,
        "{:?}",
        started.elapsed()
    );
    let found = s.report.of(LinkKind::InsecureRequest);
    assert_eq!(found.len(), 1);
    assert!(found[0].reference.ends_with('\u{2026}'));
}
