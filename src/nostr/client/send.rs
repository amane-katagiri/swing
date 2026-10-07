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

pub fn print_relay_send_result_lines(results: &[RelaySendResult]) {
    for result in results {
        print_relay_line(&result.relay, result.ok);
    }
}
