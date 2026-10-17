//! Sunshine packet format: parser and streaming line scanner.
//!
//! A packet is one brace group on a line:
//!
//! ```text
//! {ID|Name|R,G,B|value|Name2|value2 value3|...}
//! ```
//!
//! * `ID` may carry a chart-type prefix: `L:` line (default), `B:` bar, `P:` pie, `S:` scatter.
//! * The colour field after a name is optional and must be exactly three integers 0–255.
//! * A value field holds one or more space-separated numbers with a dot decimal separator.
//!   One value is a Y sample; two or more are interpreted by the chart (e.g. X/Y pairs).
//! * Several packets may follow each other on one line: `{a|x|1}{b|y|2}`.
//!
//! Compared with the C parser this one is strict: NaN/inf/overflow, empty IDs, series without
//! values, text after `}` and comma decimals such as `1,5` are rejected (ROADMAP defect S4).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Longest accepted ID or series name in characters.
pub const MAX_NAME_LEN: usize = 64;
/// Most series accepted in one packet.
pub const MAX_SERIES: usize = 256;
/// Longest line the scanner buffers before discarding it.
pub const MAX_LINE_LEN: usize = 4096;

/// Chart type requested by the packet ID prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    /// Time series line chart.
    #[default]
    Line,
    /// Bar chart of the latest values.
    Bar,
    /// Pie chart of the latest values.
    Pie,
    /// X/Y scatter plot.
    Scatter,
}

impl ChartKind {
    /// All kinds in UI order.
    pub const ALL: [Self; 4] = [Self::Line, Self::Bar, Self::Pie, Self::Scatter];

    /// Human readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Bar => "Bar",
            Self::Pie => "Pie",
            Self::Scatter => "Scatter",
        }
    }

    fn from_prefix(c: char) -> Option<Self> {
        match c.to_ascii_uppercase() {
            'L' => Some(Self::Line),
            'B' => Some(Self::Bar),
            'P' => Some(Self::Pie),
            'S' => Some(Self::Scatter),
            _ => None,
        }
    }
}

/// An RGB colour given in the packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb {
    /// Red 0–255.
    pub r: u8,
    /// Green 0–255.
    pub g: u8,
    /// Blue 0–255.
    pub b: u8,
}

/// One named series inside a packet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// Series name.
    pub name: String,
    /// Colour, if the packet specified one.
    pub color: Option<Rgb>,
    /// One or more finite values.
    pub values: Vec<f64>,
}

/// A parsed packet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Packet {
    /// Packet ID without the chart prefix; one chart per ID.
    pub id: String,
    /// Chart type from the prefix, if one was given.
    pub kind: Option<ChartKind>,
    /// Series in packet order.
    pub series: Vec<Series>,
}

/// Why a packet was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// Line does not start with `{`.
    #[error("packet must start with '{{'")]
    MissingOpenBrace,
    /// No closing `}`.
    #[error("packet is not terminated by '}}'")]
    Unterminated,
    /// Text after the last packet on the line.
    #[error("unexpected text after packet: {0:?}")]
    TrailingGarbage(String),
    /// Empty ID.
    #[error("packet ID is empty")]
    EmptyId,
    /// Name longer than [`MAX_NAME_LEN`].
    #[error("name longer than {MAX_NAME_LEN} characters: {0:?}")]
    NameTooLong(String),
    /// A series name was empty.
    #[error("empty series name in field {0}")]
    EmptyName(usize),
    /// A series without values.
    #[error("series {0:?} has no value")]
    MissingValue(String),
    /// A value token that is not a finite number.
    #[error("invalid value {token:?} for series {series:?}{hint}")]
    InvalidValue {
        /// Series name.
        series: String,
        /// Offending token.
        token: String,
        /// Extra hint, e.g. about comma decimals.
        hint: &'static str,
    },
    /// A colour field that is not three integers 0–255.
    #[error("invalid colour {0:?} (expected R,G,B with 0-255)")]
    InvalidColor(String),
    /// More than [`MAX_SERIES`] series.
    #[error("too many series (max {MAX_SERIES})")]
    TooManySeries,
    /// No series at all.
    #[error("packet {0:?} has no series")]
    NoSeries(String),
}

fn check_name(name: &str) -> Result<(), ParseError> {
    if name.chars().count() > MAX_NAME_LEN {
        return Err(ParseError::NameTooLong(name.chars().take(MAX_NAME_LEN).collect()));
    }
    Ok(())
}

fn parse_color(field: &str) -> Result<Rgb, ParseError> {
    let parts: Vec<&str> = field.split(',').map(str::trim).collect();
    let bad = || ParseError::InvalidColor(field.to_owned());
    let [r, g, b] = parts.as_slice() else {
        return Err(bad());
    };
    let p = |s: &str| s.parse::<u8>().map_err(|_| bad());
    Ok(Rgb {
        r: p(r)?,
        g: p(g)?,
        b: p(b)?,
    })
}

/// A field that looks like `R,G,B` (three comma-separated integers).
fn looks_like_color(field: &str) -> bool {
    let parts: Vec<&str> = field.split(',').map(str::trim).collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn parse_values(series: &str, field: &str) -> Result<Vec<f64>, ParseError> {
    let mut values = Vec::new();
    for token in field.split_whitespace() {
        let invalid = |hint| ParseError::InvalidValue {
            series: series.to_owned(),
            token: token.to_owned(),
            hint,
        };
        if token.contains(',') {
            return Err(invalid(" (use '.' as decimal separator)"));
        }
        // Rust's parser accepts "inf"/"NaN"; only plain finite decimals are allowed.
        let is_plain = token
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'));
        match token.parse::<f64>() {
            Ok(v) if is_plain && v.is_finite() => values.push(v),
            _ => return Err(invalid("")),
        }
    }
    if values.is_empty() {
        return Err(ParseError::MissingValue(series.to_owned()));
    }
    Ok(values)
}

fn parse_body(body: &str) -> Result<Packet, ParseError> {
    let mut fields = body.split('|');
    let raw_id = fields.next().unwrap_or_default().trim();
    let (kind, id) = match raw_id.split_once(':') {
        Some((prefix, rest)) if prefix.len() == 1 => {
            match prefix.chars().next().and_then(ChartKind::from_prefix) {
                Some(kind) => (Some(kind), rest.trim()),
                None => (None, raw_id),
            }
        }
        _ => (None, raw_id),
    };
    if id.is_empty() {
        return Err(ParseError::EmptyId);
    }
    check_name(id)?;

    let fields: Vec<&str> = fields.map(str::trim).collect();
    let mut series = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let name = fields[i];
        if name.is_empty() {
            return Err(ParseError::EmptyName(i + 1));
        }
        check_name(name)?;
        i += 1;
        let mut color = None;
        if let Some(field) = fields.get(i).filter(|f| looks_like_color(f)) {
            color = Some(parse_color(field)?);
            i += 1;
        }
        let Some(value_field) = fields.get(i) else {
            return Err(ParseError::MissingValue(name.to_owned()));
        };
        let values = parse_values(name, value_field)?;
        i += 1;
        if series.len() == MAX_SERIES {
            return Err(ParseError::TooManySeries);
        }
        series.push(Series {
            name: name.to_owned(),
            color,
            values,
        });
    }
    if series.is_empty() {
        return Err(ParseError::NoSeries(id.to_owned()));
    }
    Ok(Packet {
        id: id.to_owned(),
        kind,
        series,
    })
}

/// Parse all packets on one line. Returns an empty vector for lines that are not packets.
pub fn parse_line(line: &str) -> Vec<Result<Packet, ParseError>> {
    let mut rest = line.trim();
    let mut out = Vec::new();
    if !rest.starts_with('{') {
        return out;
    }
    while !rest.is_empty() {
        let Some(body_and_more) = rest.strip_prefix('{') else {
            out.push(Err(ParseError::TrailingGarbage(rest.chars().take(32).collect())));
            break;
        };
        let Some(end) = body_and_more.find(['{', '}']) else {
            out.push(Err(ParseError::Unterminated));
            break;
        };
        if body_and_more.as_bytes()[end] == b'{' {
            out.push(Err(ParseError::Unterminated));
            rest = &body_and_more[end..];
            continue;
        }
        out.push(parse_body(&body_and_more[..end]));
        rest = body_and_more[end + 1..].trim_start();
    }
    out
}

/// Parse exactly one packet.
pub fn parse_packet(line: &str) -> Result<Packet, ParseError> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return Err(ParseError::MissingOpenBrace);
    }
    let mut results = parse_line(trimmed).into_iter();
    let first = results.next().unwrap_or(Err(ParseError::MissingOpenBrace))?;
    match results.next() {
        None => Ok(first),
        Some(Err(e)) => Err(e),
        Some(Ok(_)) => Err(ParseError::TrailingGarbage(trimmed.to_owned())),
    }
}

/// Something the scanner found.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanEvent {
    /// A valid packet.
    Packet(Packet),
    /// A line that started like a packet but was rejected.
    Error {
        /// The (possibly truncated) line.
        line: String,
        /// Reason.
        error: ParseError,
    },
    /// A line exceeded [`MAX_LINE_LEN`] and was discarded.
    Overflow,
}

/// Streaming line scanner. Partial lines are kept between [`Scanner::feed`] calls, so packets
/// split across serial reads are reassembled (ROADMAP defect S2).
#[derive(Debug, Default, Clone)]
pub struct Scanner {
    line: Vec<u8>,
    discarding: bool,
}

impl Scanner {
    /// New scanner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop any buffered partial line.
    pub fn reset(&mut self) {
        self.line.clear();
        self.discarding = false;
    }

    /// Bytes currently buffered as an incomplete line.
    pub fn pending(&self) -> usize {
        self.line.len()
    }

    /// Feed received bytes; returns the events for every line completed by them.
    pub fn feed(&mut self, data: &[u8]) -> Vec<ScanEvent> {
        let mut events = Vec::new();
        for &b in data {
            match b {
                b'\r' | b'\n' => {
                    if self.discarding {
                        self.discarding = false;
                    } else if !self.line.is_empty() {
                        self.finish_line(&mut events);
                    }
                    self.line.clear();
                }
                _ if self.discarding => {}
                b'\t' | 0x20.. => {
                    if self.line.len() == MAX_LINE_LEN {
                        self.line.clear();
                        self.discarding = true;
                        events.push(ScanEvent::Overflow);
                    } else {
                        self.line.push(b);
                    }
                }
                _ => {} // other control bytes are dropped, as in the C scanner
            }
        }
        events
    }

    /// Treat buffered data as a complete line (e.g. when the port closes).
    pub fn flush(&mut self) -> Vec<ScanEvent> {
        let mut events = Vec::new();
        if !self.discarding && !self.line.is_empty() {
            self.finish_line(&mut events);
        }
        self.reset();
        events
    }

    fn finish_line(&self, events: &mut Vec<ScanEvent>) {
        let text = String::from_utf8_lossy(&self.line);
        for result in parse_line(&text) {
            events.push(match result {
                Ok(p) => ScanEvent::Packet(p),
                Err(error) => ScanEvent::Error {
                    line: text.chars().take(200).collect(),
                    error,
                },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    fn ok(line: &str) -> Packet {
        parse_packet(line).unwrap_or_else(|e| panic!("{line:?}: {e}"))
    }

    #[test]
    fn readme_examples() {
        let p = ok("{P0|Temperature|255,0,0|23.5}");
        assert_eq!(p.id, "P0");
        assert_eq!(p.kind, None);
        assert_eq!(
            p.series,
            vec![Series {
                name: "Temperature".into(),
                color: Some(Rgb { r: 255, g: 0, b: 0 }),
                values: vec![23.5]
            }]
        );

        let p = ok("{P1|Roll|255,0,0|45.2|Pitch|0,255,0|12.3|Yaw|0,0,255|180.0}");
        assert_eq!(
            p.series.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Roll", "Pitch", "Yaw"]
        );
        assert_eq!(p.series[2].values, [180.0]);
    }

    #[rstest]
    #[case("{T|a|1}", None, "T")]
    #[case("{L:T|a|1}", Some(ChartKind::Line), "T")]
    #[case("{b:T|a|1}", Some(ChartKind::Bar), "T")]
    #[case("{P: pie |a|1}", Some(ChartKind::Pie), "pie")]
    #[case("{S:xy|a|1 2 3 4}", Some(ChartKind::Scatter), "xy")]
    #[case("{dev:01|a|1}", None, "dev:01")]
    #[case("{X:a|b|1}", None, "X:a")]
    fn chart_prefixes(#[case] line: &str, #[case] kind: Option<ChartKind>, #[case] id: &str) {
        let p = ok(line);
        assert_eq!((p.kind, p.id.as_str()), (kind, id));
    }

    #[test]
    fn optional_color_and_multiple_values() {
        let p = ok("{id|a|1 2.5 -3e2|b|0,0,0|+4}");
        assert_eq!(p.series[0].values, [1.0, 2.5, -300.0]);
        assert_eq!(p.series[0].color, None);
        assert_eq!(p.series[1].color, Some(Rgb { r: 0, g: 0, b: 0 }));
    }

    #[rstest]
    #[case("{}", ParseError::EmptyId)]
    #[case("{|a|1}", ParseError::EmptyId)]
    #[case("{a}", ParseError::NoSeries("a".into()))]
    #[case("{a|b}", ParseError::MissingValue("b".into()))]
    #[case("{a|b|255,0,0}", ParseError::MissingValue("b".into()))]
    #[case("{a|b|1,5}", ParseError::InvalidValue { series: "b".into(), token: "1,5".into(), hint: " (use '.' as decimal separator)" })]
    #[case("{a|b|nan}", ParseError::InvalidValue { series: "b".into(), token: "nan".into(), hint: "" })]
    #[case("{a|b|inf}", ParseError::InvalidValue { series: "b".into(), token: "inf".into(), hint: "" })]
    #[case("{a|b|1e400}", ParseError::InvalidValue { series: "b".into(), token: "1e400".into(), hint: "" })]
    #[case("{a|b|0x1p3}", ParseError::InvalidValue { series: "b".into(), token: "0x1p3".into(), hint: "" })]
    #[case("{a|b|256,0,0|1}", ParseError::InvalidColor("256,0,0".into()))]
    #[case("{a||1}", ParseError::EmptyName(1))]
    #[case("{a|b|1", ParseError::Unterminated)]
    #[case("{a|b|1} trailing", ParseError::TrailingGarbage("trailing".into()))]
    fn rejects(#[case] line: &str, #[case] want: ParseError) {
        assert_eq!(parse_packet(line), Err(want));
    }

    #[test]
    fn long_names_rejected() {
        let long = "n".repeat(MAX_NAME_LEN + 1);
        assert!(matches!(
            parse_packet(&format!("{{{long}|a|1}}")),
            Err(ParseError::NameTooLong(_))
        ));
        assert!(parse_packet(&format!("{{id|{}|1}}", "n".repeat(MAX_NAME_LEN))).is_ok());
    }

    #[test]
    fn several_packets_per_line() {
        let r = parse_line("{a|x|1}{b|y|2} {c|z|3}");
        let ids: Vec<_> = r.into_iter().map(|p| p.unwrap().id).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        // The C parser produced series "2" and a value-less "x" for this input.
        assert_eq!(parse_line("{a|x|1}{a|x|2}").len(), 2);
    }

    #[test]
    fn non_packet_lines_are_ignored() {
        assert!(parse_line("boot: ok").is_empty());
        let mut s = Scanner::new();
        assert!(s.feed(b"hello world\r\n").is_empty());
    }

    #[test]
    fn scanner_reassembles_split_packets() {
        let mut s = Scanner::new();
        assert!(s.feed(b"{P0|T|2").is_empty());
        let ev = s.feed(b"3.5}\n");
        assert_eq!(ev.len(), 1);
        let ScanEvent::Packet(p) = &ev[0] else {
            panic!("{ev:?}")
        };
        assert_eq!(p.series[0].values, [23.5]);
    }

    #[test]
    fn scanner_line_endings_and_control_bytes() {
        let mut s = Scanner::new();
        let ev = s.feed(b"{a|x|1}\r{b|\x07y|2}\r\n\n{c|z|3}\n");
        assert_eq!(ev.iter().filter(|e| matches!(e, ScanEvent::Packet(_))).count(), 3);
    }

    #[test]
    fn scanner_overflow_discards_until_newline() {
        let mut s = Scanner::new();
        let mut junk = vec![b'x'; MAX_LINE_LEN + 10];
        junk.extend_from_slice(b"{a|b|1}\n{a|b|2}\n");
        let ev = s.feed(&junk);
        assert_eq!(ev[0], ScanEvent::Overflow);
        // The packet glued to the overflowing line is lost, the next one is fine.
        assert_eq!(ev.len(), 2);
        assert!(matches!(&ev[1], ScanEvent::Packet(p) if p.series[0].values == [2.0]));
    }

    #[test]
    fn invalid_utf8_names_are_made_valid() {
        let mut s = Scanner::new();
        let ev = s.feed(b"{id|na\xffme|1}\n");
        let ScanEvent::Packet(p) = &ev[0] else {
            panic!("{ev:?}")
        };
        assert_eq!(p.series[0].name, "na\u{fffd}me");
    }

    #[test]
    fn flush_emits_pending_line() {
        let mut s = Scanner::new();
        assert!(s.feed(b"{a|b|1}").is_empty());
        assert_eq!(s.flush().len(), 1);
        assert_eq!(s.pending(), 0);
    }

    fn packets(events: &[ScanEvent]) -> Vec<Packet> {
        events
            .iter()
            .filter_map(|e| {
                if let ScanEvent::Packet(p) = e {
                    Some(p.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    proptest! {
        /// Splitting the byte stream anywhere yields the same packets as feeding it whole.
        #[test]
        fn split_anywhere_is_equivalent(split in 0usize..120, value in -1.0e6f64..1.0e6) {
            let stream = format!("noise\r\n{{P0|Temp|255,0,0|{value}}}\n{{B:P1|a|1|b|2}}\r\n");
            let bytes = stream.as_bytes();
            let split = split.min(bytes.len());
            let whole = packets(&Scanner::new().feed(bytes));
            let mut s = Scanner::new();
            let mut parts = s.feed(&bytes[..split]);
            parts.extend(s.feed(&bytes[split..]));
            prop_assert_eq!(packets(&parts), whole.clone());
            prop_assert_eq!(whole.len(), 2);
        }

        /// The scanner never panics and never buffers more than MAX_LINE_LEN.
        #[test]
        fn arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..10_000)) {
            let mut s = Scanner::new();
            let _ = s.feed(&data);
            prop_assert!(s.pending() <= MAX_LINE_LEN);
        }
    }
}
