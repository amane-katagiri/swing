use std::fmt;

use tracing::field::{Field, Visit};
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::format::Writer;

use crate::format::Sanitized;

pub struct SanitizedFields;

impl<'writer> FormatFields<'writer> for SanitizedFields {
    fn format_fields<R: RecordFields>(&self, writer: Writer<'writer>, fields: R) -> fmt::Result {
        let mut visitor = FieldVisitor {
            writer,
            first: true,
            result: Ok(()),
        };
        fields.record(&mut visitor);
        visitor.result
    }
}

struct FieldVisitor<'writer> {
    writer: Writer<'writer>,
    first: bool,
    result: fmt::Result,
}

impl FieldVisitor<'_> {
    fn write(&mut self, field: &Field, value: fmt::Arguments<'_>) {
        if self.result.is_err() || field.name().starts_with("log.") {
            return;
        }
        let separator = if self.first { "" } else { " " };
        self.first = false;
        let text = Sanitized(value);
        self.result = if field.name() == "message" {
            write!(self.writer, "{separator}{text}")
        } else {
            write!(self.writer, "{separator}{}={text}", field.name())
        };
    }
}

impl Visit for FieldVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.write(field, format_args!("{value}"));
        } else {
            self.write(field, format_args!("{value:?}"));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.write(field, format_args!("{value:?}"));
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn logged(emit: impl FnOnce()) -> String {
        let buffer = Buffer::default();
        let sink = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .fmt_fields(SanitizedFields)
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_writer(move || sink.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, emit);
        String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn untrusted_text_stays_on_one_line_without_escapes() {
        let remote = "bad\n ERROR forged\r\u{1b}[2J\u{202e}x";
        let out = logged(|| {
            tracing::warn!(error = %remote, count = 2, quoted = ?remote, "relay said {remote}");
        });
        assert_eq!(out.lines().count(), 1, "{out:?}");
        assert!(!out.contains('\u{1b}') && !out.contains('\r') && !out.contains('\u{202e}'));
        assert!(
            out.contains("relay said bad  ERROR forged  [2Jx"),
            "{out:?}"
        );
        assert!(
            out.contains(" error=bad  ERROR forged  [2Jx count=2 quoted=\"bad\\n"),
            "{out:?}"
        );
    }

    #[test]
    fn string_fields_are_quoted_and_the_message_is_not() {
        let out = logged(|| tracing::info!(reason = "too big", "skip"));
        assert!(out.ends_with("INFO skip reason=\"too big\"\n"), "{out:?}");
    }
}
