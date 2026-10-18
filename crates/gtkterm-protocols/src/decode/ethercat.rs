//! EtherCAT frames: 2-byte header (length, type) followed by datagrams. All datagrams are decoded
//! (the C version stopped after the first).

use std::fmt::Write as _;

use super::{Decoder, Frame, Protocol, hex};

const COMMANDS: &[(u8, &str)] = &[
    (0x00, "NOP"),
    (0x01, "APRD"),
    (0x02, "APWR"),
    (0x03, "APRW"),
    (0x04, "FPRD"),
    (0x05, "FPWR"),
    (0x06, "FPRW"),
    (0x07, "BRD"),
    (0x08, "BWR"),
    (0x09, "BRW"),
    (0x0A, "LRD"),
    (0x0B, "LWR"),
    (0x0C, "LRW"),
    (0x0D, "ARMW"),
    (0x0E, "FRMW"),
];

/// Largest EtherCAT payload in an Ethernet frame.
const MAX_LEN: usize = 1498;

/// Checks the datagram chain visible so far against the header length, so a garbage byte that
/// happens to look like a header does not stall the stream waiting for a bogus length.
fn plausible(b: &[u8], len: usize) -> bool {
    let end = 2 + len;
    let mut i = 2;
    while i + 10 <= b.len() {
        if b[i] > 0x0E {
            return false;
        }
        let lf = u16::from_le_bytes([b[i + 6], b[i + 7]]);
        let next = i + 12 + usize::from(lf & 0x07FF);
        if next > end || (lf & 0x8000 == 0 && next != end && b.len() >= end) {
            return false;
        }
        if lf & 0x8000 == 0 {
            return true;
        }
        i = next;
    }
    i < b.len() || b.len() < end
}

/// Reassembles length-prefixed EtherCAT frames.
#[derive(Debug, Default)]
pub struct EtherCatDecoder {
    buf: Vec<u8>,
}

fn decode_frame(b: &[u8]) -> Frame {
    let header = u16::from_le_bytes([b[0], b[1]]);
    let (len, kind) = ((header & 0x07FF) as usize, header >> 12);
    let mut parts = Vec::new();
    let mut f = Frame::new(Protocol::EtherCat, b, String::new())
        .field("type", kind.to_string())
        .field("length", len.to_string());
    let mut i = 2;
    while i + 10 <= b.len() {
        let cmd = b[i];
        let idx = b[i + 1];
        let addr = u32::from_le_bytes([b[i + 2], b[i + 3], b[i + 4], b[i + 5]]);
        let lf = u16::from_le_bytes([b[i + 6], b[i + 7]]);
        let dlen = (lf & 0x07FF) as usize;
        let (circulating, more) = (lf & 0x4000 != 0, lf & 0x8000 != 0);
        let irq = u16::from_le_bytes([b[i + 8], b[i + 9]]);
        let data_end = i + 10 + dlen;
        let Some(data) = b.get(i + 10..data_end) else {
            parts.push("truncated datagram".to_owned());
            break;
        };
        let wkc = b
            .get(data_end..data_end + 2)
            .map(|w| u16::from_le_bytes([w[0], w[1]]));
        let name = COMMANDS
            .iter()
            .find(|(c, _)| *c == cmd)
            .map_or_else(|| format!("0x{cmd:02X}"), |(_, n)| (*n).to_owned());
        let mut p = format!("Cmd:{name} Idx:{idx} Addr:0x{addr:08X} DLen:{dlen}");
        if irq != 0 {
            let _ = write!(p, " IRQ:0x{irq:04X}");
        }
        if !data.is_empty() {
            let _ = write!(p, " Data: {}", hex(&data[..data.len().min(16)]));
        }
        if let Some(w) = wkc {
            let _ = write!(p, " WKC:{w}");
        }
        if circulating {
            p.push_str(" [CIRCULATING]");
        }
        parts.push(p);
        f = f.field(&format!("datagram{}", parts.len()), name);
        i = data_end + 2;
        if !more {
            break;
        }
    }
    f.summary = format!("[EtherCAT] Type:{kind} Len:{len} {}", parts.join(" | "));
    f
}

impl Decoder for EtherCatDecoder {
    fn protocol(&self) -> Protocol {
        Protocol::EtherCat
    }

    fn feed(&mut self, data: &[u8], _ts_us: i64) -> Vec<Frame> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        // Scan with an index and drop the consumed prefix once: removing bytes one by one
        // while resynchronising on noise would be quadratic.
        let mut i = 0;
        while self.buf.len() - i >= 2 {
            let b = &self.buf[i..];
            let header = u16::from_le_bytes([b[0], b[1]]);
            let (len, kind) = ((header & 0x07FF) as usize, header >> 12);
            if kind != 1 || !(12..=MAX_LEN).contains(&len) || !plausible(&b[..b.len().min(2 + len)], len) {
                i += 1; // resynchronise
                continue;
            }
            if b.len() < 2 + len {
                break;
            }
            out.push(decode_frame(&b[..2 + len]));
            i += 2 + len;
        }
        self.buf.drain(..i);
        out
    }

    fn reset(&mut self) {
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datagram(cmd: u8, idx: u8, data: &[u8], more: bool, wkc: u16) -> Vec<u8> {
        let mut d = vec![cmd, idx, 0x00, 0x00, 0x30, 0x01];
        let lf = data.len() as u16 | if more { 0x8000 } else { 0 };
        d.extend_from_slice(&lf.to_le_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(data);
        d.extend_from_slice(&wkc.to_le_bytes());
        d
    }

    /// Noise that looks like EtherCAT headers at every offset costs linear time to skip.
    #[test]
    fn resync_on_noise_is_fast() {
        let mut d = EtherCatDecoder::default();
        let noise: Vec<u8> = [0x40, 0x10].iter().copied().cycle().take(256 * 1024).collect();
        let started = std::time::Instant::now();
        assert!(d.feed(&noise, 0).is_empty());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        assert!(d.buf.len() < 2 + MAX_LEN, "skipped noise is dropped");
    }

    #[test]
    fn decodes_all_datagrams() {
        let mut body = datagram(0x07, 0, &[0x11, 0x22], true, 3);
        body.extend(datagram(0x0C, 1, &[0xAA], false, 1));
        let header = (body.len() as u16) | (1 << 12);
        let mut frame = header.to_le_bytes().to_vec();
        frame.extend(&body);
        let mut d = EtherCatDecoder::default();
        let mut stream = vec![0x00];
        stream.extend(&frame);
        let out = d.feed(&stream, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].summary,
            "[EtherCAT] Type:1 Len:27 Cmd:BRD Idx:0 Addr:0x01300000 DLen:2 Data: 11 22 WKC:3 | Cmd:LRW Idx:1 Addr:0x01300000 DLen:1 Data: AA WKC:1"
        );
    }
}
