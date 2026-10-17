//! Protocol decoders: frame assembly plus human-readable decoding.
//!
//! Every decoder is sans-IO: it is fed bytes with their arrival time (µs) and returns complete
//! [`Frame`]s. Unlike the C decoders, which assumed one read == one frame, frames are assembled
//! across reads, and nothing reads past the bytes it was given (ROADMAP defect D1).

mod at;
mod can;
mod ethercat;
mod framing;
pub mod modbus;
mod nmea;

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

pub use ethercat::EtherCatDecoder;
pub use framing::LineAssembler;

/// Supported protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// One frame per received chunk, shown as hex and ASCII.
    Raw,
    /// Hayes AT commands and responses.
    At,
    /// NMEA 0183 sentences.
    Nmea,
    /// Modbus RTU (binary, CRC-16).
    ModbusRtu,
    /// Modbus ASCII (`:` … LRC CRLF).
    ModbusAscii,
    /// Modbus TCP (MBAP header).
    ModbusTcp,
    /// candump text (`can0 123 [2] 11 22`, `123#1122`).
    Can,
    /// Lawicel SLCAN (`t1232AABB`).
    Slcan,
    /// EtherCAT frames.
    EtherCat,
    /// RS232 modem line changes (no byte decoding).
    Rs232,
}

impl Protocol {
    /// All protocols in menu order.
    pub const ALL: [Self; 10] = [
        Self::Raw,
        Self::At,
        Self::Nmea,
        Self::ModbusRtu,
        Self::ModbusAscii,
        Self::ModbusTcp,
        Self::Can,
        Self::Slcan,
        Self::EtherCat,
        Self::Rs232,
    ];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Raw => "Raw chunks",
            Self::At => "AT commands",
            Self::Nmea => "NMEA 0183",
            Self::ModbusRtu => "Modbus RTU",
            Self::ModbusAscii => "Modbus ASCII",
            Self::ModbusTcp => "Modbus TCP",
            Self::Can => "CAN (candump)",
            Self::Slcan => "CAN (SLCAN)",
            Self::EtherCat => "EtherCAT",
            Self::Rs232 => "RS232 signals",
        }
    }

    /// Identifier used in configs and MCP (`modbus_rtu`, ...).
    pub fn id(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::At => "at",
            Self::Nmea => "nmea",
            Self::ModbusRtu => "modbus_rtu",
            Self::ModbusAscii => "modbus_ascii",
            Self::ModbusTcp => "modbus_tcp",
            Self::Can => "can",
            Self::Slcan => "slcan",
            Self::EtherCat => "ethercat",
            Self::Rs232 => "rs232",
        }
    }

    /// Parse an identifier (also accepts `modbus-rtu`, `rtu`, ...).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Some(match s.as_str() {
            "raw" | "chunks" => Self::Raw,
            "at" => Self::At,
            "nmea" | "nmea0183" | "gps" => Self::Nmea,
            "modbus_rtu" | "rtu" | "modbus" => Self::ModbusRtu,
            "modbus_ascii" | "ascii" => Self::ModbusAscii,
            "modbus_tcp" | "mbap" => Self::ModbusTcp,
            "can" | "candump" => Self::Can,
            "slcan" | "lawicel" => Self::Slcan,
            "ethercat" | "ecat" => Self::EtherCat,
            "rs232" | "signals" | "modem" => Self::Rs232,
            _ => return None,
        })
    }
}

/// Integrity check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Check {
    /// The protocol has no checksum, or none was present.
    None,
    /// Checksum valid.
    Ok,
    /// Checksum mismatch.
    Failed {
        /// Value in the frame.
        received: String,
        /// Value computed over the frame.
        computed: String,
    },
}

/// A decoded field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Name.
    pub name: String,
    /// Value.
    pub value: String,
}

/// Which side sent the data: masters send requests, slaves send responses (Modbus).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Data we sent (requests when we are the master).
    Request,
    /// Data we received (responses from the device).
    Response,
}

/// A decoded frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// Protocol.
    pub protocol: Protocol,
    /// The frame's bytes.
    pub bytes: Vec<u8>,
    /// One-line summary in the C decoders' style.
    pub summary: String,
    /// Structured fields.
    pub fields: Vec<Field>,
    /// Checksum result.
    pub check: Check,
}

impl Frame {
    pub(crate) fn new(protocol: Protocol, bytes: &[u8], summary: String) -> Self {
        Self {
            protocol,
            bytes: bytes.to_vec(),
            summary,
            fields: Vec::new(),
            check: Check::None,
        }
    }

    pub(crate) fn field(mut self, name: &str, value: impl Into<String>) -> Self {
        self.fields.push(Field {
            name: name.to_owned(),
            value: value.into(),
        });
        self
    }

    pub(crate) fn with_check(mut self, check: Check) -> Self {
        self.check = check;
        self
    }
}

/// Hex bytes separated by spaces (`DE AD`).
pub(crate) fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}

/// Strict hex number: only ASCII hex digits (`from_str_radix` alone also accepts a sign).
pub(crate) fn parse_hex(s: &[u8]) -> Option<u32> {
    if s.is_empty() || s.len() > 8 || !s.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    s.iter()
        .try_fold(0u32, |acc, &b| Some(acc << 4 | char::from(b).to_digit(16)?))
}

/// Printable ASCII with `.` for everything else.
pub(crate) fn ascii(data: &[u8]) -> String {
    data.iter()
        .map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' })
        .collect()
}

/// A streaming decoder.
pub trait Decoder: Send + std::fmt::Debug {
    /// The protocol.
    fn protocol(&self) -> Protocol;
    /// Feed bytes that arrived at `ts_us`; returns completed frames.
    fn feed(&mut self, data: &[u8], ts_us: i64) -> Vec<Frame>;
    /// Called periodically: frames completed by silence (Modbus RTU gap), or `[]`.
    fn poll(&mut self, _ts_us: i64) -> Vec<Frame> {
        Vec::new()
    }
    /// Drop partial data.
    fn reset(&mut self);
}

/// Builds the frame for a modem line change: `lines` in display order, `changed` the names
/// that differ from the previous state (empty for the initial state).
pub fn signals_frame(lines: &[(&str, bool)], changed: &[&str]) -> Frame {
    let mut summary = String::from("[RS232]");
    for (name, on) in lines {
        let _ = write!(summary, " {name}:{}", u8::from(*on));
    }
    if changed.is_empty() {
        summary.push_str(" (initial)");
    } else {
        let _ = write!(summary, " Changed: {}", changed.join(","));
    }
    let mut f = Frame::new(Protocol::Rs232, &[], summary);
    for (name, on) in lines {
        f = f.field(name, if *on { "on" } else { "off" });
    }
    f.field("changed", changed.join(","))
}

#[derive(Debug)]
struct NullDecoder;

impl Decoder for NullDecoder {
    fn protocol(&self) -> Protocol {
        Protocol::Rs232
    }

    fn feed(&mut self, _data: &[u8], _ts_us: i64) -> Vec<Frame> {
        Vec::new()
    }

    fn reset(&mut self) {}
}

#[derive(Debug)]
struct RawDecoder;

impl Decoder for RawDecoder {
    fn protocol(&self) -> Protocol {
        Protocol::Raw
    }

    fn feed(&mut self, data: &[u8], _ts_us: i64) -> Vec<Frame> {
        if data.is_empty() {
            return Vec::new();
        }
        let shown = &data[..data.len().min(64)];
        let more = if data.len() > 64 { " …" } else { "" };
        vec![
            Frame::new(
                Protocol::Raw,
                data,
                format!(
                    "[RAW] {} bytes: {}{more} | {}",
                    data.len(),
                    hex(shown),
                    ascii(shown)
                ),
            )
            .field("length", data.len().to_string()),
        ]
    }

    fn reset(&mut self) {}
}

/// A line-oriented decoder built from a decode function.
#[derive(Debug)]
struct LineDecoder {
    protocol: Protocol,
    lines: LineAssembler,
    decode: fn(&[u8]) -> Option<Frame>,
}

impl Decoder for LineDecoder {
    fn protocol(&self) -> Protocol {
        self.protocol
    }

    fn feed(&mut self, data: &[u8], _ts_us: i64) -> Vec<Frame> {
        self.lines
            .feed(data)
            .iter()
            .filter_map(|l| (self.decode)(l))
            .collect()
    }

    fn reset(&mut self) {
        self.lines.reset();
    }
}

/// Create a decoder. `baud` sets the Modbus RTU inter-frame gap.
pub fn decoder(protocol: Protocol, role: Role, baud: Option<u32>) -> Box<dyn Decoder> {
    match protocol {
        Protocol::Raw => Box::new(RawDecoder),
        Protocol::At => Box::new(LineDecoder {
            protocol,
            lines: LineAssembler::new(1024),
            decode: at::decode,
        }),
        Protocol::Nmea => Box::new(LineDecoder {
            protocol,
            lines: LineAssembler::new(1024),
            decode: nmea::decode,
        }),
        Protocol::Can => Box::new(LineDecoder {
            protocol,
            lines: LineAssembler::new(512),
            decode: can::decode_candump,
        }),
        Protocol::Slcan => Box::new(LineDecoder {
            protocol,
            lines: LineAssembler::new(64),
            decode: can::decode_slcan,
        }),
        Protocol::ModbusRtu => Box::new(modbus::RtuDecoder::new(role, baud)),
        Protocol::ModbusAscii => Box::new(modbus::AsciiDecoder::new(role)),
        Protocol::ModbusTcp => Box::new(modbus::TcpDecoder::new(role)),
        Protocol::EtherCat => Box::new(ethercat::EtherCatDecoder::default()),
        Protocol::Rs232 => Box::new(NullDecoder),
    }
}

/// Score how well `sample` matches each protocol (0.0–1.0), best first. Raw is never suggested.
pub fn detect(sample: &[u8]) -> Vec<(Protocol, f32)> {
    let lines: Vec<&[u8]> = sample
        .split(|&b| b == b'\n' || b == b'\r')
        .filter(|l| !l.is_empty())
        .collect();
    let line_share = |f: &dyn Fn(&[u8]) -> bool| {
        if lines.is_empty() {
            0.0
        } else {
            lines.iter().filter(|l| f(l)).count() as f32 / lines.len() as f32
        }
    };
    let mut scores = vec![
        (Protocol::Nmea, line_share(&|l| nmea::looks_valid(l))),
        (Protocol::At, line_share(&at::looks_like)),
        (
            Protocol::ModbusAscii,
            line_share(&|l| modbus::ascii_looks_valid(l)),
        ),
        (Protocol::Slcan, line_share(&can::slcan_looks_like) * 0.95),
        (Protocol::Can, line_share(&can::candump_looks_like)),
        (Protocol::ModbusRtu, modbus::rtu_score(sample)),
        (Protocol::ModbusTcp, modbus::tcp_score(sample)),
    ];
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores
}

/// Pick a protocol for `sample` if one scores at least `threshold`.
pub fn detect_best(sample: &[u8], threshold: f32) -> Option<Protocol> {
    detect(sample)
        .into_iter()
        .next()
        .filter(|(_, s)| *s >= threshold)
        .map(|(p, _)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_ids_roundtrip() {
        for p in Protocol::ALL {
            assert_eq!(Protocol::parse(p.id()), Some(p));
        }
        assert_eq!(Protocol::parse("Modbus-RTU"), Some(Protocol::ModbusRtu));
        assert_eq!(Protocol::parse("x"), None);
    }

    #[test]
    fn raw_frames() {
        let mut d = decoder(Protocol::Raw, Role::Response, None);
        let f = d.feed(b"Hi\x00", 0);
        assert_eq!(f[0].summary, "[RAW] 3 bytes: 48 69 00 | Hi.");
    }

    #[test]
    fn signal_frames() {
        let f = signals_frame(&[("DTR", true), ("CTS", false)], &["CTS"]);
        assert_eq!(f.summary, "[RS232] DTR:1 CTS:0 Changed: CTS");
        assert_eq!(
            signals_frame(&[("DTR", false)], &[]).summary,
            "[RS232] DTR:0 (initial)"
        );
    }

    #[test]
    fn detection() {
        let nmea = b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A\r\n";
        assert_eq!(detect_best(nmea, 0.6), Some(Protocol::Nmea));
        assert_eq!(
            detect_best(b"AT+CSQ\r\n+CSQ: 23,99\r\nOK\r\n", 0.6),
            Some(Protocol::At)
        );
        assert_eq!(
            detect_best(b":010300000001FB\r\n:0103020001F9\r\n", 0.6),
            Some(Protocol::ModbusAscii)
        );
        assert_eq!(
            detect_best(b"t12320102\rt1234AABBCCDD\r", 0.6),
            Some(Protocol::Slcan)
        );
        assert_eq!(
            detect_best(b"  can0  123   [2]  11 22\n  can0  1F334455   [1]  01\n", 0.6),
            Some(Protocol::Can)
        );
        let mut rtu = modbus::rtu_frame(1, &[0x03, 0x00, 0x00, 0x00, 0x0A]);
        rtu.extend(modbus::rtu_frame(1, &[0x03, 0x02, 0x00, 0x2A]));
        assert_eq!(detect_best(&rtu, 0.6), Some(Protocol::ModbusRtu));
        assert_eq!(detect_best(b"hello world, just text\n", 0.6), None);
    }
}
