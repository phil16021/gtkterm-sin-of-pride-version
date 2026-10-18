//! Output snapshots: the terminal view in every mode and every log format. A change here changes
//! what users see or what their tools parse; review it with `cargo insta review`.
#![allow(clippy::unwrap_used)]

use std::fmt::Write as _;

use gtkterm_protocols::logfmt::{LogFormat, LogMeta, LogRecord, footer, header, record};
use gtkterm_protocols::view::{ViewFormatter, ViewMode, ViewSettings};

/// Text, a line break, control and high bytes, and a line longer than one dump row.
const SAMPLE: &[u8] =
    b"AT+GMR\r\nOK\r\n\x00\x01\x7f\x80\xfe\xff{P0|T|23.5}\nthe quick brown fox jumps over the lazy dog";

/// Control characters and invalid UTF-8 as `\u{..}`; line breaks stay.
fn visible(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '\n' || !(c.is_control() || c == char::REPLACEMENT_CHARACTER) {
                c.to_string()
            } else {
                c.escape_unicode().to_string()
            }
        })
        .collect()
}

#[test]
fn view_modes() {
    let mut out = String::new();
    for mode in ViewMode::ALL {
        for show_index in [false, true] {
            let mut f = ViewFormatter::new(ViewSettings {
                mode,
                show_index,
                hex_chars: 16,
                columns: 80,
            });
            // Two chunks: the line state carries over.
            let (a, b) = SAMPLE.split_at(13);
            let mut bytes = f.format(a);
            bytes.extend(f.format_stamped(b, Some("12:00:00.000")));
            let text = visible(&String::from_utf8_lossy(&bytes).replace('\r', ""));
            let _ = writeln!(out, "=== {} (index {show_index}) ===\n{text}\n", mode.label());
        }
    }
    insta::assert_snapshot!(out);
}

#[test]
fn log_formats() {
    let meta = LogMeta {
        start_time: "2026-10-18T12:00:00+02:00".into(),
        port: "/dev/ttyUSB0 115200-8-N-1".into(),
        profile: "lab \"bench\" <1>".into(),
    };
    let records = [
        LogRecord {
            time: "2026-10-18T12:00:00.125+02:00",
            rx: false,
            data: b"AT\r",
        },
        LogRecord {
            time: "2026-10-18T12:00:00.250+02:00",
            rx: true,
            data: b"OK\r\n\x00\xff<&>\"",
        },
    ];
    let mut out = String::new();
    for format in LogFormat::ALL {
        let mut bytes = header(format, &meta).into_bytes();
        for (i, r) in records.iter().enumerate() {
            if format.logs_tx() || r.rx {
                bytes.extend(record(format, r, i == 0 || !format.logs_tx()));
            }
        }
        bytes.extend_from_slice(footer(format).as_bytes());
        let _ = writeln!(
            out,
            "=== {format:?} ===\n{}\n",
            visible(&String::from_utf8_lossy(&bytes))
        );
    }
    insta::assert_snapshot!(out);
}
