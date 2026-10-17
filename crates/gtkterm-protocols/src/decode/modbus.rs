//! Modbus RTU, ASCII and TCP: framing, decoding, request building and response parsing.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::{Check, Decoder, Frame, LineAssembler, Protocol, Role, hex};
use crate::checksum::{crc16_modbus, lrc};

const FUNCTIONS: &[(u8, &str)] = &[
    (0x01, "Read Coils"),
    (0x02, "Read Discrete Inputs"),
    (0x03, "Read Holding Registers"),
    (0x04, "Read Input Registers"),
    (0x05, "Write Single Coil"),
    (0x06, "Write Single Register"),
    (0x07, "Read Exception Status"),
    (0x08, "Diagnostics"),
    (0x0F, "Write Multiple Coils"),
    (0x10, "Write Multiple Registers"),
    (0x11, "Report Server ID"),
    (0x14, "Read File Record"),
    (0x15, "Write File Record"),
    (0x16, "Mask Write Register"),
    (0x17, "Read/Write Multiple Registers"),
    (0x2B, "Read Device Identification"),
];

/// Name of a function code.
pub fn function_name(fc: u8) -> &'static str {
    FUNCTIONS
        .iter()
        .find(|(c, _)| *c == fc & 0x7F)
        .map_or("Unknown", |(_, n)| n)
}

/// Name of an exception code.
pub fn exception_name(code: u8) -> &'static str {
    match code {
        1 => "Illegal Function",
        2 => "Illegal Data Address",
        3 => "Illegal Data Value",
        4 => "Server Device Failure",
        5 => "Acknowledge",
        6 => "Server Device Busy",
        8 => "Memory Parity Error",
        10 => "Gateway Path Unavailable",
        11 => "Gateway Target Failed to Respond",
        _ => "Unknown exception",
    }
}

fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]))
}

/// Describe a PDU (function code + data) and add fields to `frame`.
fn describe_pdu(pdu: &[u8], role: Role, frame: &mut Frame) -> String {
    let Some(&fc) = pdu.first() else {
        return "empty PDU".into();
    };
    let data = &pdu[1..];
    let mut s = format!("Func:{fc:02X} ({})", function_name(fc));
    frame.fields.push(super::Field {
        name: "function".into(),
        value: format!("0x{fc:02X} {}", function_name(fc)),
    });
    let mut field = |n: &str, v: String| {
        frame.fields.push(super::Field {
            name: n.into(),
            value: v,
        });
    };
    if fc & 0x80 != 0 {
        let code = data.first().copied().unwrap_or(0);
        let _ = write!(s, " Exception:0x{code:02X} ({})", exception_name(code));
        field("exception", format!("{code} {}", exception_name(code)));
        return s;
    }
    match (fc, role) {
        (1..=4, Role::Request) if data.len() == 4 => {
            let (a, n) = (u16_at(data, 0).unwrap_or(0), u16_at(data, 2).unwrap_or(0));
            let _ = write!(s, " Addr:0x{a:04X} Count:{n}");
            field("address", a.to_string());
            field("count", n.to_string());
        }
        (3 | 4, Role::Response) if !data.is_empty() && data.len() == 1 + data[0] as usize => {
            let regs: Vec<u16> = data[1..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            let shown: Vec<String> = regs.iter().take(16).map(|r| format!("0x{r:04X}")).collect();
            let more = if regs.len() > 16 { " …" } else { "" };
            let noun = if regs.len() == 1 { "register" } else { "registers" };
            let _ = write!(s, " [{} {noun}] {}{more}", regs.len(), shown.join(" "));
            field(
                "registers",
                regs.iter().map(ToString::to_string).collect::<Vec<_>>().join(","),
            );
        }
        (1 | 2, Role::Response) if !data.is_empty() && data.len() == 1 + data[0] as usize => {
            let bits: String = data[1..]
                .iter()
                .flat_map(|b| (0..8).map(move |i| if b >> i & 1 == 1 { '1' } else { '0' }))
                .collect();
            let _ = write!(s, " Bytes:{} Bits(LSB first):{bits}", data[0]);
            field("bits", bits);
        }
        (5 | 6, _) if data.len() == 4 => {
            let (a, v) = (u16_at(data, 0).unwrap_or(0), u16_at(data, 2).unwrap_or(0));
            let _ = write!(s, " [Addr:0x{a:04X} Val:0x{v:04X}]");
            field("address", a.to_string());
            field("value", v.to_string());
        }
        (0x0F | 0x10, Role::Request) if data.len() >= 5 => {
            let (a, n) = (u16_at(data, 0).unwrap_or(0), u16_at(data, 2).unwrap_or(0));
            let _ = write!(s, " Addr:0x{a:04X} Count:{n} Data: {}", hex(&data[5..]));
            field("address", a.to_string());
            field("count", n.to_string());
        }
        (0x0F | 0x10, Role::Response) if data.len() == 4 => {
            let (a, n) = (u16_at(data, 0).unwrap_or(0), u16_at(data, 2).unwrap_or(0));
            let _ = write!(s, " [Addr:0x{a:04X} Count:{n}]");
            field("address", a.to_string());
            field("count", n.to_string());
        }
        _ if !data.is_empty() => {
            let _ = write!(s, " Data: {}", hex(&data[..data.len().min(32)]));
            if data.len() > 32 {
                let _ = write!(s, "... ({} more bytes)", data.len() - 32);
            }
        }
        _ => {}
    }
    s
}

/// Candidate RTU frame lengths for the data at the start of `buf` (best guess first).
fn rtu_lengths(buf: &[u8], role: Role) -> Vec<usize> {
    let Some(&fc) = buf.get(1) else { return Vec::new() };
    let req = |b: &[u8]| -> Option<usize> {
        match fc {
            1..=6 => Some(8),
            0x0F | 0x10 => Some(9 + *b.get(6)? as usize),
            0x07 | 0x11 => Some(4),
            _ => None,
        }
    };
    let resp = |b: &[u8]| -> Option<usize> {
        match fc {
            f if f & 0x80 != 0 => Some(5),
            1..=4 | 0x11 | 0x17 => Some(5 + *b.get(2)? as usize),
            5 | 6 | 0x0F | 0x10 => Some(8),
            0x07 => Some(5),
            _ => None,
        }
    };
    let (first, second) = match role {
        Role::Request => (req(buf), resp(buf)),
        Role::Response => (resp(buf), req(buf)),
    };
    let mut v: Vec<usize> = [first, second].into_iter().flatten().collect();
    v.dedup();
    v
}

fn crc_ok(frame: &[u8]) -> bool {
    frame.len() >= 4 && {
        let n = frame.len();
        crc16_modbus(&frame[..n - 2]).to_le_bytes() == [frame[n - 2], frame[n - 1]]
    }
}

fn rtu_frame_out(bytes: &[u8], role: Role) -> Frame {
    let mut f = Frame::new(Protocol::ModbusRtu, bytes, String::new());
    if bytes.len() < 4 {
        f.summary = format!("[MODBUS-RTU] incomplete frame: {}", hex(bytes));
        f.check = Check::Failed {
            received: String::new(),
            computed: "frame too short".into(),
        };
        return f;
    }
    let n = bytes.len();
    let rx = u16::from_le_bytes([bytes[n - 2], bytes[n - 1]]);
    let calc = crc16_modbus(&bytes[..n - 2]);
    f.fields.push(super::Field {
        name: "slave".into(),
        value: bytes[0].to_string(),
    });
    let desc = describe_pdu(&bytes[1..n - 2], role, &mut f);
    let crc = if rx == calc {
        "[CRC:OK]".to_owned()
    } else {
        format!("[CRC:FAIL rx:{rx:04X} calc:{calc:04X}]")
    };
    f.check = if rx == calc {
        Check::Ok
    } else {
        Check::Failed {
            received: format!("{rx:04X}"),
            computed: format!("{calc:04X}"),
        }
    };
    f.summary = format!("[MODBUS-RTU] Slave:{:02X} {desc} {crc}", bytes[0]);
    f
}

/// Modbus RTU decoder: CRC-synchronised framing, flushed by the inter-frame gap.
#[derive(Debug)]
pub struct RtuDecoder {
    role: Role,
    buf: Vec<u8>,
    last_us: Option<i64>,
    gap_us: i64,
}

impl RtuDecoder {
    /// New decoder; `baud` sets the 3.5-character gap (at least 25 ms, USB adapters batch reads).
    pub fn new(role: Role, baud: Option<u32>) -> Self {
        let t35 = baud.map_or(2000, |b| {
            if b > 19_200 {
                1750
            } else {
                (38_500_000 / i64::from(b.max(1))).max(1)
            }
        });
        Self {
            role,
            buf: Vec::new(),
            last_us: None,
            gap_us: t35.max(25_000),
        }
    }

    fn extract(&mut self) -> Vec<Frame> {
        let mut out = Vec::new();
        loop {
            let lens = rtu_lengths(&self.buf, self.role);
            let Some(n) = lens
                .iter()
                .copied()
                .find(|&n| self.buf.len() >= n && crc_ok(&self.buf[..n]))
            else {
                break;
            };
            let frame: Vec<u8> = self.buf.drain(..n).collect();
            let role = if rtu_lengths(&frame, self.role).first() == Some(&n) {
                self.role
            } else {
                other(self.role)
            };
            out.push(rtu_frame_out(&frame, role));
        }
        if self.buf.len() > 256 {
            out.push(rtu_frame_out(&std::mem::take(&mut self.buf), self.role));
        }
        out
    }

    fn flush(&mut self) -> Vec<Frame> {
        if self.buf.is_empty() {
            Vec::new()
        } else {
            vec![rtu_frame_out(&std::mem::take(&mut self.buf), self.role)]
        }
    }
}

fn other(r: Role) -> Role {
    match r {
        Role::Request => Role::Response,
        Role::Response => Role::Request,
    }
}

impl Decoder for RtuDecoder {
    fn protocol(&self) -> Protocol {
        Protocol::ModbusRtu
    }

    fn feed(&mut self, data: &[u8], ts_us: i64) -> Vec<Frame> {
        let mut out = Vec::new();
        if self.last_us.is_some_and(|t| ts_us - t >= self.gap_us) {
            out.extend(self.flush());
        }
        self.last_us = Some(ts_us);
        self.buf.extend_from_slice(data);
        out.extend(self.extract());
        out
    }

    fn poll(&mut self, ts_us: i64) -> Vec<Frame> {
        if self.last_us.is_some_and(|t| ts_us - t >= self.gap_us) {
            self.flush()
        } else {
            Vec::new()
        }
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.last_us = None;
    }
}

fn decode_hex_pairs(s: &[u8]) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_chunks::<2>()
        .0
        .iter()
        .map(|p| super::parse_hex(p).and_then(|v| u8::try_from(v).ok()))
        .collect()
}

pub(super) fn ascii_looks_valid(line: &[u8]) -> bool {
    line.first() == Some(&b':')
        && decode_hex_pairs(&line[1..])
            .is_some_and(|b| b.len() >= 3 && lrc(&b[..b.len() - 1]) == b[b.len() - 1])
}

/// Modbus ASCII decoder.
#[derive(Debug)]
pub struct AsciiDecoder {
    role: Role,
    lines: LineAssembler,
}

impl AsciiDecoder {
    /// New decoder.
    pub fn new(role: Role) -> Self {
        Self {
            role,
            lines: LineAssembler::new(600),
        }
    }
}

fn ascii_frame_out(line: &[u8], role: Role) -> Option<Frame> {
    let body = line.strip_prefix(b":")?;
    let mut f = Frame::new(Protocol::ModbusAscii, line, String::new());
    let Some(bytes) = decode_hex_pairs(body).filter(|b| b.len() >= 3) else {
        f.summary = format!(
            "[MODBUS-ASCII] malformed frame: {}",
            String::from_utf8_lossy(line)
        );
        f.check = Check::Failed {
            received: String::new(),
            computed: "not hex".into(),
        };
        return Some(f);
    };
    let n = bytes.len();
    let (rx, calc) = (bytes[n - 1], lrc(&bytes[..n - 1]));
    f.fields.push(super::Field {
        name: "slave".into(),
        value: bytes[0].to_string(),
    });
    let desc = describe_pdu(&bytes[1..n - 1], role, &mut f);
    let check = if rx == calc {
        "[LRC:OK]".to_owned()
    } else {
        format!("[LRC:FAIL rx:{rx:02X} calc:{calc:02X}]")
    };
    f.check = if rx == calc {
        Check::Ok
    } else {
        Check::Failed {
            received: format!("{rx:02X}"),
            computed: format!("{calc:02X}"),
        }
    };
    f.summary = format!("[MODBUS-ASCII] Slave:{:02X} {desc} {check}", bytes[0]);
    Some(f)
}

impl Decoder for AsciiDecoder {
    fn protocol(&self) -> Protocol {
        Protocol::ModbusAscii
    }

    fn feed(&mut self, data: &[u8], _ts_us: i64) -> Vec<Frame> {
        let role = self.role;
        self.lines
            .feed(data)
            .iter()
            .filter_map(|l| ascii_frame_out(l, role))
            .collect()
    }

    fn reset(&mut self) {
        self.lines.reset();
    }
}

/// Modbus TCP decoder (MBAP header: transaction, protocol 0, length, unit).
#[derive(Debug)]
pub struct TcpDecoder {
    role: Role,
    buf: Vec<u8>,
}

impl TcpDecoder {
    /// New decoder.
    pub fn new(role: Role) -> Self {
        Self {
            role,
            buf: Vec::new(),
        }
    }
}

fn tcp_frame_len(buf: &[u8]) -> Result<Option<usize>, ()> {
    if buf.len() < 7 {
        return Ok(None);
    }
    let pid = u16::from_be_bytes([buf[2], buf[3]]);
    let len = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    if pid != 0 || !(2..=254).contains(&len) {
        return Err(());
    }
    Ok((buf.len() >= 6 + len).then_some(6 + len))
}

impl Decoder for TcpDecoder {
    fn protocol(&self) -> Protocol {
        Protocol::ModbusTcp
    }

    fn feed(&mut self, data: &[u8], _ts_us: i64) -> Vec<Frame> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            match tcp_frame_len(&self.buf) {
                Ok(Some(n)) => {
                    let b: Vec<u8> = self.buf.drain(..n).collect();
                    let mut f = Frame::new(Protocol::ModbusTcp, &b, String::new());
                    let tid = u16::from_be_bytes([b[0], b[1]]);
                    f.fields.push(super::Field {
                        name: "transaction".into(),
                        value: tid.to_string(),
                    });
                    f.fields.push(super::Field {
                        name: "unit".into(),
                        value: b[6].to_string(),
                    });
                    let desc = describe_pdu(&b[7..], self.role, &mut f);
                    f.summary = format!("[Modbus TCP] TID:0x{tid:04X} Unit:{} Len:{} {desc}", b[6], n - 6);
                    out.push(f);
                }
                Ok(None) => break,
                Err(()) => {
                    // Not an MBAP header here: resynchronise one byte later.
                    self.buf.remove(0);
                }
            }
        }
        if self.buf.len() > 4096 {
            self.buf.clear();
        }
        out
    }

    fn reset(&mut self) {
        self.buf.clear();
    }
}

/// Share of `sample` covered by CRC-valid RTU frames.
pub(super) fn rtu_score(sample: &[u8]) -> f32 {
    if sample.len() < 4 {
        return 0.0;
    }
    let (mut i, mut covered) = (0, 0);
    while i + 4 <= sample.len() {
        let rest = &sample[i..];
        let mut lens = rtu_lengths(rest, Role::Request);
        lens.extend(rtu_lengths(rest, Role::Response));
        if let Some(n) = lens.into_iter().find(|&n| rest.len() >= n && crc_ok(&rest[..n])) {
            covered += n;
            i += n;
        } else {
            i += 1;
        }
    }
    covered as f32 / sample.len() as f32
}

/// Share of `sample` covered by consecutive valid MBAP frames from the start.
pub(super) fn tcp_score(sample: &[u8]) -> f32 {
    let (mut i, mut covered) = (0, 0);
    while let Ok(Some(n)) = tcp_frame_len(&sample[i..]) {
        if sample.len() < i + 8 {
            break;
        }
        covered += n;
        i += n;
    }
    if sample.is_empty() {
        0.0
    } else {
        covered as f32 / sample.len() as f32
    }
}

// ----- requests and responses -----

/// A Modbus request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub enum Request {
    /// 0x01.
    ReadCoils {
        /// First coil.
        address: u16,
        /// Number of coils (1-2000).
        count: u16,
    },
    /// 0x02.
    ReadDiscreteInputs {
        /// First input.
        address: u16,
        /// Number of inputs (1-2000).
        count: u16,
    },
    /// 0x03.
    ReadHoldingRegisters {
        /// First register.
        address: u16,
        /// Number of registers (1-125).
        count: u16,
    },
    /// 0x04.
    ReadInputRegisters {
        /// First register.
        address: u16,
        /// Number of registers (1-125).
        count: u16,
    },
    /// 0x05.
    WriteSingleCoil {
        /// Coil.
        address: u16,
        /// On or off.
        value: bool,
    },
    /// 0x06.
    WriteSingleRegister {
        /// Register.
        address: u16,
        /// Value.
        value: u16,
    },
    /// 0x0F.
    WriteMultipleCoils {
        /// First coil.
        address: u16,
        /// Values.
        values: Vec<bool>,
    },
    /// 0x10.
    WriteMultipleRegisters {
        /// First register.
        address: u16,
        /// Values.
        values: Vec<u16>,
    },
}

impl Request {
    /// Function code.
    pub fn function(&self) -> u8 {
        match self {
            Self::ReadCoils { .. } => 1,
            Self::ReadDiscreteInputs { .. } => 2,
            Self::ReadHoldingRegisters { .. } => 3,
            Self::ReadInputRegisters { .. } => 4,
            Self::WriteSingleCoil { .. } => 5,
            Self::WriteSingleRegister { .. } => 6,
            Self::WriteMultipleCoils { .. } => 0x0F,
            Self::WriteMultipleRegisters { .. } => 0x10,
        }
    }

    /// Encode the PDU, validating counts.
    pub fn pdu(&self) -> Result<Vec<u8>, String> {
        let fc = self.function();
        let mut p = vec![fc];
        let push16 = |p: &mut Vec<u8>, v: u16| p.extend_from_slice(&v.to_be_bytes());
        match self {
            Self::ReadCoils { address, count } | Self::ReadDiscreteInputs { address, count } => {
                if !(1..=2000).contains(count) {
                    return Err(format!("count {count} must be 1-2000"));
                }
                push16(&mut p, *address);
                push16(&mut p, *count);
            }
            Self::ReadHoldingRegisters { address, count } | Self::ReadInputRegisters { address, count } => {
                if !(1..=125).contains(count) {
                    return Err(format!("count {count} must be 1-125"));
                }
                push16(&mut p, *address);
                push16(&mut p, *count);
            }
            Self::WriteSingleCoil { address, value } => {
                push16(&mut p, *address);
                push16(&mut p, if *value { 0xFF00 } else { 0 });
            }
            Self::WriteSingleRegister { address, value } => {
                push16(&mut p, *address);
                push16(&mut p, *value);
            }
            Self::WriteMultipleCoils { address, values } => {
                if !(1..=1968).contains(&values.len()) {
                    return Err("1-1968 coils".into());
                }
                push16(&mut p, *address);
                push16(&mut p, values.len() as u16);
                let bytes: Vec<u8> = values
                    .chunks(8)
                    .map(|c| {
                        c.iter()
                            .enumerate()
                            .fold(0u8, |acc, (i, &on)| acc | (u8::from(on) << i))
                    })
                    .collect();
                p.push(bytes.len() as u8);
                p.extend(bytes);
            }
            Self::WriteMultipleRegisters { address, values } => {
                if !(1..=123).contains(&values.len()) {
                    return Err("1-123 registers".into());
                }
                push16(&mut p, *address);
                push16(&mut p, values.len() as u16);
                p.push((values.len() * 2) as u8);
                for v in values {
                    push16(&mut p, *v);
                }
            }
        }
        Ok(p)
    }
}

/// RTU frame: unit, PDU, CRC (low byte first).
pub fn rtu_frame(unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut f = vec![unit];
    f.extend_from_slice(pdu);
    let crc = crc16_modbus(&f);
    f.extend_from_slice(&crc.to_le_bytes());
    f
}

/// ASCII frame: `:` HEX(unit PDU LRC) CR LF.
pub fn ascii_frame(unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut bin = vec![unit];
    bin.extend_from_slice(pdu);
    bin.push(lrc(&bin));
    let mut s = String::from(":");
    for b in bin {
        let _ = write!(s, "{b:02X}");
    }
    s.push_str("\r\n");
    s.into_bytes()
}

/// TCP frame with MBAP header.
pub fn tcp_frame(transaction: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(7 + pdu.len());
    f.extend_from_slice(&transaction.to_be_bytes());
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(&((pdu.len() + 1) as u16).to_be_bytes());
    f.push(unit);
    f.extend_from_slice(pdu);
    f
}

/// Unit and PDU of a decoded frame's bytes (`None` if malformed or failing its check).
pub fn application_data(protocol: Protocol, bytes: &[u8]) -> Option<(u8, Vec<u8>, Option<u16>)> {
    match protocol {
        Protocol::ModbusRtu if crc_ok(bytes) => Some((bytes[0], bytes[1..bytes.len() - 2].to_vec(), None)),
        Protocol::ModbusAscii => {
            let b = decode_hex_pairs(bytes.strip_prefix(b":")?)?;
            (b.len() >= 3 && lrc(&b[..b.len() - 1]) == b[b.len() - 1])
                .then(|| (b[0], b[1..b.len() - 1].to_vec(), None))
        }
        Protocol::ModbusTcp if bytes.len() >= 8 => Some((
            bytes[6],
            bytes[7..].to_vec(),
            Some(u16::from_be_bytes([bytes[0], bytes[1]])),
        )),
        _ => None,
    }
}

/// Parsed response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    /// Coils / discrete inputs.
    Bits {
        /// Values, first address first.
        values: Vec<bool>,
    },
    /// Holding / input registers.
    Registers {
        /// Values, first address first.
        values: Vec<u16>,
    },
    /// Write acknowledged.
    Written {
        /// Address echoed by the device.
        address: u16,
        /// Value or count echoed by the device.
        value: u16,
    },
    /// Exception response.
    Exception {
        /// Exception code.
        code: u8,
        /// Meaning.
        name: String,
    },
}

/// Interpret `pdu` as the response to `req`. `Ok(None)` means: not a response to this request.
pub fn parse_response(req: &Request, pdu: &[u8]) -> Result<Option<Response>, String> {
    let fc = req.function();
    let Some(&rfc) = pdu.first() else { return Ok(None) };
    if rfc == fc | 0x80 {
        let code = pdu.get(1).copied().unwrap_or(0);
        return Ok(Some(Response::Exception {
            code,
            name: exception_name(code).into(),
        }));
    }
    if rfc != fc {
        return Ok(None);
    }
    let data = &pdu[1..];
    match req {
        Request::ReadCoils { count, .. } | Request::ReadDiscreteInputs { count, .. } => {
            let n = *data.first().ok_or("empty response")? as usize;
            let bytes = data.get(1..1 + n).ok_or("response shorter than its byte count")?;
            let values = (0..*count as usize)
                .map(|i| bytes.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 == 1))
                .collect();
            Ok(Some(Response::Bits { values }))
        }
        Request::ReadHoldingRegisters { count, .. } | Request::ReadInputRegisters { count, .. } => {
            let n = *data.first().ok_or("empty response")? as usize;
            let bytes = data.get(1..1 + n).ok_or("response shorter than its byte count")?;
            let values: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            if values.len() != *count as usize {
                return Err(format!("expected {count} registers, got {}", values.len()));
            }
            Ok(Some(Response::Registers { values }))
        }
        _ => {
            let address = u16_at(data, 0).ok_or("short write response")?;
            let value = u16_at(data, 2).ok_or("short write response")?;
            Ok(Some(Response::Written { address, value }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::decoder;
    use super::*;

    #[test]
    fn rtu_request_and_response() {
        let req = rtu_frame(
            1,
            &Request::ReadHoldingRegisters {
                address: 0,
                count: 10,
            }
            .pdu()
            .unwrap(),
        );
        assert_eq!(req, [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]);
        let mut tx = decoder(Protocol::ModbusRtu, Role::Request, Some(19_200));
        let f = tx.feed(&req, 0);
        assert_eq!(
            f[0].summary,
            "[MODBUS-RTU] Slave:01 Func:03 (Read Holding Registers) Addr:0x0000 Count:10 [CRC:OK]"
        );
        let resp = rtu_frame(1, &[0x03, 0x04, 0x00, 0x2A, 0x01, 0x00]);
        let mut rx = decoder(Protocol::ModbusRtu, Role::Response, Some(19_200));
        // Split across reads, then a second frame glued to the first.
        assert!(rx.feed(&resp[..3], 0).is_empty());
        let mut tail = resp[3..].to_vec();
        tail.extend(rtu_frame(2, &[0x83, 0x02]));
        let frames = rx.feed(&tail, 1_000);
        assert_eq!(frames.len(), 2);
        assert_eq!(
            frames[0].summary,
            "[MODBUS-RTU] Slave:01 Func:03 (Read Holding Registers) [2 registers] 0x002A 0x0100 [CRC:OK]"
        );
        assert_eq!(
            frames[1].summary,
            "[MODBUS-RTU] Slave:02 Func:83 (Read Holding Registers) Exception:0x02 (Illegal Data Address) [CRC:OK]"
        );
    }

    #[test]
    fn rtu_gap_flushes_garbage_with_crc_failure() {
        let mut rx = RtuDecoder::new(Role::Response, Some(9600));
        assert!(rx.feed(&[0x01, 0x03, 0x02], 0).is_empty());
        assert!(rx.poll(10_000).is_empty(), "within the gap");
        let f = rx.poll(100_000);
        assert_eq!(f.len(), 1);
        assert!(matches!(f[0].check, Check::Failed { .. }));
        assert!(f[0].summary.starts_with("[MODBUS-RTU] incomplete frame"));
    }

    #[test]
    fn ascii_frames_lowercase_and_lrc() {
        let pdu = Request::ReadHoldingRegisters { address: 0, count: 1 }
            .pdu()
            .unwrap();
        assert_eq!(ascii_frame(1, &pdu), b":010300000001FB\r\n");
        let mut d = decoder(Protocol::ModbusAscii, Role::Request, None);
        let f = d.feed(b":010300000001fb\r\n:010300000001FC\r\n", 0);
        assert_eq!(f[0].check, Check::Ok, "lowercase hex accepted (C decoder failed)");
        assert_eq!(
            f[0].summary,
            "[MODBUS-ASCII] Slave:01 Func:03 (Read Holding Registers) Addr:0x0000 Count:1 [LRC:OK]"
        );
        assert!(f[1].summary.ends_with("[LRC:FAIL rx:FC calc:FB]"));
    }

    #[test]
    fn tcp_framing_and_resync() {
        let pdu = Request::WriteSingleRegister { address: 1, value: 3 }
            .pdu()
            .unwrap();
        let mut stream = vec![0xFF];
        stream.extend(tcp_frame(7, 1, &pdu));
        stream.extend(tcp_frame(8, 1, &pdu));
        let mut d = decoder(Protocol::ModbusTcp, Role::Request, None);
        let mut frames = d.feed(&stream[..10], 0);
        frames.extend(d.feed(&stream[10..], 0));
        assert_eq!(frames.len(), 2);
        assert_eq!(
            frames[0].summary,
            "[Modbus TCP] TID:0x0007 Unit:1 Len:6 Func:06 (Write Single Register) [Addr:0x0001 Val:0x0003]"
        );
        assert_eq!(
            application_data(Protocol::ModbusTcp, &frames[1].bytes).unwrap().2,
            Some(8)
        );
    }

    #[test]
    fn requests_and_responses() {
        let coils = Request::WriteMultipleCoils {
            address: 19,
            values: vec![true, false, true, true, false, false, true, true, true, false],
        };
        assert_eq!(
            coils.pdu().unwrap(),
            [0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01]
        );
        assert!(
            Request::ReadHoldingRegisters {
                address: 0,
                count: 126
            }
            .pdu()
            .is_err()
        );
        let r = Request::ReadHoldingRegisters { address: 0, count: 2 };
        assert_eq!(
            parse_response(&r, &[0x03, 0x04, 0x00, 0x2A, 0x01, 0x00]).unwrap(),
            Some(Response::Registers {
                values: vec![42, 256]
            })
        );
        assert_eq!(
            parse_response(&r, &[0x83, 0x02]).unwrap(),
            Some(Response::Exception {
                code: 2,
                name: "Illegal Data Address".into()
            })
        );
        assert_eq!(
            parse_response(&r, &[0x04, 0x02, 0, 0]).unwrap(),
            None,
            "other function: not our response"
        );
        assert!(
            parse_response(&r, &[0x03, 0x02, 0x00, 0x01]).is_err(),
            "wrong register count"
        );
        let c = Request::ReadCoils {
            address: 0,
            count: 10,
        };
        let Some(Response::Bits { values }) = parse_response(&c, &[0x01, 0x02, 0xCD, 0x01]).unwrap() else {
            panic!()
        };
        assert_eq!(
            values,
            [true, false, true, true, false, false, true, true, true, false]
        );
        let w = Request::WriteSingleRegister { address: 1, value: 3 };
        assert_eq!(
            parse_response(&w, &[0x06, 0, 1, 0, 3]).unwrap(),
            Some(Response::Written { address: 1, value: 3 })
        );
    }

    #[test]
    fn scores() {
        let mut s = rtu_frame(1, &[3, 0, 0, 0, 10]);
        s.extend(rtu_frame(1, &[3, 2, 0, 42]));
        assert!(rtu_score(&s) > 0.9);
        assert!(rtu_score(b"hello world this is text") < 0.2);
        assert!(tcp_score(&tcp_frame(1, 1, &[3, 0, 0, 0, 1])) > 0.9);
    }
}
