//! Session log file formats (compatible with the C version's layouts).
//!
//! | format | header | record |
//! |---|---|---|
//! | raw  | – | received bytes as-is |
//! | text | – | `[HH:MM:SS.mmm] R: printable<0D><0A>` |
//! | csv  | `Timestamp,Direction,Length,Data_Hex,Data_ASCII` | `ts,RX,len,"AA BB","ascii"` |
//! | json | `{"session": {...}, "data": [` | `{"timestamp": .., "direction": "rx", "length": n, "hex": [..]}` |
//! | xml  | `<gtkterm_log version="1.0"><packets>` | `<packet ...><hex/><ascii><![CDATA[..]]></ascii></packet>` |
//!
//! Unlike the C version the JSON and XML footers are always written, so files are valid.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Log file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Received bytes only, unmodified.
    #[default]
    Raw,
    /// One readable line per chunk.
    Text,
    /// Comma separated values.
    Csv,
    /// JSON document.
    Json,
    /// XML document.
    Xml,
}

impl LogFormat {
    /// All formats.
    pub const ALL: [Self; 5] = [Self::Raw, Self::Text, Self::Csv, Self::Json, Self::Xml];

    /// File extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Raw => "log",
            Self::Text => "txt",
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Xml => "xml",
        }
    }

    /// Parse a name or extension.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "raw" | "log" | "bin" => Some(Self::Raw),
            "text" | "txt" => Some(Self::Text),
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            "xml" => Some(Self::Xml),
            _ => None,
        }
    }

    /// Whether transmitted data is logged too (raw logs contain received bytes only).
    pub fn logs_tx(self) -> bool {
        self != Self::Raw
    }
}

/// Information written into structured headers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogMeta {
    /// Session start, ISO 8601.
    pub start_time: String,
    /// Port description.
    pub port: String,
    /// Profile name.
    pub profile: String,
}

/// One chunk to log.
#[derive(Debug, Clone, Copy)]
pub struct LogRecord<'a> {
    /// Pre-formatted timestamp (`HH:MM:SS.mmm` for text, ISO 8601 otherwise).
    pub time: &'a str,
    /// True for received data.
    pub rx: bool,
    /// Payload.
    pub data: &'a [u8],
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn printable(b: u8) -> bool {
    (0x20..=0x7E).contains(&b)
}

/// Header written when the file is created.
pub fn header(format: LogFormat, meta: &LogMeta) -> String {
    match format {
        LogFormat::Raw | LogFormat::Text => String::new(),
        LogFormat::Csv => "Timestamp,Direction,Length,Data_Hex,Data_ASCII\n".into(),
        LogFormat::Json => format!(
            "{{\n  \"session\": {{\n    \"start_time\": {},\n    \"port\": {},\n    \"profile\": {}\n  }},\n  \"data\": [\n",
            json_str(&meta.start_time),
            json_str(&meta.port),
            json_str(&meta.profile)
        ),
        LogFormat::Xml => format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gtkterm_log version=\"1.0\" start_time=\"{}\" port=\"{}\" profile=\"{}\">\n  <packets>\n",
            xml_attr(&meta.start_time),
            xml_attr(&meta.port),
            xml_attr(&meta.profile)
        ),
    }
}

/// Footer written when logging stops.
pub fn footer(format: LogFormat) -> &'static str {
    match format {
        LogFormat::Json => "\n  ]\n}\n",
        LogFormat::Xml => "  </packets>\n</gtkterm_log>\n",
        _ => "",
    }
}

fn xml_attr(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            c if (c as u32) < 0x20 => " ".to_owned(),
            c => c.to_string(),
        })
        .collect()
}

/// Format one record. `first` must be true for the first record after the header (JSON commas).
pub fn record(format: LogFormat, rec: &LogRecord<'_>, first: bool) -> Vec<u8> {
    let dir_upper = if rec.rx { "RX" } else { "TX" };
    let dir_lower = if rec.rx { "rx" } else { "tx" };
    match format {
        LogFormat::Raw => rec.data.to_vec(),
        LogFormat::Text => {
            let mut s = format!("[{}] {}: ", rec.time, if rec.rx { 'R' } else { 'T' });
            for &b in rec.data {
                if printable(b) {
                    s.push(b as char);
                } else {
                    let _ = write!(s, "<{b:02X}>");
                }
            }
            s.push('\n');
            s.into_bytes()
        }
        LogFormat::Csv => {
            let hex = rec
                .data
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii: String = rec
                .data
                .iter()
                .map(|&b| if printable(b) { b as char } else { '.' })
                .collect::<String>()
                .replace('"', "\"\"");
            format!(
                "{},{dir_upper},{},\"{hex}\",\"{ascii}\"\n",
                rec.time,
                rec.data.len()
            )
            .into_bytes()
        }
        LogFormat::Json => {
            let hex = rec.data.iter().map(u8::to_string).collect::<Vec<_>>().join(", ");
            format!(
                "{}    {{\"timestamp\": {}, \"direction\": \"{dir_lower}\", \"length\": {}, \"hex\": [{hex}]}}",
                if first { "" } else { ",\n" },
                json_str(rec.time),
                rec.data.len()
            )
            .into_bytes()
        }
        LogFormat::Xml => {
            let hex = rec
                .data
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii: String = rec
                .data
                .iter()
                .map(|&b| if printable(b) { b as char } else { '.' })
                .collect();
            // "]]>" cannot appear inside CDATA; split it across two sections.
            let ascii = ascii.replace("]]>", "]]]]><![CDATA[>");
            format!(
                "    <packet timestamp=\"{}\" direction=\"{dir_lower}\" length=\"{}\">\n      <hex>{hex}</hex>\n      <ascii><![CDATA[{ascii}]]></ascii>\n    </packet>\n",
                xml_attr(rec.time),
                rec.data.len()
            )
            .into_bytes()
        }
    }
}

/// File name for a new session log: `YYYYMMDD_HHMMSS_<port>_<profile>.<ext>` with unsafe
/// characters replaced, so a profile or port name cannot escape the log directory.
pub fn session_file_name(stamp: &str, port: &str, profile: &str, format: LogFormat) -> String {
    fn clean(s: &str) -> String {
        let s = s.strip_prefix("/dev/").unwrap_or(s);
        let cleaned: String = s
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '+') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let trimmed = cleaned.trim_matches('_');
        if trimmed.is_empty() {
            "x".into()
        } else {
            trimmed.chars().take(48).collect()
        }
    }
    format!(
        "{}_{}_{}.{}",
        clean(stamp),
        clean(port),
        clean(profile),
        format.extension()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(rx: bool, data: &[u8]) -> LogRecord<'_> {
        LogRecord {
            time: "12:00:00.123",
            rx,
            data,
        }
    }

    fn document(format: LogFormat, records: &[LogRecord<'_>]) -> String {
        let meta = LogMeta {
            start_time: "2026-10-17T12:00:00".into(),
            port: "/dev/ttyUSB0".into(),
            profile: "a\"b".into(),
        };
        let mut out = header(format, &meta).into_bytes();
        for (i, r) in records.iter().enumerate() {
            out.extend(record(format, r, i == 0));
        }
        out.extend_from_slice(footer(format).as_bytes());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn text_and_csv_lines() {
        assert_eq!(
            record(LogFormat::Text, &rec(true, b"OK\r\n"), true),
            b"[12:00:00.123] R: OK<0D><0A>\n"
        );
        assert_eq!(
            record(LogFormat::Csv, &rec(false, b"a\"\x01"), true),
            b"12:00:00.123,TX,3,\"61 22 01\",\"a\"\".\"\n"
        );
        assert_eq!(record(LogFormat::Raw, &rec(true, b"\x00\xff"), true), b"\x00\xff");
    }

    #[test]
    fn json_document_is_valid() {
        let doc = document(LogFormat::Json, &[rec(true, b"hi"), rec(false, &[0, 255])]);
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(v["session"]["profile"], "a\"b");
        assert_eq!(v["data"][1]["hex"], serde_json::json!([0, 255]));
        assert_eq!(v["data"][0]["direction"], "rx");
        // Empty logs are valid too.
        serde_json::from_str::<serde_json::Value>(&document(LogFormat::Json, &[])).unwrap();
    }

    #[test]
    fn xml_document_escapes() {
        let doc = document(LogFormat::Xml, &[rec(true, b"x]]>y<&")]);
        assert!(doc.contains("profile=\"a&quot;b\""));
        assert!(doc.contains("<![CDATA[x]]]]><![CDATA[>y<&]]>"));
        assert!(doc.ends_with("</gtkterm_log>\n"));
    }

    #[test]
    fn session_file_names_are_sanitized() {
        assert_eq!(
            session_file_name("20261017_120000", "/dev/ttyUSB0", "default", LogFormat::Csv),
            "20261017_120000_ttyUSB0_default.csv"
        );
        assert_eq!(
            session_file_name("20261017_120000", "TCP:host:5000", "../../etc", LogFormat::Json),
            "20261017_120000_TCP_host_5000_etc.json"
        );
        assert!(!session_file_name("s", "/", "/", LogFormat::Raw).contains('/'));
    }
}
