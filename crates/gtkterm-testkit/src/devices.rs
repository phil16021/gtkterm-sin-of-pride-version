//! Fake devices for manual tests and integration tests: a Modbus slave, an AT modem and an
//! NMEA GPS. They are sans-IO: feed received bytes, get the bytes to send back.

use std::fmt::Write as _;

use gtkterm_protocols::checksum::nmea_xor;
use gtkterm_protocols::decode::{Decoder, LineAssembler, Protocol, Role, decoder, modbus};

/// A Modbus slave with 256 holding/input registers and 256 coils.
#[derive(Debug)]
pub struct ModbusSlave {
    /// Unit ID it answers to.
    pub unit: u8,
    /// Holding registers (input registers read the same values + 1000).
    pub registers: [u16; 256],
    /// Coils (discrete inputs read the inverse).
    pub coils: [bool; 256],
    framing: Protocol,
    rx: Box<dyn Decoder>,
}

impl ModbusSlave {
    /// Slave for `unit` speaking `framing` (`ModbusRtu`, `ModbusAscii` or `ModbusTcp`).
    pub fn new(unit: u8, framing: Protocol) -> Self {
        let mut registers = [0u16; 256];
        for (i, r) in registers.iter_mut().enumerate() {
            *r = u16::try_from(i).unwrap_or(0) * 10;
        }
        Self {
            unit,
            registers,
            coils: [false; 256],
            framing,
            rx: decoder(framing, Role::Request, Some(19_200)),
        }
    }

    /// Feed received bytes; returns the responses to send.
    pub fn feed(&mut self, data: &[u8], ts_us: i64) -> Vec<Vec<u8>> {
        let (framing, me) = (self.framing, self.unit);
        let requests: Vec<_> = self
            .rx
            .feed(data, ts_us)
            .iter()
            .filter_map(|f| modbus::application_data(framing, &f.bytes))
            .filter(|(unit, _, _)| *unit == me)
            .collect();
        requests
            .into_iter()
            .map(|(unit, pdu, tid)| {
                let reply = self.handle(&pdu);
                match self.framing {
                    Protocol::ModbusAscii => modbus::ascii_frame(unit, &reply),
                    Protocol::ModbusTcp => modbus::tcp_frame(tid.unwrap_or(0), unit, &reply),
                    _ => modbus::rtu_frame(unit, &reply),
                }
            })
            .collect()
    }

    /// Frames completed by silence (RTU).
    pub fn poll(&mut self, ts_us: i64) {
        let _ = self.rx.poll(ts_us);
    }

    fn handle(&mut self, pdu: &[u8]) -> Vec<u8> {
        let fc = pdu.first().copied().unwrap_or(0);
        let u16_at = |i: usize| pdu.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
        let exception = |code: u8| vec![fc | 0x80, code];
        let (Some(addr), Some(arg)) = (u16_at(1), u16_at(3)) else {
            return exception(3);
        };
        let (a, n) = (usize::from(addr), usize::from(arg));
        match fc {
            1 | 2 => {
                if n == 0 || n > 2000 || a + n > 256 {
                    return exception(2);
                }
                let mut bytes = vec![0u8; n.div_ceil(8)];
                for i in 0..n {
                    if self.coils[a + i] != (fc == 2) {
                        bytes[i / 8] |= 1 << (i % 8);
                    }
                }
                let mut out = vec![fc, u8::try_from(bytes.len()).unwrap_or(0)];
                out.extend(bytes);
                out
            }
            3 | 4 => {
                if n == 0 || n > 125 || a + n > 256 {
                    return exception(2);
                }
                let mut out = vec![fc, u8::try_from(n * 2).unwrap_or(0)];
                for r in &self.registers[a..a + n] {
                    let v = if fc == 4 { r.wrapping_add(1000) } else { *r };
                    out.extend_from_slice(&v.to_be_bytes());
                }
                out
            }
            5 => {
                if a >= 256 || !(arg == 0xFF00 || arg == 0) {
                    return exception(if a >= 256 { 2 } else { 3 });
                }
                self.coils[a] = arg == 0xFF00;
                pdu[..5].to_vec()
            }
            6 => {
                if a >= 256 {
                    return exception(2);
                }
                self.registers[a] = arg;
                pdu[..5].to_vec()
            }
            15 | 16 => {
                let data = pdu.get(6..).unwrap_or(&[]);
                if a + n > 256 || n == 0 {
                    return exception(2);
                }
                for i in 0..n {
                    if fc == 15 {
                        self.coils[a + i] = data.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 == 1);
                    } else if let Some(v) = data.get(i * 2..i * 2 + 2) {
                        self.registers[a + i] = u16::from_be_bytes([v[0], v[1]]);
                    }
                }
                pdu[..5].to_vec()
            }
            _ => exception(1),
        }
    }
}

/// A Hayes modem that answers a handful of commands.
#[derive(Debug)]
pub struct AtModem {
    lines: LineAssembler,
    echo: bool,
}

impl Default for AtModem {
    fn default() -> Self {
        Self {
            lines: LineAssembler::new(256),
            echo: true,
        }
    }
}

impl AtModem {
    /// Feed received bytes; returns the reply bytes.
    pub fn feed(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in self.lines.feed(data) {
            let cmd = String::from_utf8_lossy(&line).trim().to_ascii_uppercase();
            if cmd.is_empty() {
                continue;
            }
            if self.echo {
                out.extend_from_slice(cmd.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            let reply = match cmd.as_str() {
                "AT" | "ATZ" => "OK",
                "ATE0" => {
                    self.echo = false;
                    "OK"
                }
                "ATE1" => {
                    self.echo = true;
                    "OK"
                }
                "ATI" => "GTKTerm fake modem 1.0\r\nOK",
                "AT+GMR" => "+GMR: 1.0.0\r\nOK",
                "AT+CSQ" => "+CSQ: 23,99\r\nOK",
                "AT+CREG?" => "+CREG: 0,1\r\nOK",
                "AT+CPIN?" => "+CPIN: READY\r\nOK",
                c if c.starts_with("AT+CMGS") => "+CMS ERROR: 330",
                _ => "ERROR",
            };
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(reply.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out
    }
}

/// NMEA GGA + RMC sentences for second `t` along a small circle near Munich.
pub fn nmea_fix(t: u64) -> String {
    let angle = (t % 360) as f64 * std::f64::consts::PI / 180.0;
    let lat = 48.1173 + 0.001 * angle.sin();
    let lon = 11.5167 + 0.001 * angle.cos();
    let fmt_coord = |v: f64, deg_digits: usize| {
        let deg = v.trunc();
        let min = (v - deg) * 60.0;
        format!("{:0w$}{:07.4}", deg as u32, min, w = deg_digits)
    };
    let (h, m, s) = ((12 + t / 3600) % 24, (t / 60) % 60, t % 60);
    let mut out = String::new();
    for body in [
        format!(
            "GPGGA,{h:02}{m:02}{s:02}.00,{},N,{},E,1,08,0.9,545.4,M,46.9,M,,",
            fmt_coord(lat, 2),
            fmt_coord(lon, 3)
        ),
        format!(
            "GPRMC,{h:02}{m:02}{s:02}.00,A,{},N,{},E,000.5,{:05.1},230394,003.1,W",
            fmt_coord(lat, 2),
            fmt_coord(lon, 3),
            (t % 360) as f64
        ),
    ] {
        let _ = write!(out, "${body}*{:02X}\r\n", nmea_xor(body.as_bytes()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtkterm_protocols::decode::{Check, detect_best};

    #[test]
    fn modbus_slave_answers_reads_and_writes() {
        let mut s = ModbusSlave::new(1, Protocol::ModbusRtu);
        let req = |pdu: &[u8]| modbus::rtu_frame(1, pdu);
        let out = s.feed(&req(&[0x03, 0x00, 0x02, 0x00, 0x02]), 0);
        assert_eq!(out, [modbus::rtu_frame(1, &[0x03, 0x04, 0x00, 20, 0x00, 30])]);
        let out = s.feed(&req(&[0x06, 0x00, 0x02, 0x12, 0x34]), 0);
        assert_eq!(out, [req(&[0x06, 0x00, 0x02, 0x12, 0x34])]);
        assert_eq!(s.registers[2], 0x1234);
        let out = s.feed(&req(&[0x03, 0x01, 0x00, 0x00, 0x01]), 0);
        assert_eq!(out, [modbus::rtu_frame(1, &[0x83, 0x02])], "address out of range");
        assert!(
            s.feed(&modbus::rtu_frame(2, &[0x03, 0, 0, 0, 1]), 0).is_empty(),
            "other unit"
        );
        let mut tcp = ModbusSlave::new(5, Protocol::ModbusTcp);
        let out = tcp.feed(&modbus::tcp_frame(77, 5, &[0x05, 0x00, 0x03, 0xFF, 0x00]), 0);
        assert_eq!(out, [modbus::tcp_frame(77, 5, &[0x05, 0x00, 0x03, 0xFF, 0x00])]);
        assert!(tcp.coils[3]);
    }

    #[test]
    fn at_modem_replies() {
        let mut m = AtModem::default();
        assert_eq!(m.feed(b"ATE0\r"), b"ATE0\r\n\r\nOK\r\n");
        assert_eq!(m.feed(b"AT+CSQ\r"), b"\r\n+CSQ: 23,99\r\nOK\r\n");
        assert_eq!(m.feed(b"AT+NOPE\r"), b"\r\nERROR\r\n");
    }

    #[test]
    fn nmea_sentences_are_valid() {
        for t in [0, 59, 3_599, 86_399] {
            let text = nmea_fix(t);
            assert_eq!(detect_best(text.as_bytes(), 0.6), Some(Protocol::Nmea), "{text}");
            let mut d = decoder(Protocol::Nmea, Role::Response, None);
            for f in d.feed(text.as_bytes(), 0) {
                assert_eq!(f.check, Check::Ok, "{}", f.summary);
            }
        }
    }
}
