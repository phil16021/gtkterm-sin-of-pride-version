//! Receive-side text transform for the ASCII view: automatic CR/LF and line timestamps.
//!
//! * `auto_cr`: a LF that is not preceded by CR becomes CR LF (bare-LF devices).
//! * `auto_lf`: a CR that is not followed by LF becomes CR LF (bare-CR devices).
//!   Whether a CR is "bare" is only known when the next byte arrives, so the LF is
//!   inserted lazily, also across chunk boundaries.
//! * `timestamps`: every line, including the first one, starts with a timestamp prefix.
//!
//! The C version never tracked the previous CR (`cr_received` stayed 0), so auto-CR never
//! fired and auto-LF doubled CR in CR LF input; see ROADMAP defect B3.

use jiff::{Timestamp, tz::TimeZone};
use serde::{Deserialize, Serialize};

/// How line timestamps are rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampFormat {
    /// Local wall clock `HH:MM:SS.mmm`.
    #[default]
    LocalTime,
    /// Seconds since the session started, `+S.mmm`.
    Relative,
    /// RFC 3339 / ISO 8601 with offset, millisecond precision.
    Iso8601,
}

/// Render `ts` according to `format`. `start` is the session start for [`TimestampFormat::Relative`].
pub fn format_timestamp(format: TimestampFormat, ts: Timestamp, tz: &TimeZone, start: Timestamp) -> String {
    match format {
        TimestampFormat::LocalTime => ts.to_zoned(tz.clone()).strftime("%H:%M:%S%.3f").to_string(),
        TimestampFormat::Relative => {
            let ms = ts.as_millisecond().saturating_sub(start.as_millisecond()).max(0);
            format!("+{}.{:03}", ms / 1000, ms % 1000)
        }
        TimestampFormat::Iso8601 => ts
            .to_zoned(tz.clone())
            .strftime("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string(),
    }
}

/// Options of the transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransformOptions {
    /// Insert CR before a bare LF.
    pub auto_cr: bool,
    /// Insert LF after a bare CR.
    pub auto_lf: bool,
    /// Prefix each line with a timestamp.
    pub timestamps: bool,
    /// Timestamp rendering.
    pub timestamp_format: TimestampFormat,
}

/// Streaming transform state.
#[derive(Debug, Clone)]
pub struct Transform {
    options: TransformOptions,
    tz: TimeZone,
    session_start: Timestamp,
    at_line_start: bool,
    prev_cr: bool,
}

impl Transform {
    /// New transform. `session_start` anchors relative timestamps.
    pub fn new(options: TransformOptions, tz: TimeZone, session_start: Timestamp) -> Self {
        Self {
            options,
            tz,
            session_start,
            at_line_start: true,
            prev_cr: false,
        }
    }

    /// Current options.
    pub fn options(&self) -> TransformOptions {
        self.options
    }

    /// Replace options without losing line state.
    pub fn set_options(&mut self, options: TransformOptions) {
        self.options = options;
    }

    /// Forget line state (after clearing the screen).
    pub fn reset(&mut self) {
        self.at_line_start = true;
        self.prev_cr = false;
    }

    fn line_start(&mut self, out: &mut Vec<u8>, ts: Timestamp) {
        if self.at_line_start && self.options.timestamps {
            let stamp = format_timestamp(self.options.timestamp_format, ts, &self.tz, self.session_start);
            out.push(b'[');
            out.extend_from_slice(stamp.as_bytes());
            out.extend_from_slice(b"] ");
        }
        self.at_line_start = false;
    }

    /// Transform one received chunk that arrived at `ts`.
    pub fn apply(&mut self, data: &[u8], ts: Timestamp) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 32);
        for &byte in data {
            if self.prev_cr && byte != b'\n' && self.options.auto_lf {
                out.push(b'\n');
                self.at_line_start = true;
            }
            match byte {
                b'\n' => {
                    if !self.prev_cr && self.options.auto_cr {
                        out.push(b'\r');
                    }
                    out.push(b'\n');
                    self.at_line_start = true;
                }
                b'\r' => out.push(b'\r'),
                _ => {
                    self.line_start(&mut out, ts);
                    out.push(byte);
                }
            }
            self.prev_cr = byte == b'\r';
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(ms).unwrap()
    }

    fn run(opts: TransformOptions, chunks: &[&[u8]]) -> Vec<u8> {
        let mut t = Transform::new(opts, TimeZone::UTC, ts(0));
        chunks.iter().flat_map(|c| t.apply(c, ts(1_500))).collect()
    }

    fn opts(auto_cr: bool, auto_lf: bool) -> TransformOptions {
        TransformOptions {
            auto_cr,
            auto_lf,
            ..Default::default()
        }
    }

    #[test]
    fn cr_lf_matrix() {
        let off = opts(false, false);
        let cr = opts(true, false);
        let lf = opts(false, true);
        let both = opts(true, true);
        for (o, input, want) in [
            (off, &b"a\nb\rc\r\nd"[..], &b"a\nb\rc\r\nd"[..]),
            (cr, b"a\nb\r\nc", b"a\r\nb\r\nc"),
            (lf, b"a\rb\r\nc", b"a\r\nb\r\nc"),
            (both, b"a\nb\rc\r\n", b"a\r\nb\r\nc\r\n"),
        ] {
            assert_eq!(run(o, &[input]), want, "options {o:?} input {input:?}");
        }
    }

    #[test]
    fn crlf_split_across_chunks_is_not_doubled() {
        assert_eq!(run(opts(true, true), &[b"a\r", b"\nb"]), b"a\r\nb");
        assert_eq!(run(opts(false, true), &[b"a\r", b"b"]), b"a\r\nb");
    }

    #[test]
    fn timestamps_prefix_every_line_including_first() {
        let o = TransformOptions {
            timestamps: true,
            timestamp_format: TimestampFormat::Relative,
            ..Default::default()
        };
        assert_eq!(run(o, &[b"x\r\ny\n"]), b"[+1.500] x\r\n[+1.500] y\n");
        // An empty line gets no stamp, the next text does.
        assert_eq!(run(o, &[b"\n\nz"]), b"\n\n[+1.500] z");
    }

    #[test]
    fn timestamp_formats() {
        let t = ts(3_723_004); // 01:02:03.004 UTC
        assert_eq!(
            format_timestamp(TimestampFormat::LocalTime, t, &TimeZone::UTC, ts(0)),
            "01:02:03.004"
        );
        assert_eq!(
            format_timestamp(TimestampFormat::Relative, t, &TimeZone::UTC, ts(3_000)),
            "+3720.004"
        );
        assert_eq!(
            format_timestamp(TimestampFormat::Iso8601, t, &TimeZone::UTC, ts(0)),
            "1970-01-01T01:02:03.004+00:00"
        );
    }
}
