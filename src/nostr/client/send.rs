use nostr_sdk::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaySendResult {
    pub relay: String,
    pub ok: bool,
    pub error: Option<String>,
}

pub fn relay_send_results(
    relays: &[String],
    output: &Output<EventId, EventSendStatus, String>,
) -> Vec<RelaySendResult> {
    relays
        .iter()
        .map(|relay_url| {
            let parsed = RelayUrl::parse(relay_url).ok();
            let ok = parsed
                .as_ref()
                .is_some_and(|u| output.success.contains_key(u));
            let error = if ok {
                None
            } else {
                parsed.as_ref().and_then(|u| output.failed.get(u)).cloned()
            };
            RelaySendResult {
                relay: relay_url.clone(),
                ok,
                error,
            }
        })
        .collect()
}

pub fn print_relay_line(relay: &str, ok: bool) {
    if ok {
        println!("  \u{2713} {relay}");
    } else {
        println!("  \u{2717} {relay}");
    }
}

const MAX_REJECTION_DISPLAY_CHARS: usize = 500;

pub fn with_rejection_hint(error: &str, hint: &str) -> String {
    let suffix = format!(" ({hint})");
    let ellipsis = 1;
    let budget = MAX_REJECTION_DISPLAY_CHARS.saturating_sub(suffix.chars().count() + ellipsis);
    format!(
        "{}{suffix}",
        crate::format::sanitize_display_text(error, budget)
    )
}

fn rejection_line(relay: &str, error: &str) -> String {
    format!(
        "  \u{2717} {relay}: {}",
        crate::format::sanitize_display_text(error, MAX_REJECTION_DISPLAY_CHARS)
    )
}

pub fn print_relay_send_result_lines(results: &[RelaySendResult]) {
    for result in results {
        match &result.error {
            Some(error) if !result.ok => println!("{}", rejection_line(&result.relay, error)),
            _ => print_relay_line(&result.relay, result.ok),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_lines_strip_terminal_controls_and_cap_the_length() {
        assert_eq!(
            rejection_line("wss://a", "blocked:\u{1b}[2J\u{202e} spam"),
            "  \u{2717} wss://a: blocked: [2J spam"
        );
        let long = "x".repeat(MAX_REJECTION_DISPLAY_CHARS + 10);
        assert_eq!(
            rejection_line("wss://a", &long).chars().count(),
            "  \u{2717} wss://a: ".chars().count() + MAX_REJECTION_DISPLAY_CHARS + 1
        );
    }

    #[test]
    fn a_hint_survives_the_rejection_line_cap() {
        let hint = "check your clock";
        let fits = "x".repeat(MAX_REJECTION_DISPLAY_CHARS - hint.len() - 4);
        assert_eq!(with_rejection_hint(&fits, hint), format!("{fits} ({hint})"));
        let long = "x".repeat(MAX_REJECTION_DISPLAY_CHARS * 4);
        let hinted = with_rejection_hint(&long, hint);
        assert_eq!(hinted.chars().count(), MAX_REJECTION_DISPLAY_CHARS);
        assert!(hinted.ends_with("\u{2026} (check your clock)"), "{hinted}");
        assert_eq!(
            rejection_line("wss://a", &hinted),
            format!("  \u{2717} wss://a: {hinted}")
        );
    }
}
