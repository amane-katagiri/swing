fn is_port(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

pub fn split_host_port(host_header: &str) -> (&str, Option<&str>) {
    let host_header = host_header.trim();
    if let Some(rest) = host_header.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((host, "")) => (host, None),
            // Anything after `]` that isn't `:<digits>` makes the header malformed; fall back to
            // the raw header so it can't coincidentally match a real host like `::1`.
            Some((host, after)) => match after.strip_prefix(':').filter(|p| is_port(p)) {
                Some(port) => (host, Some(port)),
                None => (host_header, None),
            },
            None => (host_header, None),
        };
    }
    match host_header.rsplit_once(':') {
        Some((host, port)) if is_port(port) => (host, Some(port)),
        _ => (host_header, None),
    }
}

pub fn extract_host(host_header: &str) -> String {
    split_host_port(host_header).0.to_ascii_lowercase()
}

pub fn is_loopback_name(host: &str) -> bool {
    ["localhost", "127.0.0.1", "::1"]
        .iter()
        .any(|name| host.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_names_match_regardless_of_case() {
        for name in ["localhost", "LocalHost", "127.0.0.1", "::1"] {
            assert!(is_loopback_name(name), "{name}");
        }
        for name in ["localhost.example", "127.0.0.2", "example.com"] {
            assert!(!is_loopback_name(name), "{name}");
        }
    }

    #[test]
    fn extract_host_strips_port_when_present() {
        assert_eq!(extract_host("127.0.0.1:8082"), "127.0.0.1");
        assert_eq!(extract_host("127.0.0.1"), "127.0.0.1");
        assert_eq!(extract_host("Localhost:8082"), "localhost");
    }

    #[test]
    fn extract_host_handles_ipv6_brackets() {
        assert_eq!(extract_host("[::1]:8082"), "::1");
        assert_eq!(extract_host("[::1]"), "::1");
    }

    #[test]
    fn extract_host_handles_edge_cases() {
        assert_eq!(extract_host("localhost"), "localhost");
        assert_eq!(extract_host("[::1"), "[::1");
        assert_eq!(
            extract_host("evil.com:8082@localhost"),
            "evil.com:8082@localhost"
        );
    }

    #[test]
    fn extract_host_rejects_junk_after_the_bracketed_host() {
        assert_ne!(extract_host("[::1]xyz"), "::1");
        assert_ne!(extract_host("[::1]:8082xyz"), "::1");
        assert_ne!(extract_host("[::1]:"), "::1");
        assert_ne!(extract_host("[::1]:abc"), "::1");
        assert_ne!(extract_host("[example.com]junk"), "example.com");
        assert_eq!(extract_host("[::1]:8082"), "::1");
        assert_eq!(extract_host("[::1]"), "::1");
    }

    #[test]
    fn extract_host_rejects_non_digit_ports() {
        assert_ne!(extract_host("localhost:abc"), "localhost");
    }

    #[test]
    fn split_host_port_returns_the_port() {
        assert_eq!(
            split_host_port("example.com:8081"),
            ("example.com", Some("8081"))
        );
        assert_eq!(split_host_port("[::1]:8082"), ("::1", Some("8082")));
        assert_eq!(split_host_port("example.com"), ("example.com", None));
    }
}
