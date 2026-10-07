use std::fmt::{self, Write as _};

const BYTE_UNITS: [(u64, &str); 4] = [
    (1u64 << 40, "TiB"),
    (1u64 << 30, "GiB"),
    (1u64 << 20, "MiB"),
    (1u64 << 10, "KiB"),
];

pub fn format_bytes(n: u64) -> String {
    for &(factor, unit) in &BYTE_UNITS {
        if n < factor {
            continue;
        }
        if n.is_multiple_of(factor) {
            return format!("{} {unit}", n / factor);
        }
        if (u128::from(n) * 10).is_multiple_of(u128::from(factor)) {
            return format!("{:.1} {unit}", n as f64 / factor as f64);
        }
    }
    format!("{n} B")
}

pub fn format_bytes_approx(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64;
    let mut unit = "B";
    for &(_, name) in BYTE_UNITS.iter().rev() {
        if unit != "B" && value < 1023.95 {
            break;
        }
        value /= 1024.0;
        unit = name;
    }
    let text = format!("{value:.1}");
    format!("{} {unit}", text.strip_suffix(".0").unwrap_or(&text))
}

const DURATION_UNITS: [(u64, &str); 3] = [(86_400, "d"), (3_600, "h"), (60, "m")];

pub fn format_duration_secs(secs: u64) -> String {
    for &(factor, unit) in &DURATION_UNITS {
        if secs != 0 && secs.is_multiple_of(factor) {
            return format!("{}{unit}", secs / factor);
        }
    }
    format!("{secs}s")
}

pub struct Sanitized<T>(pub T);

struct SanitizingWriter<'a, 'b>(&'a mut fmt::Formatter<'b>);

impl fmt::Write for SanitizingWriter<'_, '_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if c.is_control() {
                self.0.write_char(' ')?;
            } else if !crate::nostr::is_unsafe_char(c) {
                self.0.write_char(c)?;
            }
        }
        Ok(())
    }
}

impl<T: fmt::Display> fmt::Display for Sanitized<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let alternate = f.alternate();
        let mut out = SanitizingWriter(f);
        if alternate {
            write!(out, "{:#}", self.0)
        } else {
            write!(out, "{}", self.0)
        }
    }
}

pub fn error_report(error: &anyhow::Error) -> String {
    let mut out = format!("Error: {}", Sanitized(error));
    let causes: Vec<_> = error.chain().skip(1).collect();
    if !causes.is_empty() {
        out.push_str("\n\nCaused by:");
    }
    for (i, cause) in causes.iter().enumerate() {
        if causes.len() == 1 {
            out.push_str(&format!("\n    {}", Sanitized(cause)));
        } else {
            out.push_str(&format!("\n    {i}: {}", Sanitized(cause)));
        }
    }
    let backtrace = error.backtrace();
    if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
        out.push_str(&format!("\n\nStack backtrace:\n{backtrace}"));
    }
    out
}

pub fn sanitize_display_text(text: &str, max_chars: usize) -> String {
    let cleaned: Vec<char> = Sanitized(text).to_string().chars().collect();
    let mut shown: String = cleaned.iter().take(max_chars).collect();
    if cleaned.len() > max_chars {
        shown.push('\u{2026}');
    }
    shown.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_prefers_the_largest_exact_unit() {
        assert_eq!(format_bytes(107_374_182_400), "100 GiB");
        assert_eq!(format_bytes(512 * (1u64 << 20)), "512 MiB");
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(300), "300 B");
    }

    #[test]
    fn format_bytes_approx_rounds_to_one_decimal() {
        assert_eq!(format_bytes_approx(0), "0 B");
        assert_eq!(format_bytes_approx(1023), "1023 B");
        assert_eq!(format_bytes_approx(1024), "1 KiB");
        assert_eq!(format_bytes_approx(1_300_000), "1.2 MiB");
        assert_eq!(format_bytes_approx((1 << 20) - 10), "1 MiB");
        assert_eq!(format_bytes_approx(5 << 40), "5 TiB");
    }

    #[test]
    fn format_duration_secs_prefers_the_largest_exact_unit() {
        assert_eq!(format_duration_secs(300), "5m");
        assert_eq!(format_duration_secs(86_400), "1d");
        assert_eq!(format_duration_secs(7_200), "2h");
        assert_eq!(format_duration_secs(600), "10m");
        assert_eq!(format_duration_secs(30), "30s");
        assert_eq!(format_duration_secs(0), "0s");
    }

    #[test]
    fn sanitized_flattens_controls_and_drops_invisible_formatting() {
        let text = "a\nerror: forged\r\u{1b}[31mb\u{9b}c\u{202e}d\u{200b}e\t\u{7f}f";
        assert_eq!(Sanitized(text).to_string(), "a error: forged  [31mb cde  f");
        assert_eq!(Sanitized("  日本語 ok  ").to_string(), "  日本語 ok  ");
    }

    #[test]
    fn sanitized_keeps_the_alternate_form() {
        let error = anyhow::anyhow!("inner\nline").context("outer");
        assert_eq!(format!("{:#}", Sanitized(&error)), "outer: inner line");
        assert_eq!(format!("{}", Sanitized(&error)), "outer");
    }

    #[test]
    fn error_report_puts_each_cause_on_its_own_line() {
        let error = anyhow::anyhow!("relay said: ok\nError: forged\u{1b}[2J")
            .context("sending failed")
            .context("publishing example.com");
        assert_eq!(
            error_report(&error),
            "Error: publishing example.com\n\nCaused by:\n    0: sending failed\n    1: relay said: ok Error: forged [2J"
        );
        assert_eq!(
            error_report(&anyhow::anyhow!("a\rb").context("c")),
            "Error: c\n\nCaused by:\n    a b"
        );
        assert_eq!(error_report(&anyhow::anyhow!("plain")), "Error: plain");
    }

    #[test]
    fn sanitize_display_text_truncates_after_sanitizing() {
        assert_eq!(sanitize_display_text(" a\u{202e}b\nc d", 4), "ab \u{2026}");
    }
}
