use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use super::time::format_unix_timestamp;
use super::{MirrorListView, MirrorSet, SiteRow, SitesView, npub};
use crate::config::Config;
use crate::dashboard::dto;
use crate::format::sanitize_display_text;
use crate::nostr::FOLLOW_SET_KIND;
use crate::{nostr, replicas};

pub(super) fn print_mirror_set(mirror_set_name: &str, set: &MirrorSet) {
    println!("Mirror set: {mirror_set_name} (kind {FOLLOW_SET_KIND})");
    if let Some(title) = set.title() {
        println!(
            "Title: {}",
            sanitize_display_text(title, MAX_MESSAGE_DISPLAY_CHARS)
        );
    }
    let pubkeys = set.pubkeys();
    println!("{} pubkey(s):", pubkeys.len());
    for pk in pubkeys {
        println!("  {}  {}", npub(&pk), pk.to_hex());
    }
}

pub(super) fn print_mirror_list(config: &Config, view: &MirrorListView) {
    if let Some(note) = view.note {
        println!("{note}");
    }
    match &view.set {
        Some(set) => print_mirror_set(&config.nostr.mirror_set, set),
        None => println!("(no follow set found)"),
    }
}

pub(super) fn print_relay_results_dto(results: &[dto::RelayResultDto]) {
    for r in results {
        nostr::print_relay_line(&r.relay, r.ok);
    }
}

pub(super) fn print_pubkeys_dto(pubkeys: &[dto::PubkeyDto]) {
    println!("{} pubkey(s):", pubkeys.len());
    for pk in pubkeys {
        println!("  {}  {}", pk.npub, pk.pubkey);
    }
}

pub(super) fn print_mirror_change_dto(
    change: &dto::MirrorChangeDto,
    unchanged_label: &str,
    changed_label: &str,
    requires_follow_set: bool,
) {
    if requires_follow_set && !change.follow_set_found {
        println!("(no follow set found); no changes");
        return;
    }
    if let Some(note) = &change.note {
        println!("{note}");
    }
    for pk in &change.unchanged {
        println!("{unchanged_label}: {} ({})", pk.npub, pk.pubkey);
    }
    if change.changed.is_empty() {
        println!("no changes; not publishing");
        return;
    }
    for pk in &change.changed {
        println!("{changed_label}: {} ({})", pk.npub, pk.pubkey);
    }
    println!();
    println!("Nostr");
    print_relay_results_dto(&change.relays);
    println!();
    print_pubkeys_dto(&change.members);
}

// The declared `size` is only a claim, so it is parenthesized rather than shown bare.
pub(super) fn format_size_column(row: &SiteRow) -> String {
    match (row.stored_size, row.size) {
        (Some(s), _) => s.to_string(),
        (None, Some(s)) => format!("({s})"),
        (None, None) => "-".to_string(),
    }
}

pub(super) fn format_site_line(row: &SiteRow, status: &str) -> String {
    format!(
        "  d={:<24} cid={:<62} url={:<32} size={:<12} created_at={:<25} nip05={:<14} replicas={:<4} [{}]",
        row.d,
        row.cid,
        row.url.clone().unwrap_or_else(|| "-".to_string()),
        format_size_column(row),
        format_unix_timestamp(row.created_at),
        row.nip05.as_deref().unwrap_or("-"),
        row.replicas
            .map_or_else(|| "-".to_string(), replicas::format_replica_counts),
        status
    )
}

pub(super) const MAX_MESSAGE_DISPLAY_CHARS: usize = 200;

pub(super) fn format_title_line(title: &str) -> String {
    format!(
        "    title: {}",
        sanitize_display_text(title, MAX_MESSAGE_DISPLAY_CHARS)
    )
}

pub(super) fn format_message_line(message: &str) -> String {
    format!(
        "    message: {}",
        sanitize_display_text(message, MAX_MESSAGE_DISPLAY_CHARS)
    )
}

pub(super) fn print_account_header(pubkey_hex: &str, suffix: &str) -> Result<PublicKey> {
    let pk = PublicKey::from_hex(pubkey_hex).context("parsing pubkey")?;
    println!("{} ({}){suffix}", npub(&pk), pubkey_hex);
    Ok(pk)
}

pub(super) fn print_sites(view: &SitesView) -> Result<()> {
    if let Some(note) = view.follow_note {
        println!("{note}");
    }
    if !view.follow_set_found {
        println!("(no follow set found)");
    } else if view.accounts.is_empty() {
        println!("(follow set is empty)");
    }
    if let Some(err) = &view.replicas_error {
        println!("(fetching replica reports failed: {err})");
    }

    for account in &view.accounts {
        print_account_header(&account.pubkey.to_hex(), "")?;
        if account.sites.is_empty() {
            println!("  (no site events)");
            continue;
        }
        for site in &account.sites {
            let status = if site.stored { "stored" } else { "not stored" };
            println!("{}", format_site_line(site, status));
            if let Some(title) = &site.title {
                println!("{}", format_title_line(title));
            }
            if let Some(message) = &site.message {
                println!("{}", format_message_line(message));
            }
        }
    }

    if view.unfollowed.is_empty() {
        return Ok(());
    }
    println!();
    if view.remove_on_unfollow && !view.follow_set_found {
        println!(
            "Unfollowed but still stored (no follow set found, so the agent keeps them until one is; if you changed the key or mirror_set, change it back to keep them or add someone to the new set to remove them):"
        );
    } else if view.remove_on_unfollow {
        println!("Unfollowed but still stored (the agent removes them on its next poll):");
    } else {
        println!(
            "Unfollowed but still stored (kept because remove_on_unfollow is false; set it to true to remove them):"
        );
    }
    for account in &view.unfollowed {
        print_account_header(&account.pubkey.to_hex(), " [unfollowed]")?;
        for site in &account.sites {
            println!("{}", format_site_line(site, "unfollowed"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_line_neutralizes_control_chars_and_truncates() {
        assert_eq!(
            format_message_line("Add posts\n\u{1b}[31mred"),
            "    message: Add posts  [31mred"
        );
        assert_eq!(
            format_title_line("My\u{202E}etiS\u{200B}\u{E0041}"),
            "    title: MyetiS"
        );
        let long = "あ".repeat(MAX_MESSAGE_DISPLAY_CHARS + 1);
        let line = format_message_line(&long);
        assert!(line.ends_with("あ\u{2026}"));
        assert_eq!(
            line.chars().count(),
            "    message: ".len() + MAX_MESSAGE_DISPLAY_CHARS + 1
        );
    }

    fn site_row(size: Option<u64>, stored_size: Option<u64>) -> SiteRow {
        SiteRow {
            d: "x.example".to_string(),
            cid: "bafy".to_string(),
            url: None,
            size,
            stored_size,
            stored_at: stored_size.map(|_| 0),
            created_at: 0,
            title: None,
            message: None,
            nip05: None,
            replicas: None,
            stored: stored_size.is_some(),
        }
    }

    #[test]
    fn format_size_column_prefers_measured_over_declared() {
        assert_eq!(format_size_column(&site_row(Some(999), Some(123))), "123");
    }

    #[test]
    fn format_size_column_parenthesizes_declared_when_not_stored() {
        assert_eq!(format_size_column(&site_row(Some(456), None)), "(456)");
    }

    #[test]
    fn format_size_column_is_dash_when_neither_is_known() {
        assert_eq!(format_size_column(&site_row(None, None)), "-");
    }
}
