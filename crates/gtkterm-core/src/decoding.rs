//! Protocol decoding for a session: one decoder per direction, auto-detection, modem line
//! frames and a ring of decoded frames with monotonic sequence numbers.
//!
//! Transmitted data is decoded as requests and received data as responses, so a Modbus master
//! session shows both sides of every transaction.

use std::collections::VecDeque;
use std::fmt;
use std::str::FromStr;

use gtkterm_protocols::decode::{self, Check, Decoder, Frame, Protocol, Role};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::port::ModemLines;

/// Frames kept per session.
pub const FRAME_CAPACITY: usize = 10_000;
/// Bytes collected before auto-detection gives up on the oldest data.
const DETECT_WINDOW: usize = 4096;
/// Smallest sample auto-detection looks at.
const DETECT_MIN: usize = 24;

/// Which decoder a session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecoderMode {
    /// No decoding.
    #[default]
    Off,
    /// Detect the protocol from the first data, then decode with it.
    Auto,
    /// A fixed protocol.
    Protocol(Protocol),
}

impl DecoderMode {
    /// `true` for [`DecoderMode::Off`].
    pub fn is_off(&self) -> bool {
        *self == Self::Off
    }

    /// All modes in menu order.
    pub fn all() -> impl Iterator<Item = Self> {
        [Self::Off, Self::Auto]
            .into_iter()
            .chain(Protocol::ALL.into_iter().map(Self::Protocol))
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Auto => "Auto-detect",
            Self::Protocol(p) => p.label(),
        }
    }
}

impl fmt::Display for DecoderMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::Protocol(p) => p.id(),
        })
    }
}

impl FromStr for DecoderMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "" => Ok(Self::Off),
            "auto" => Ok(Self::Auto),
            other => Protocol::parse(other).map(Self::Protocol).ok_or_else(|| {
                let ids: Vec<&str> = Protocol::ALL.iter().map(|p| p.id()).collect();
                format!("unknown decoder `{s}` (off, auto, {})", ids.join(", "))
            }),
        }
    }
}

impl Serialize for DecoderMode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DecoderMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Decoder settings stored in a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DecoderConfig {
    /// Decoder mode.
    pub mode: DecoderMode,
    /// Record modem line changes as frames.
    pub signals: bool,
}

impl DecoderConfig {
    /// `true` for the default (off, no signals).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Where a frame came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameSource {
    /// Received data.
    Rx,
    /// Transmitted data.
    Tx,
    /// Modem line change.
    Lines,
}

/// A frame in the session's frame ring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecodedFrame {
    /// Sequence number, unique and increasing within a session.
    pub seq: u64,
    /// When the frame completed.
    pub time: Timestamp,
    /// Direction.
    pub source: FrameSource,
    /// The TCP client whose data completed the frame (TCP server connections).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<std::net::SocketAddr>,
    /// The decoded frame.
    #[serde(flatten)]
    pub frame: Frame,
}

/// Frames as CSV: `Seq,Timestamp,Direction,Protocol,Check,Summary,Bytes_Hex`.
pub fn frames_to_csv(frames: &[DecodedFrame], tz: &jiff::tz::TimeZone) -> String {
    use std::fmt::Write as _;
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut out = String::from("Seq,Timestamp,Direction,Protocol,Check,Summary,Bytes_Hex\n");
    for f in frames {
        let check = match &f.frame.check {
            Check::None => String::new(),
            Check::Ok => "ok".into(),
            Check::Failed { received, computed } => {
                format!("failed (received {received}, computed {computed})")
            }
        };
        let dir = match (f.source, f.peer) {
            (FrameSource::Rx, Some(peer)) => format!("rx {peer}"),
            (FrameSource::Rx, None) => "rx".into(),
            (FrameSource::Tx, _) => "tx".into(),
            (FrameSource::Lines, _) => "lines".into(),
        };
        let hex = f
            .frame
            .bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            out,
            "{},{},{dir},{},{},{},{}",
            f.seq,
            f.time.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M:%S%.3f"),
            quote(f.frame.protocol.label()),
            quote(&check),
            quote(&f.frame.summary),
            quote(&hex)
        );
    }
    out
}

/// Result of [`FrameStore::since`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramesRead {
    /// Frames with `seq >= since`, oldest first.
    pub frames: Vec<DecodedFrame>,
    /// Pass this as `since` to continue.
    pub next_seq: u64,
    /// Frames between `since` and the oldest kept frame were dropped.
    pub dropped: u64,
}

/// Bounded ring of decoded frames.
#[derive(Debug)]
pub struct FrameStore {
    frames: VecDeque<DecodedFrame>,
    capacity: usize,
    next_seq: u64,
    check_failures: u64,
    /// TCP client of the data being decoded; stamped on the frames it completes.
    peer: Option<std::net::SocketAddr>,
}

impl Default for FrameStore {
    fn default() -> Self {
        Self::with_capacity(FRAME_CAPACITY)
    }
}

impl FrameStore {
    /// Store keeping at most `capacity` frames.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            capacity: capacity.max(1),
            next_seq: 0,
            check_failures: 0,
            peer: None,
        }
    }

    /// Append a frame.
    pub fn push(&mut self, source: FrameSource, time: Timestamp, frame: Frame) -> DecodedFrame {
        if matches!(frame.check, Check::Failed { .. }) {
            self.check_failures += 1;
        }
        let f = DecodedFrame {
            seq: self.next_seq,
            time,
            source,
            peer: self.peer,
            frame,
        };
        self.next_seq += 1;
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(f.clone());
        f
    }

    /// Up to `max` frames starting at `since`.
    pub fn since(&self, since: u64, max: usize) -> FramesRead {
        let first = self.frames.front().map_or(self.next_seq, |f| f.seq);
        let start = since.max(first);
        let skip = usize::try_from(start - first).unwrap_or(usize::MAX);
        let frames: Vec<DecodedFrame> = self.frames.iter().skip(skip).take(max).cloned().collect();
        FramesRead {
            next_seq: frames.last().map_or(start.min(self.next_seq), |f| f.seq + 1),
            dropped: first.saturating_sub(since),
            frames,
        }
    }

    /// The newest `n` frames.
    pub fn last(&self, n: usize) -> Vec<DecodedFrame> {
        self.frames
            .iter()
            .skip(self.frames.len().saturating_sub(n))
            .cloned()
            .collect()
    }

    /// Sequence number the next frame gets.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Frames kept.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// `true` if no frames are kept.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Frames with a failed checksum since the last clear.
    pub fn check_failures(&self) -> u64 {
        self.check_failures
    }

    /// Drop all frames (sequence numbers keep increasing).
    pub fn clear(&mut self) {
        self.frames.clear();
        self.check_failures = 0;
    }
}

/// Decoder state for status displays and MCP.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecoderStatus {
    /// Selected mode.
    pub mode: DecoderMode,
    /// Protocol in use (the detected one in auto mode).
    pub protocol: Option<Protocol>,
    /// Auto mode is still collecting data.
    pub detecting: bool,
    /// Modem line changes are recorded as frames (always for the RS232 decoder).
    pub signals: bool,
    /// Frames kept.
    pub frames: usize,
    /// Sequence number of the next frame.
    pub next_seq: u64,
    /// Frames with a failed checksum.
    pub check_failures: u64,
}

/// Per-session decoding state.
#[derive(Debug)]
pub struct Decoding {
    mode: DecoderMode,
    signals: bool,
    baud: Option<u32>,
    decoders: Option<(Box<dyn Decoder>, Box<dyn Decoder>)>,
    pending: Vec<(FrameSource, Vec<u8>, i64)>,
    pending_bytes: usize,
    lines: Option<ModemLines>,
    /// Decoded frames.
    pub store: FrameStore,
}

impl Default for Decoding {
    fn default() -> Self {
        Self {
            mode: DecoderMode::Off,
            signals: false,
            baud: None,
            decoders: None,
            pending: Vec::new(),
            pending_bytes: 0,
            lines: None,
            store: FrameStore::default(),
        }
    }
}

fn micros(t: Timestamp) -> i64 {
    t.as_microsecond()
}

impl Decoding {
    /// Current settings.
    pub fn config(&self) -> DecoderConfig {
        DecoderConfig {
            mode: self.mode,
            signals: self.signals,
        }
    }

    /// Status snapshot.
    pub fn status(&self) -> DecoderStatus {
        DecoderStatus {
            mode: self.mode,
            protocol: self.protocol(),
            detecting: self.mode == DecoderMode::Auto && self.decoders.is_none(),
            signals: self.signals,
            frames: self.store.len(),
            next_seq: self.store.next_seq(),
            check_failures: self.store.check_failures(),
        }
    }

    /// Protocol in use.
    pub fn protocol(&self) -> Option<Protocol> {
        self.decoders.as_ref().map(|(tx, _)| tx.protocol())
    }

    fn signals_enabled(&self) -> bool {
        self.signals || self.protocol() == Some(Protocol::Rs232)
    }

    /// Select a mode; partial frames and detection state are dropped.
    pub fn set_mode(&mut self, mode: DecoderMode) {
        self.mode = mode;
        self.restart();
    }

    /// Also record modem line changes as frames (always on for [`Protocol::Rs232`]).
    pub fn set_signals(&mut self, on: bool) {
        self.signals = on;
        self.lines = None;
    }

    /// Baud rate of the connection (sets the Modbus RTU frame gap); restarts the decoders.
    pub fn set_baud(&mut self, baud: Option<u32>) {
        self.baud = baud;
        self.restart();
    }

    /// Forget partial frames (port reopened); a detected protocol is detected again.
    pub fn restart(&mut self) {
        self.decoders = match self.mode {
            DecoderMode::Protocol(p) => Some(self.build(p)),
            DecoderMode::Off | DecoderMode::Auto => None,
        };
        self.pending.clear();
        self.pending_bytes = 0;
        self.lines = None;
    }

    fn build(&self, p: Protocol) -> (Box<dyn Decoder>, Box<dyn Decoder>) {
        (
            decode::decoder(p, Role::Request, self.baud),
            decode::decoder(p, Role::Response, self.baud),
        )
    }

    /// Decoded frames need periodic [`Decoding::poll`] calls (Modbus RTU gap detection).
    pub fn needs_poll(&self) -> bool {
        self.protocol() == Some(Protocol::ModbusRtu)
    }

    /// [`Decoding::feed`] for data from a known TCP client; frames it completes name the client.
    pub fn feed_from(
        &mut self,
        source: FrameSource,
        data: &[u8],
        ts: Timestamp,
        peer: Option<std::net::SocketAddr>,
    ) -> Vec<DecodedFrame> {
        self.store.peer = peer;
        let frames = self.feed(source, data, ts);
        self.store.peer = None;
        frames
    }

    /// Feed transmitted or received data; returns frames added to the store.
    pub fn feed(&mut self, source: FrameSource, data: &[u8], ts: Timestamp) -> Vec<DecodedFrame> {
        let mut out = Vec::new();
        match self.mode {
            DecoderMode::Off => {}
            DecoderMode::Protocol(_) => self.decode(source, data, micros(ts), ts, &mut out),
            DecoderMode::Auto if self.decoders.is_some() => {
                self.decode(source, data, micros(ts), ts, &mut out);
            }
            DecoderMode::Auto => {
                self.pending.push((source, data.to_vec(), micros(ts)));
                self.pending_bytes += data.len();
                if let Some(p) = self.detect() {
                    self.decoders = Some(self.build(p));
                    for (src, bytes, us) in std::mem::take(&mut self.pending) {
                        self.decode(src, &bytes, us, ts, &mut out);
                    }
                    self.pending_bytes = 0;
                } else {
                    while self.pending_bytes > DETECT_WINDOW && !self.pending.is_empty() {
                        let (_, old, _) = self.pending.remove(0);
                        self.pending_bytes -= old.len();
                    }
                }
            }
        }
        out
    }

    fn detect(&self) -> Option<Protocol> {
        // Prefer the device's answers; a session that only transmits is judged by its TX.
        let sample = |want: FrameSource| -> Vec<u8> {
            self.pending
                .iter()
                .filter(|(s, _, _)| *s == want)
                .flat_map(|(_, b, _)| b.iter().copied())
                .collect()
        };
        let rx = sample(FrameSource::Rx);
        let s = if rx.len() >= DETECT_MIN {
            rx
        } else {
            sample(FrameSource::Tx)
        };
        if s.len() < DETECT_MIN {
            return None;
        }
        let threshold = if s.len() < 128 { 0.9 } else { 0.6 };
        decode::detect_best(&s, threshold)
    }

    fn decode(
        &mut self,
        source: FrameSource,
        data: &[u8],
        us: i64,
        ts: Timestamp,
        out: &mut Vec<DecodedFrame>,
    ) {
        let Some((tx, rx)) = self.decoders.as_mut() else {
            return;
        };
        let frames = match source {
            FrameSource::Tx => tx.feed(data, us),
            FrameSource::Rx => rx.feed(data, us),
            FrameSource::Lines => return,
        };
        for f in frames {
            out.push(self.store.push(source, ts, f));
        }
    }

    /// Frames completed by silence.
    pub fn poll(&mut self, ts: Timestamp) -> Vec<DecodedFrame> {
        let Some((tx, rx)) = self.decoders.as_mut() else {
            return Vec::new();
        };
        let us = micros(ts);
        let (t, r) = (tx.poll(us), rx.poll(us));
        let mut out = Vec::new();
        for f in t {
            out.push(self.store.push(FrameSource::Tx, ts, f));
        }
        for f in r {
            out.push(self.store.push(FrameSource::Rx, ts, f));
        }
        out
    }

    /// Record modem lines; returns a frame when they changed and signals are enabled.
    pub fn modem(&mut self, m: ModemLines, ts: Timestamp) -> Option<DecodedFrame> {
        if !self.signals_enabled() {
            return None;
        }
        let lines = [
            ("DTR", m.dtr),
            ("RTS", m.rts),
            ("CTS", m.cts),
            ("DSR", m.dsr),
            ("DCD", m.dcd),
            ("RI", m.ri),
        ];
        let changed: Vec<&str> = match self.lines {
            Some(prev) if prev == m => return None,
            Some(prev) => {
                let old = [prev.dtr, prev.rts, prev.cts, prev.dsr, prev.dcd, prev.ri];
                lines
                    .iter()
                    .zip(old)
                    .filter(|((_, now), was)| now != was)
                    .map(|((n, _), _)| *n)
                    .collect()
            }
            None => Vec::new(),
        };
        self.lines = Some(m);
        Some(
            self.store
                .push(FrameSource::Lines, ts, decode::signals_frame(&lines, &changed)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_export_as_csv() {
        let f = DecodedFrame {
            seq: 7,
            time: Timestamp::from_millisecond(1_700_000_000_123).unwrap(),
            source: FrameSource::Tx,
            peer: None,
            frame: Frame {
                protocol: Protocol::ModbusRtu,
                bytes: vec![0x01, 0x03],
                summary: "Read \"holding\", 2".into(),
                fields: Vec::new(),
                check: Check::Ok,
            },
        };
        let csv = frames_to_csv(&[f], &jiff::tz::TimeZone::UTC);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next(),
            Some("Seq,Timestamp,Direction,Protocol,Check,Summary,Bytes_Hex")
        );
        assert_eq!(
            lines.next(),
            Some(r#"7,2023-11-14 22:13:20.123,tx,"Modbus RTU","ok","Read ""holding"", 2","01 03""#)
        );
    }
    use gtkterm_protocols::decode::modbus;

    fn ts(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(1_700_000_000_000 + ms).unwrap()
    }

    #[test]
    fn mode_strings() {
        for m in DecoderMode::all() {
            assert_eq!(m.to_string().parse::<DecoderMode>().unwrap(), m);
        }
        assert_eq!(
            "Modbus-RTU".parse::<DecoderMode>().unwrap(),
            DecoderMode::Protocol(Protocol::ModbusRtu)
        );
        assert!("bogus".parse::<DecoderMode>().unwrap_err().contains("modbus_rtu"));
        assert_eq!(serde_json::to_string(&DecoderMode::Auto).unwrap(), "\"auto\"");
    }

    #[test]
    fn store_ring_and_cursor() {
        let mut s = FrameStore::with_capacity(3);
        let mut d = decode::decoder(Protocol::Raw, Role::Response, None);
        for i in 0..5u8 {
            let f = d.feed(&[i], 0).remove(0);
            s.push(FrameSource::Rx, ts(0), f);
        }
        let r = s.since(0, 10);
        assert_eq!(r.frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!((r.next_seq, r.dropped), (5, 2));
        let r = s.since(4, 10);
        assert_eq!((r.frames.len(), r.next_seq, r.dropped), (1, 5, 0));
        let r = s.since(5, 10);
        assert_eq!((r.frames.len(), r.next_seq), (0, 5));
        let r = s.since(2, 1);
        assert_eq!((r.frames[0].seq, r.next_seq), (2, 3));
        assert_eq!(s.last(2).iter().map(|f| f.seq).collect::<Vec<_>>(), [3, 4]);
    }

    #[test]
    fn fixed_protocol_decodes_both_directions() {
        let mut d = Decoding::default();
        d.set_mode(DecoderMode::Protocol(Protocol::ModbusRtu));
        let req = modbus::rtu_frame(1, &[0x03, 0x00, 0x00, 0x00, 0x01]);
        let resp = modbus::rtu_frame(1, &[0x03, 0x02, 0x00, 0x2A]);
        let a = d.feed(FrameSource::Tx, &req, ts(0));
        let b = d.feed(FrameSource::Rx, &resp, ts(5));
        assert_eq!(a[0].source, FrameSource::Tx);
        assert!(
            a[0].frame.summary.contains("Addr:0x0000 Count:1"),
            "{}",
            a[0].frame.summary
        );
        assert!(b[0].frame.summary.contains("0x002A"), "{}", b[0].frame.summary);
        assert_eq!(d.status().frames, 2);
    }

    #[test]
    fn auto_detects_and_replays() {
        let mut d = Decoding::default();
        d.set_mode(DecoderMode::Auto);
        let line = b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n";
        assert!(d.feed(FrameSource::Rx, &line[..10], ts(0)).is_empty());
        assert!(d.status().detecting);
        let out = d.feed(FrameSource::Rx, &line[10..], ts(1));
        let out2 = d.feed(FrameSource::Rx, line, ts(2));
        assert_eq!(d.protocol(), Some(Protocol::Nmea));
        assert_eq!(
            out.len() + out2.len(),
            2,
            "buffered data is replayed into the decoder"
        );
        d.restart();
        assert!(d.status().detecting, "restart detects again");
    }

    #[test]
    fn auto_ignores_plain_text() {
        let mut d = Decoding::default();
        d.set_mode(DecoderMode::Auto);
        for _ in 0..200 {
            assert!(
                d.feed(FrameSource::Rx, b"just some log output here\n", ts(0))
                    .is_empty()
            );
        }
        assert!(d.status().detecting);
        assert!(d.pending_bytes <= DETECT_WINDOW + 32);
    }

    #[test]
    fn modem_frames() {
        let mut d = Decoding::default();
        let m = ModemLines {
            dtr: true,
            ..ModemLines::default()
        };
        assert!(d.modem(m, ts(0)).is_none(), "off unless enabled");
        d.set_mode(DecoderMode::Protocol(Protocol::Rs232));
        assert!(d.modem(m, ts(0)).unwrap().frame.summary.ends_with("(initial)"));
        assert!(d.modem(m, ts(1)).is_none());
        let f = d.modem(ModemLines { cts: true, ..m }, ts(2)).unwrap();
        assert_eq!(f.source, FrameSource::Lines);
        assert!(f.frame.summary.ends_with("Changed: CTS"), "{}", f.frame.summary);
        assert!(d.feed(FrameSource::Rx, b"data", ts(3)).is_empty());
    }
}
