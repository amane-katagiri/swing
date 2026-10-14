use super::*;

#[test]
fn normalize_hostname_accepts_dns_names_only() {
    let long_label = "a".repeat(63);
    let accepted = format!("{long_label}.com");
    for (input, want) in [
        ("Ama.NE.jp", "ama.ne.jp"),
        ("example.com", "example.com"),
        ("sub.example.com", "sub.example.com"),
        (accepted.as_str(), accepted.as_str()),
        ("xn--eckwd4c7c.xn--zckzah", "xn--eckwd4c7c.xn--zckzah"),
    ] {
        assert_eq!(normalize_hostname(input).as_deref(), Some(want), "{input}");
    }
    let too_long_label = format!("{}.com", "a".repeat(64));
    let too_long_name = format!("{}.com", "a".repeat(250));
    for (case, input) in [
        ("no dot", "localhost"),
        ("scheme", "https://example.com"),
        ("path", "example.com/path"),
        ("port", "example.com:8080"),
        ("empty", ""),
        ("space", "exa mple.com"),
        ("underscore", "exa_mple.com"),
        ("empty label", "example..com"),
        ("ipv4 loopback", "127.0.0.1"),
        ("ipv4 private", "10.0.0.5"),
        ("ipv4 link-local", "169.254.169.254"),
        ("hex ipv4", "0x7f.1"),
        ("octal ipv4", "0177.0.0.1"),
        ("numeric tld", "example.123"),
        ("leading hyphen", "-example.com"),
        ("trailing hyphen", "example-.com"),
        ("label over 63", too_long_label.as_str()),
        ("name over 253", too_long_name.as_str()),
    ] {
        assert_eq!(normalize_hostname(input), None, "{case}: {input}");
    }
}

#[test]
fn public_ip_filter() {
    for ip in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "255.255.255.255",
        "198.18.0.1",
        "192.0.0.8",
        "224.0.0.1",
        "::1",
        "::",
        "fc00::1",
        "fd12::1",
        "fe80::1",
        "2001:db8::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "64:ff9b::7f00:1",
        "64:ff9b::a00:1",
        "64:ff9b:1::1",
        "2002:7f00:1::1",
        "2002:c0a8:101::1",
        "::127.0.0.1",
        "::10.0.0.1",
        "fec0::1",
        "feff::1",
        "2001::1",
        "2001:0:4136:e378:8000:63bf:3fff:fdd2",
    ] {
        assert!(!is_public_ip(ip.parse().unwrap()), "{ip} should be blocked");
    }
    for ip in [
        "1.1.1.1",
        "93.184.216.34",
        "2606:4700::1111",
        "::ffff:8.8.8.8",
        "64:ff9b::808:808",
        "2002:808:808::1",
        "::8.8.8.8",
    ] {
        assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be allowed");
    }
}

#[tokio::test]
async fn public_only_resolver_refuses_loopback_names() {
    let name: Name = "localhost".parse().unwrap();
    let Err(err) = PublicOnlyResolver.resolve(name).await else {
        panic!("localhost resolved to a public address");
    };
    assert_eq!(err.to_string(), "localhost has no public address");
}

#[tokio::test]
async fn public_only_resolver_resolves_only_public_ip_literals() {
    for (literal, public) in [
        ("10.0.0.5", false),
        ("192.168.1.1", false),
        ("169.254.169.254", false),
        ("::1", false),
        ("fc00::1", false),
        ("1.1.1.1", true),
        ("8.8.8.8", true),
    ] {
        let name: Name = literal.parse().unwrap();
        assert_eq!(
            PublicOnlyResolver.resolve(name).await.is_ok(),
            public,
            "{literal}"
        );
    }
}

#[test]
fn evaluate_body_matches_the_underscore_name() {
    for (case, body, pubkey, want) in [
        (
            "hex case ignored",
            r#"{"names":{"_":"ABCDEF0123"}}"#,
            "abcdef0123",
            "verified",
        ),
        (
            "different hex",
            r#"{"names":{"_":"abcdef0123"}}"#,
            "0000000000",
            "mismatch",
        ),
        (
            "no underscore name",
            r#"{"names":{"someone_else":"abcdef0123"}}"#,
            "abcdef0123",
            "mismatch",
        ),
        ("invalid json", "not json", "abcdef0123", "error"),
    ] {
        assert_eq!(evaluate_body(body, pubkey).as_state_str(), want, "{case}");
    }
}

#[test]
fn state_str_and_detail_roundtrip() {
    assert_eq!(VerificationResult::Verified.as_state_str(), "verified");
    assert_eq!(VerificationResult::Mismatch.as_state_str(), "mismatch");
    assert_eq!(
        VerificationResult::NotApplicable.as_state_str(),
        "not_applicable"
    );
    let err = VerificationResult::Error("boom".to_string(), ErrorCategory::InvalidResponse);
    assert_eq!(err.as_state_str(), "error");
    assert_eq!(err.detail(), Some("boom".to_string()));
    assert_eq!(VerificationResult::Verified.detail(), None);
}

#[test]
fn coarse_detail_hides_the_raw_message() {
    let err = VerificationResult::Error(
        "connection refused to 10.0.0.5:443".to_string(),
        ErrorCategory::Unreachable,
    );
    assert_eq!(err.coarse_detail(), Some("unreachable"));
    assert_eq!(
        VerificationResult::Error("deadline exceeded".to_string(), ErrorCategory::Timeout)
            .coarse_detail(),
        Some("timeout")
    );
    assert_eq!(VerificationResult::Verified.coarse_detail(), None);
    assert_eq!(VerificationResult::Mismatch.coarse_detail(), None);
    assert_eq!(VerificationResult::NotApplicable.coarse_detail(), None);
}

#[test]
fn classify_from_flags_prioritizes_timeout_over_connect() {
    assert_eq!(classify_from_flags(true, true), ErrorCategory::Timeout);
    assert_eq!(classify_from_flags(true, false), ErrorCategory::Timeout);
    assert_eq!(classify_from_flags(false, true), ErrorCategory::Unreachable);
    assert_eq!(
        classify_from_flags(false, false),
        ErrorCategory::InvalidResponse
    );
}

#[tokio::test]
async fn a_d_that_is_not_a_hostname_is_not_looked_up() {
    assert_eq!(
        HttpNip05Verifier::new().verify("my-blog", "abcdef").await,
        VerificationResult::NotApplicable
    );
}

async fn served(
    status: u16,
    headers: &'static [(&'static str, &'static str)],
    body: Vec<u8>,
    pubkey_hex: &str,
) -> VerificationResult {
    let router = axum::Router::new().route(
        "/.well-known/nostr.json",
        axum::routing::get(move || async move {
            let mut resp = axum::response::Response::new(axum::body::Body::from(body));
            *resp.status_mut() = axum::http::StatusCode::from_u16(status).unwrap();
            for (name, value) in headers {
                resp.headers_mut().insert(*name, value.parse().unwrap());
            }
            resp
        }),
    );
    let addr = crate::test_support::serve_router(router).await;
    let resp = HttpNip05Verifier::new()
        .http
        .get(format!("http://{addr}/.well-known/nostr.json?name=_"))
        .send()
        .await
        .unwrap();
    read_response(resp, pubkey_hex).await
}

fn invalid(result: VerificationResult) -> String {
    match result {
        VerificationResult::Error(message, ErrorCategory::InvalidResponse) => message,
        other => panic!("expected an invalid response, got {other:?}"),
    }
}

#[tokio::test]
async fn a_served_nostr_json_is_read_and_evaluated() {
    let body = br#"{"names":{"_":"abcdef"}}"#.to_vec();
    assert_eq!(
        served(200, &[], body.clone(), "abcdef").await,
        VerificationResult::Verified
    );
    assert_eq!(
        served(200, &[], body, "012345").await,
        VerificationResult::Mismatch
    );
}

#[tokio::test]
async fn non_success_and_redirects_are_not_followed() {
    let body = br#"{"names":{"_":"abcdef"}}"#.to_vec();
    assert!(invalid(served(404, &[], body.clone(), "abcdef").await).contains("404"));
    let redirected = served(
        302,
        &[("location", "https://example.com/.well-known/nostr.json")],
        body,
        "abcdef",
    )
    .await;
    assert!(invalid(redirected).contains("302"));
}

#[tokio::test]
async fn bodies_over_the_cap_or_not_utf8_are_refused() {
    let mut big = br#"{"names":{"_":"abcdef"}, "pad":""#.to_vec();
    big.resize(MAX_BODY_BYTES + 1, b' ');
    assert!(invalid(served(200, &[], big, "abcdef").await).contains("size cap"));

    let mut exact = br#"{"names":{"_":"abcdef"}}"#.to_vec();
    exact.resize(MAX_BODY_BYTES, b' ');
    assert_eq!(
        served(200, &[], exact, "abcdef").await,
        VerificationResult::Verified
    );

    let latin1 = b"{\"names\":{\"_\":\"abcdef\"},\"x\":\"\xff\"}".to_vec();
    assert!(invalid(served(200, &[], latin1, "abcdef").await).contains("UTF-8"));
}
