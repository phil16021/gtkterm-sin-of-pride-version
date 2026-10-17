//! XMODEM (checksum, CRC, 1K), YMODEM batch and YMODEM-G as sans-IO state machines.
//!
//! The driver feeds received bytes with [`Transfer::input`], writes [`Output::bytes`] to the
//! line, calls [`Transfer::drained`] once those bytes have actually been transmitted (timeouts
//! start then, so a 1 KiB block at 300 baud does not time out while it is still being sent)
//! and calls [`Transfer::tick`] when [`Transfer::deadline`] has passed. Times are milliseconds
//! from an arbitrary origin.
//!
//! Compared with the C version (ROADMAP §8):
//! - X1: start requests (`C`) the receiver repeats while block 1 is on its way are ignored, and
//!   acknowledgements that arrive before a packet was completely sent are stale and ignored too, so
//!   a crossing `C`/NAK can neither lose block 1 nor make an extra ACK skip the next block.
//! - X2: packets are parsed by length, so EOT/CAN bytes inside the payload are just data.
//! - X3: only the padding of the final XMODEM block is stripped; YMODEM uses the size from the
//!   header and keeps every byte.
//! - X4: block numbers wrap modulo 256; a repeated block is acknowledged and dropped.

use serde::{Deserialize, Serialize};

use crate::checksum::{crc16_xmodem, sum8};

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const BS: u8 = 0x08;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1A;
const WANT_CRC: u8 = b'C';
const WANT_G: u8 = b'G';

/// How long a sender waits for the answer to an EOT.
const EOT_TIMEOUT_MS: u64 = 2_000;

/// Largest file a receiver accepts.
pub const MAX_FILE_SIZE: u64 = 256 << 20;

/// Protocol variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// 128-byte blocks; CRC if the receiver asks with `C`, else the 8-bit checksum.
    Xmodem,
    /// 1024-byte blocks (CRC).
    Xmodem1k,
    /// Batch transfer with a header block carrying name and size.
    Ymodem,
    /// YMODEM streaming without per-block acknowledgements (error-free links only). The
    /// receiver decides: YMODEM senders stream when asked with `G`, a YMODEM-G sender falls back
    /// to acknowledged YMODEM when asked with `C`.
    YmodemG,
}

impl Variant {
    /// All variants in menu order.
    pub const ALL: [Self; 4] = [Self::Xmodem, Self::Xmodem1k, Self::Ymodem, Self::YmodemG];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Xmodem => "XMODEM",
            Self::Xmodem1k => "XMODEM-1K",
            Self::Ymodem => "YMODEM",
            Self::YmodemG => "YMODEM-G",
        }
    }

    /// Identifier for configs and MCP.
    pub fn id(self) -> &'static str {
        match self {
            Self::Xmodem => "xmodem",
            Self::Xmodem1k => "xmodem_1k",
            Self::Ymodem => "ymodem",
            Self::YmodemG => "ymodem_g",
        }
    }

    /// Parse an identifier (`xmodem-1k`, `1k`, `ymodem-g`, ...).
    pub fn parse(s: &str) -> Option<Self> {
        Some(
            match s.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
                "xmodem" | "x" | "xmodem_crc" => Self::Xmodem,
                "xmodem_1k" | "1k" | "xmodem1k" => Self::Xmodem1k,
                "ymodem" | "y" | "ymodem_batch" => Self::Ymodem,
                "ymodem_g" | "g" | "ymodemg" => Self::YmodemG,
                _ => return None,
            },
        )
    }

    /// Several files per session, names and sizes in a header block.
    pub fn batch(self) -> bool {
        matches!(self, Self::Ymodem | Self::YmodemG)
    }

    fn big_blocks(self) -> bool {
        self != Self::Xmodem
    }
}

/// Timeouts and retry limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long to wait for the other side to start.
    pub start_timeout_ms: u64,
    /// How long to wait for an acknowledgement or the next block.
    pub block_timeout_ms: u64,
    /// Receiver: interval between start requests (`C`, `G` or NAK).
    pub start_interval_ms: u64,
    /// Errors per block before giving up.
    pub retries: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            start_timeout_ms: 60_000,
            block_timeout_ms: 10_000,
            start_interval_ms: 3_000,
            retries: 10,
        }
    }
}

/// A file to send or a received file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileData {
    /// File name (YMODEM header; for XMODEM the name chosen by the driver).
    pub name: String,
    /// Contents.
    pub data: Vec<u8>,
    /// Modification time, seconds since the Unix epoch (YMODEM header).
    pub mtime: Option<u64>,
}

/// Something the driver should know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A file transfer began (`size` is known for senders and YMODEM receivers).
    FileStarted {
        /// Index within the batch.
        index: usize,
        /// Name.
        name: String,
        /// Size in bytes, if known.
        size: Option<u64>,
    },
    /// Bytes transferred of the current file.
    Progress {
        /// Index within the batch.
        index: usize,
        /// Bytes so far.
        bytes: u64,
        /// Size, if known.
        size: Option<u64>,
    },
    /// A block was repeated or a timeout occurred.
    Retry(String),
    /// Receiver: a complete file.
    FileReceived(FileData),
    /// Sender: a file was acknowledged completely.
    FileSent {
        /// Index within the batch.
        index: usize,
    },
    /// The whole session completed.
    Finished,
    /// The transfer stopped (timeout, cancel, protocol error).
    Failed(String),
}

/// Bytes to write and events, returned by every call.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Output {
    /// Bytes to transmit.
    pub bytes: Vec<u8>,
    /// Events in order.
    pub events: Vec<Event>,
}

impl Output {
    fn send(&mut self, b: &[u8]) {
        self.bytes.extend_from_slice(b);
    }

    fn event(&mut self, e: Event) {
        self.events.push(e);
    }
}

/// A running transfer.
pub trait Transfer: Send + Sync + std::fmt::Debug {
    /// Begin (receivers send their start request).
    fn start(&mut self, now_ms: u64) -> Output;
    /// Received bytes.
    fn input(&mut self, data: &[u8], now_ms: u64) -> Output;
    /// The bytes of earlier outputs have been transmitted.
    fn drained(&mut self, now_ms: u64) -> Output;
    /// Call when `deadline()` has passed.
    fn tick(&mut self, now_ms: u64) -> Output;
    /// When `tick` must be called next.
    fn deadline(&self) -> Option<u64>;
    /// Abort: sends the cancel sequence and reports `Failed("cancelled")`.
    fn cancel(&mut self) -> Output;
    /// Finished or failed.
    fn is_done(&self) -> bool;
    /// Every byte has been acknowledged and only the closing handshake is left: if the line goes
    /// away now, the transfer is complete. lrzsz flushes its output when it exits, so its final
    /// ACK (as receiver) or end-of-batch header (as YMODEM-G sender) can be lost.
    fn complete_if_closed(&self) -> bool {
        false
    }
}

/// The cancel sequence: CANs followed by backspaces that erase them from a terminal.
pub fn cancel_sequence() -> Vec<u8> {
    let mut v = vec![CAN; 8];
    v.extend_from_slice(&[BS; 8]);
    v
}

fn packet(block: u8, payload: &[u8], crc: bool) -> Vec<u8> {
    let mut p = Vec::with_capacity(payload.len() + 5);
    p.push(if payload.len() == 1024 { STX } else { SOH });
    p.push(block);
    p.push(!block);
    p.extend_from_slice(payload);
    if crc {
        p.extend_from_slice(&crc16_xmodem(payload).to_be_bytes());
    } else {
        p.push(sum8(payload));
    }
    p
}

/// YMODEM block 0 payload: `name NUL size mtime(octal) mode(octal) NUL`, padded with NULs.
fn header_payload(file: Option<&FileData>) -> Vec<u8> {
    let mut h = Vec::new();
    if let Some(f) = file {
        h.extend_from_slice(f.name.as_bytes());
        h.push(0);
        h.extend_from_slice(f.data.len().to_string().as_bytes());
        if let Some(m) = f.mtime {
            h.extend_from_slice(format!(" {m:o} 100644").as_bytes());
        }
        h.push(0);
    }
    let size = if h.len() <= 128 { 128 } else { 1024 };
    h.resize(size, 0);
    h
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SState {
    /// Waiting for `C`/NAK/`G` before a header (batch) or the first block.
    WaitStart,
    WaitHeaderAck,
    /// Header acknowledged, waiting for `C`/`G` to start the data.
    WaitDataStart,
    WaitAck,
    /// YMODEM-G: sending blocks without acknowledgements.
    Streaming,
    WaitEotAck,
    WaitFinalAck,
    Done,
}

/// Sends one file (XMODEM) or a batch (YMODEM).
#[derive(Debug)]
pub struct Sender {
    variant: Variant,
    timing: Timing,
    files: Vec<FileData>,
    index: usize,
    offset: usize,
    block: u8,
    crc: bool,
    state: SState,
    retries: u32,
    deadline: Option<u64>,
    last: Vec<u8>,
    last_len: usize,
    cans: u8,
    /// The last packet has not been completely transmitted yet (`drained` not called).
    in_flight: bool,
    /// YMODEM-G mode, chosen by the receiver's start request.
    streaming: bool,
}

impl Sender {
    /// Sender for `files`; XMODEM variants take exactly one file.
    pub fn new(variant: Variant, files: Vec<FileData>, timing: Timing) -> Result<Self, String> {
        if !variant.batch() && files.len() != 1 {
            return Err(format!("{} sends exactly one file", variant.label()));
        }
        if files.is_empty() {
            return Err("no files to send".into());
        }
        Ok(Self {
            variant,
            timing,
            files,
            index: 0,
            offset: 0,
            block: 1,
            crc: true,
            state: SState::WaitStart,
            retries: 0,
            deadline: None,
            last: Vec::new(),
            last_len: 0,
            cans: 0,
            in_flight: false,
            streaming: false,
        })
    }

    fn file(&self) -> &FileData {
        &self.files[self.index]
    }

    fn fail(&mut self, out: &mut Output, reason: &str, cancel: bool) {
        if cancel {
            out.send(&cancel_sequence());
        }
        out.event(Event::Failed(reason.to_owned()));
        self.state = SState::Done;
        self.deadline = None;
    }

    fn emit(&mut self, out: &mut Output, p: Vec<u8>, payload_len: usize) {
        out.send(&p);
        self.last = p;
        self.last_len = payload_len;
        // The deadline starts when the bytes are on the wire (`drained`).
        self.deadline = None;
        self.in_flight = true;
    }

    fn next_block(&mut self, out: &mut Output) {
        let data = &self.files[self.index].data;
        let remaining = data.len() - self.offset;
        let size = if self.variant.big_blocks() && self.crc && remaining > 128 {
            1024
        } else {
            128
        };
        let take = remaining.min(size);
        let mut payload = data[self.offset..self.offset + take].to_vec();
        payload.resize(size, SUB);
        let p = packet(self.block, &payload, self.crc);
        self.emit(out, p, take);
    }

    fn send_eot(&mut self, out: &mut Output) {
        self.state = SState::WaitEotAck;
        self.emit(out, vec![EOT], 0);
    }

    fn begin_data(&mut self, out: &mut Output) {
        self.offset = 0;
        self.block = 1;
        self.retries = 0;
        out.event(Event::FileStarted {
            index: self.index,
            name: self.file().name.clone(),
            size: Some(self.file().data.len() as u64),
        });
        if self.file().data.is_empty() {
            self.send_eot(out);
        } else if self.streaming {
            self.state = SState::Streaming;
            self.stream(out);
        } else {
            self.state = SState::WaitAck;
            self.next_block(out);
        }
    }

    /// YMODEM-G: queue up to 8 KiB, the rest after `drained`.
    fn stream(&mut self, out: &mut Output) {
        let mut queued = 0;
        while queued < 8192 && self.offset < self.file().data.len() {
            self.next_block(out);
            queued += self.last_len;
            self.offset += self.last_len;
            self.block = self.block.wrapping_add(1);
        }
        out.event(Event::Progress {
            index: self.index,
            bytes: self.offset as u64,
            size: Some(self.file().data.len() as u64),
        });
        if self.offset >= self.file().data.len() {
            self.send_eot(out);
        }
    }

    fn retry(&mut self, out: &mut Output, reason: &str) {
        if self.state == SState::WaitEotAck && self.complete_if_closed() && self.retries >= 1 {
            // All data was acknowledged; the receiver probably exited before its last ACK left.
            out.event(Event::FileSent { index: self.index });
            out.event(Event::Finished);
            self.state = SState::Done;
            self.deadline = None;
            return;
        }
        self.retries += 1;
        if self.retries > self.timing.retries {
            self.fail(out, &format!("too many errors ({reason})"), true);
            return;
        }
        out.event(Event::Retry(reason.to_owned()));
        let p = self.last.clone();
        out.send(&p);
        self.deadline = None;
        self.in_flight = true;
    }

    fn start_char(&self, b: u8) -> bool {
        match b {
            WANT_CRC => true,
            WANT_G => self.variant.batch(),
            NAK => !self.variant.batch(),
            _ => false,
        }
    }

    fn on_byte(&mut self, b: u8, out: &mut Output) {
        if b == CAN {
            self.cans += 1;
            if self.cans >= 2 {
                self.fail(out, "cancelled by the receiver", false);
            }
            return;
        }
        self.cans = 0;
        if self.in_flight && matches!(b, ACK | NAK | WANT_CRC | WANT_G) {
            // A reply to an earlier packet that crossed the one being sent.
            return;
        }
        match self.state {
            SState::WaitStart if self.start_char(b) => {
                self.crc = b != NAK;
                self.streaming = b == WANT_G;
                if !self.variant.batch() {
                    self.begin_data(out);
                } else if self.index < self.files.len() {
                    self.state = SState::WaitHeaderAck;
                    self.retries = 0;
                    let payload = header_payload(Some(self.file()));
                    let p = packet(0, &payload, true);
                    self.emit(out, p, 0);
                } else {
                    self.state = SState::WaitFinalAck;
                    let p = packet(0, &header_payload(None), true);
                    self.emit(out, p, 0);
                }
            }
            SState::WaitHeaderAck => match b {
                ACK => {
                    self.state = SState::WaitDataStart;
                    self.deadline = None;
                }
                // YMODEM-G receivers may skip the ACK and ask for data right away.
                WANT_G if self.streaming => self.begin_data(out),
                NAK | WANT_CRC => self.retry(out, "header repeated"),
                _ => {}
            },
            SState::WaitDataStart if self.start_char(b) => {
                self.streaming = b == WANT_G;
                self.begin_data(out);
            }
            SState::WaitAck => match b {
                ACK => {
                    self.offset += self.last_len;
                    self.block = self.block.wrapping_add(1);
                    self.retries = 0;
                    out.event(Event::Progress {
                        index: self.index,
                        bytes: self.offset as u64,
                        size: Some(self.file().data.len() as u64),
                    });
                    if self.offset >= self.file().data.len() {
                        self.send_eot(out);
                    } else {
                        self.next_block(out);
                    }
                }
                NAK => self.retry(out, "block rejected"),
                // A `C` here is a start request that crossed block 1 (X1); the block is on its way.
                _ => {}
            },
            SState::WaitEotAck => match b {
                ACK => {
                    out.event(Event::FileSent { index: self.index });
                    self.index += 1;
                    self.retries = 0;
                    if self.variant.batch() {
                        self.state = SState::WaitStart;
                        self.deadline = None;
                    } else {
                        out.event(Event::Finished);
                        self.state = SState::Done;
                        self.deadline = None;
                    }
                }
                // YMODEM receivers NAK the first EOT to make sure it was not noise.
                NAK => {
                    out.send(&[EOT]);
                    self.deadline = None;
                    self.in_flight = true;
                }
                _ => {}
            },
            SState::WaitFinalAck if b == ACK => {
                out.event(Event::Finished);
                self.state = SState::Done;
                self.deadline = None;
            }
            _ => {}
        }
    }
}

impl Transfer for Sender {
    fn start(&mut self, now_ms: u64) -> Output {
        self.deadline = Some(now_ms + self.timing.start_timeout_ms);
        Output::default()
    }

    fn input(&mut self, data: &[u8], _now_ms: u64) -> Output {
        let mut out = Output::default();
        for &b in data {
            if self.state == SState::Done {
                break;
            }
            self.on_byte(b, &mut out);
        }
        out
    }

    fn drained(&mut self, now_ms: u64) -> Output {
        let mut out = Output::default();
        self.in_flight = false;
        match self.state {
            SState::Done => {}
            SState::Streaming => self.stream(&mut out),
            SState::WaitStart | SState::WaitDataStart => {
                self.deadline.get_or_insert(now_ms + self.timing.start_timeout_ms);
            }
            // EOT and the end-of-batch header are answered at once; no block timeout for them.
            SState::WaitEotAck | SState::WaitFinalAck => {
                self.deadline = Some(now_ms + self.timing.block_timeout_ms.min(EOT_TIMEOUT_MS));
            }
            _ => self.deadline = Some(now_ms + self.timing.block_timeout_ms),
        }
        out
    }

    fn tick(&mut self, now_ms: u64) -> Output {
        let mut out = Output::default();
        if self.deadline.is_none_or(|d| now_ms < d) {
            return out;
        }
        match self.state {
            SState::WaitStart | SState::WaitDataStart => {
                self.fail(&mut out, "the receiver did not start", true);
            }
            SState::WaitHeaderAck | SState::WaitAck | SState::WaitEotAck => {
                self.retry(&mut out, "timeout");
            }
            // Some receivers exit after the end-of-batch header without acknowledging it.
            SState::WaitFinalAck => {
                out.event(Event::Finished);
                self.state = SState::Done;
                self.deadline = None;
            }
            SState::Streaming => self.fail(&mut out, "the line stalled", true),
            SState::Done => {}
        }
        out
    }

    fn deadline(&self) -> Option<u64> {
        self.deadline
    }

    fn cancel(&mut self) -> Output {
        let mut out = Output::default();
        if self.state != SState::Done {
            self.fail(&mut out, "cancelled", true);
        }
        out
    }

    fn is_done(&self) -> bool {
        self.state == SState::Done
    }

    fn complete_if_closed(&self) -> bool {
        let last = self.index + 1 >= self.files.len();
        match self.state {
            SState::WaitEotAck => last,
            SState::WaitStart => self.index >= self.files.len(),
            SState::WaitFinalAck => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RState {
    /// Sending start requests until block 1 (XMODEM) or block 0 (YMODEM) arrives.
    Starting,
    Data,
    /// YMODEM: first EOT was NAKed, waiting for the second.
    SecondEot,
    Done,
}

#[derive(Debug)]
struct Incoming {
    name: String,
    size: Option<u64>,
    mtime: Option<u64>,
    data: Vec<u8>,
    /// Payload bytes of the last block (XMODEM padding is stripped from it only).
    last_block: usize,
}

/// Receives one file (XMODEM) or a batch (YMODEM).
#[derive(Debug)]
pub struct Receiver {
    variant: Variant,
    timing: Timing,
    default_name: String,
    crc: bool,
    state: RState,
    /// In a YMODEM batch: waiting for a header (block 0) rather than data.
    want_header: bool,
    buf: Vec<u8>,
    expected: u8,
    file: Option<Incoming>,
    received: usize,
    retries: u32,
    start_attempts: u32,
    deadline: Option<u64>,
    start_deadline: u64,
}

fn parse_header(payload: &[u8]) -> Option<(String, Option<u64>, Option<u64>)> {
    let nul = payload.iter().position(|&b| b == 0)?;
    let name = String::from_utf8_lossy(&payload[..nul]).into_owned();
    if name.is_empty() {
        return None;
    }
    let rest = &payload[nul + 1..];
    let info_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    let info = String::from_utf8_lossy(&rest[..info_end]);
    let mut fields = info.split_ascii_whitespace();
    let size = fields.next().and_then(|s| s.parse().ok());
    let mtime = fields.next().and_then(|s| u64::from_str_radix(s, 8).ok());
    Some((name, size, mtime))
}

impl Receiver {
    /// Receiver; `default_name` names an XMODEM file (which carries no name).
    pub fn new(variant: Variant, timing: Timing, default_name: impl Into<String>) -> Self {
        Self {
            variant,
            timing,
            default_name: default_name.into(),
            crc: true,
            state: RState::Starting,
            want_header: variant.batch(),
            buf: Vec::new(),
            expected: 1,
            file: None,
            received: 0,
            retries: 0,
            start_attempts: 0,
            deadline: None,
            start_deadline: 0,
        }
    }

    /// Longer than the sender's timeout, so its retransmission arrives before our NAK and the two
    /// never cross (a crossing NAK + duplicate ACK could otherwise skip a block).
    fn data_timeout(&self) -> u64 {
        self.timing.block_timeout_ms + self.timing.block_timeout_ms / 2
    }

    fn start_request(&self) -> u8 {
        match (self.variant, self.crc) {
            (Variant::YmodemG, _) => WANT_G,
            (_, true) => WANT_CRC,
            (_, false) => NAK,
        }
    }

    fn fail(&mut self, out: &mut Output, reason: &str, cancel: bool) {
        if cancel {
            out.send(&cancel_sequence());
        }
        out.event(Event::Failed(reason.to_owned()));
        self.state = RState::Done;
        self.deadline = None;
        self.buf.clear();
    }

    fn finish_file(&mut self, out: &mut Output) {
        let Some(mut f) = self.file.take() else {
            return;
        };
        if let Some(size) = f.size {
            f.data.truncate(usize::try_from(size).unwrap_or(usize::MAX));
        } else {
            // X3: strip the SUB padding of the final block only.
            let start = f.data.len() - f.last_block;
            let keep = f.data[start..]
                .iter()
                .rposition(|&b| b != SUB)
                .map_or(0, |p| p + 1);
            f.data.truncate(start + keep);
        }
        out.event(Event::Progress {
            index: self.received,
            bytes: f.data.len() as u64,
            size: f.size,
        });
        out.event(Event::FileReceived(FileData {
            name: f.name,
            data: f.data,
            mtime: f.mtime,
        }));
        self.received += 1;
    }

    fn new_file(&mut self, out: &mut Output, name: String, size: Option<u64>, mtime: Option<u64>) {
        out.event(Event::FileStarted {
            index: self.received,
            name: name.clone(),
            size,
        });
        self.file = Some(Incoming {
            name,
            size,
            mtime,
            data: Vec::new(),
            last_block: 0,
        });
    }

    fn nak(&mut self, out: &mut Output, reason: &str) {
        if self.variant == Variant::YmodemG {
            self.fail(out, &format!("{reason} (YMODEM-G cannot repeat blocks)"), true);
            return;
        }
        self.retries += 1;
        if self.retries > self.timing.retries {
            self.fail(out, &format!("too many errors ({reason})"), true);
            return;
        }
        out.event(Event::Retry(reason.to_owned()));
        out.send(&[NAK]);
    }

    fn after_eot(&mut self, out: &mut Output, now_ms: u64) {
        self.finish_file(out);
        if self.variant.batch() {
            self.state = RState::Starting;
            self.want_header = true;
            self.crc = true;
            self.start_attempts = 0;
            self.start_deadline = now_ms + self.timing.start_timeout_ms;
            out.send(&[self.start_request()]);
        } else {
            out.event(Event::Finished);
            self.state = RState::Done;
        }
    }

    fn on_packet(&mut self, block: u8, payload: &[u8], out: &mut Output, now_ms: u64) {
        if self.want_header {
            if block != 0 {
                // Data while we expected a header: a sender that skipped block 0.
                self.nak(out, "expected the YMODEM header");
                return;
            }
            match parse_header(payload) {
                None => {
                    out.send(&[ACK]);
                    out.event(Event::Finished);
                    self.state = RState::Done;
                }
                Some((name, size, mtime)) => {
                    if size.is_some_and(|s| s > MAX_FILE_SIZE) {
                        self.fail(out, &format!("{name} is larger than {MAX_FILE_SIZE} bytes"), true);
                        return;
                    }
                    out.send(&[ACK]);
                    self.new_file(out, name, size, mtime);
                    self.want_header = false;
                    self.expected = 1;
                    self.state = RState::Data;
                    out.send(&[self.start_request()]);
                }
            }
            return;
        }
        if self.variant.batch() && block == 0 && self.expected == 1 {
            // Our ACK of the header was lost; acknowledge again and ask for data.
            out.send(&[ACK, self.start_request()]);
            return;
        }
        if block == self.expected {
            if self.file.is_none() {
                let name = self.default_name.clone();
                self.new_file(out, name, None, None);
            }
            let index = self.received;
            let f = self.file.as_mut().expect("file was just created");
            if (f.data.len() + payload.len()) as u64 > MAX_FILE_SIZE + 1024 {
                self.fail(out, &format!("file is larger than {MAX_FILE_SIZE} bytes"), true);
                return;
            }
            f.data.extend_from_slice(payload);
            f.last_block = payload.len();
            let (bytes, size) = (
                f.size.map_or(f.data.len() as u64, |s| s.min(f.data.len() as u64)),
                f.size,
            );
            self.expected = self.expected.wrapping_add(1);
            self.retries = 0;
            self.state = RState::Data;
            if self.variant != Variant::YmodemG {
                out.send(&[ACK]);
            }
            out.event(Event::Progress { index, bytes, size });
            let _ = now_ms;
        } else if block == self.expected.wrapping_sub(1) {
            // X4: a repeat of the previous block (our ACK was lost), also across the 255→0 wrap.
            out.send(&[ACK]);
            out.event(Event::Retry(format!("block {block} repeated")));
        } else {
            self.fail(
                out,
                &format!("block sequence error: got {block}, expected {}", self.expected),
                true,
            );
        }
    }

    fn process(&mut self, out: &mut Output, now_ms: u64) {
        while let Some(&first) = self.buf.first() {
            if self.state == RState::Done {
                return;
            }
            match first {
                SOH | STX => {
                    let size = if first == STX { 1024 } else { 128 };
                    let len = 3 + size + if self.crc { 2 } else { 1 };
                    if self.buf.len() < len {
                        return;
                    }
                    let pkt: Vec<u8> = self.buf.drain(..len).collect();
                    let (block, cmpl, payload) = (pkt[1], pkt[2], &pkt[3..3 + size]);
                    let ok = block == !cmpl
                        && if self.crc {
                            crc16_xmodem(payload).to_be_bytes() == pkt[3 + size..]
                        } else {
                            sum8(payload) == pkt[3 + size]
                        };
                    if ok {
                        let payload = payload.to_vec();
                        self.on_packet(block, &payload, out, now_ms);
                    } else {
                        // Purge whatever else arrived with the damaged block, then ask again.
                        self.buf.clear();
                        self.nak(out, "damaged block");
                    }
                }
                EOT => {
                    self.buf.remove(0);
                    let active = self.file.is_some() || !self.want_header;
                    if !active {
                        continue;
                    }
                    if self.variant.batch() && self.state != RState::SecondEot {
                        self.state = RState::SecondEot;
                        out.send(&[NAK]);
                    } else {
                        if self.file.is_none() {
                            // XMODEM with an empty file: EOT right after the start request.
                            let name = self.default_name.clone();
                            self.new_file(out, name, None, None);
                        }
                        out.send(&[ACK]);
                        self.after_eot(out, now_ms);
                    }
                }
                CAN => {
                    if self.buf.len() < 2 {
                        return;
                    }
                    if self.buf[1] == CAN {
                        self.fail(out, "cancelled by the sender", false);
                        return;
                    }
                    self.buf.remove(0);
                }
                _ => {
                    self.buf.remove(0);
                }
            }
        }
    }
}

impl Transfer for Receiver {
    fn start(&mut self, now_ms: u64) -> Output {
        let mut out = Output::default();
        self.start_deadline = now_ms + self.timing.start_timeout_ms;
        out.send(&[self.start_request()]);
        out
    }

    fn input(&mut self, data: &[u8], now_ms: u64) -> Output {
        let mut out = Output::default();
        if self.state == RState::Done {
            return out;
        }
        self.buf.extend_from_slice(data);
        self.process(&mut out, now_ms);
        if self.state != RState::Done {
            // Data arrived: the line is alive.
            self.deadline = Some(
                now_ms
                    + if self.state == RState::Starting {
                        self.timing.start_interval_ms
                    } else {
                        self.data_timeout()
                    },
            );
        }
        out
    }

    fn drained(&mut self, now_ms: u64) -> Output {
        if self.state != RState::Done {
            let wait = if self.state == RState::Starting {
                self.timing.start_interval_ms
            } else {
                self.data_timeout()
            };
            self.deadline = Some(now_ms + wait);
        }
        Output::default()
    }

    fn tick(&mut self, now_ms: u64) -> Output {
        let mut out = Output::default();
        if self.deadline.is_none_or(|d| now_ms < d) || self.state == RState::Done {
            return out;
        }
        self.buf.clear();
        match self.state {
            RState::Starting => {
                if self.complete_if_closed() && self.start_attempts >= 3 {
                    // lrzsz `sb -g` exits without waiting for the end-of-batch header's ACK.
                    out.event(Event::Finished);
                    self.state = RState::Done;
                    self.deadline = None;
                    return out;
                }
                if now_ms >= self.start_deadline {
                    self.fail(&mut out, "the sender did not start", true);
                    return out;
                }
                self.start_attempts += 1;
                // Plain XMODEM senders that ignore `C`: fall back to the checksum after 3 tries.
                if self.variant == Variant::Xmodem && self.start_attempts == 3 && self.file.is_none() {
                    self.crc = false;
                }
                out.send(&[self.start_request()]);
            }
            RState::Data | RState::SecondEot => self.nak(&mut out, "timeout"),
            RState::Done => {}
        }
        self.deadline = None;
        out
    }

    fn deadline(&self) -> Option<u64> {
        self.deadline
    }

    fn cancel(&mut self) -> Output {
        let mut out = Output::default();
        if self.state != RState::Done {
            self.fail(&mut out, "cancelled", true);
        }
        out
    }

    fn is_done(&self) -> bool {
        self.state == RState::Done
    }

    fn complete_if_closed(&self) -> bool {
        self.variant.batch() && self.state == RState::Starting && self.want_header && self.received > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Connects a sender and a receiver in memory with a virtual clock and optional faults.
    struct Link {
        now: u64,
        to_rx: Vec<u8>,
        to_tx: Vec<u8>,
        events_tx: Vec<Event>,
        events_rx: Vec<Event>,
    }

    type Fault = Box<dyn FnMut(&mut Vec<u8>, bool)>;

    fn run(tx: &mut dyn Transfer, rx: &mut dyn Transfer, mut fault: Fault) -> Link {
        let mut l = Link {
            now: 0,
            to_rx: Vec::new(),
            to_tx: Vec::new(),
            events_tx: Vec::new(),
            events_rx: Vec::new(),
        };
        let absorb = |o: Output, wire: &mut Vec<u8>, ev: &mut Vec<Event>| {
            wire.extend(o.bytes);
            ev.extend(o.events);
        };
        absorb(tx.start(0), &mut l.to_rx, &mut l.events_tx);
        absorb(rx.start(0), &mut l.to_tx, &mut l.events_rx);
        for _ in 0..200_000 {
            if tx.is_done() && rx.is_done() {
                break;
            }
            let mut progressed = false;
            if !l.to_rx.is_empty() {
                let mut bytes = std::mem::take(&mut l.to_rx);
                fault(&mut bytes, true);
                let o = tx.drained(l.now);
                absorb(o, &mut l.to_rx, &mut l.events_tx);
                let o = rx.input(&bytes, l.now);
                absorb(o, &mut l.to_tx, &mut l.events_rx);
                progressed = true;
            }
            if !l.to_tx.is_empty() {
                let mut bytes = std::mem::take(&mut l.to_tx);
                fault(&mut bytes, false);
                let o = rx.drained(l.now);
                absorb(o, &mut l.to_tx, &mut l.events_rx);
                let o = tx.input(&bytes, l.now);
                absorb(o, &mut l.to_rx, &mut l.events_tx);
                progressed = true;
            }
            if !progressed {
                let next = [tx.deadline(), rx.deadline()].into_iter().flatten().min();
                let Some(next) = next else { break };
                l.now = l.now.max(next);
                let o = tx.tick(l.now);
                absorb(o, &mut l.to_rx, &mut l.events_tx);
                let o = rx.tick(l.now);
                absorb(o, &mut l.to_tx, &mut l.events_rx);
            }
        }
        l
    }

    /// Events without progress and file contents, for assertion messages.
    fn brief(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Progress { .. } => None,
                Event::FileReceived(f) => Some(format!("received {} ({} bytes)", f.name, f.data.len())),
                e => Some(format!("{e:?}")),
            })
            .collect()
    }

    fn file(name: &str, data: Vec<u8>) -> FileData {
        FileData {
            name: name.into(),
            data,
            mtime: Some(1_700_000_000),
        }
    }

    fn received(events: &[Event]) -> Vec<FileData> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::FileReceived(f) => Some(f.clone()),
                _ => None,
            })
            .collect()
    }

    fn pattern(n: usize, seed: u8) -> Vec<u8> {
        (0..n)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn roundtrip(variant: Variant, files: Vec<FileData>, fault: Fault) -> (Link, Vec<FileData>) {
        let mut tx = Sender::new(variant, files, Timing::default()).unwrap();
        let mut rx = Receiver::new(variant, Timing::default(), "received.bin");
        let l = run(&mut tx, &mut rx, fault);
        assert!(
            l.events_tx.contains(&Event::Finished),
            "sender: {:?}\nreceiver: {:?}",
            brief(&l.events_tx),
            brief(&l.events_rx)
        );
        assert!(
            l.events_rx.contains(&Event::Finished),
            "receiver: {:?}",
            l.events_rx.last()
        );
        let files = received(&l.events_rx);
        (l, files)
    }

    fn none() -> Fault {
        Box::new(|_, _| {})
    }

    #[test]
    fn crc_reference_and_packet_layout() {
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
        let p = packet(1, &[0x41; 128], true);
        assert_eq!((p[0], p[1], p[2], p.len()), (SOH, 1, 0xFE, 133));
        assert_eq!(packet(255, &[0; 1024], false)[..3], [STX, 255, 0]);
    }

    #[test]
    fn xmodem_sizes_and_special_bytes() {
        // 0 bytes, block boundaries, more than 255 blocks (X4 wrap), payloads of EOT/CAN (X2).
        for variant in [Variant::Xmodem, Variant::Xmodem1k] {
            for data in [
                vec![],
                vec![7],
                pattern(127, 1),
                pattern(128, 2),
                pattern(129, 3),
                pattern(1024, 4),
                pattern(1025, 5),
                pattern(33 * 1024 + 17, 6),
                vec![EOT; 300],
                vec![CAN; 300],
            ] {
                let (_, got) = roundtrip(variant, vec![file("f", data.clone())], none());
                assert_eq!(got.len(), 1);
                assert_eq!(got[0].data, data, "{variant:?} {} bytes", data.len());
                assert_eq!(got[0].name, "received.bin");
            }
        }
    }

    #[test]
    fn xmodem_keeps_sub_bytes_inside_the_file() {
        // X3: only the final block's padding is stripped.
        let mut data = vec![SUB; 300];
        data.push(b'x');
        let (_, got) = roundtrip(Variant::Xmodem, vec![file("f", data.clone())], none());
        assert_eq!(got[0].data, data);
    }

    #[test]
    fn ymodem_batch_keeps_names_sizes_and_trailing_sub() {
        let files = vec![
            file("a.bin", vec![SUB; 1000]),
            file("empty", vec![]),
            file("b.txt", pattern(5000, 9)),
        ];
        for variant in [Variant::Ymodem, Variant::YmodemG] {
            let (_, got) = roundtrip(variant, files.clone(), none());
            assert_eq!(got, files, "{variant:?}");
        }
    }

    #[test]
    fn the_receiver_chooses_streaming() {
        let files = vec![file("a", pattern(3000, 1))];
        for (txv, rxv) in [
            (Variant::YmodemG, Variant::Ymodem),
            (Variant::Ymodem, Variant::YmodemG),
        ] {
            let mut tx = Sender::new(txv, files.clone(), Timing::default()).unwrap();
            let mut rx = Receiver::new(rxv, Timing::default(), "f");
            let l = run(&mut tx, &mut rx, none());
            assert_eq!(received(&l.events_rx), files, "{txv:?} -> {rxv:?}");
            assert_eq!(tx.streaming, rxv == Variant::YmodemG);
        }
    }

    #[test]
    fn checksum_fallback_for_old_senders() {
        // A receiver that never asks with `C` gets checksum blocks.
        let data = pattern(300, 1);
        let mut tx = Sender::new(Variant::Xmodem, vec![file("f", data.clone())], Timing::default()).unwrap();
        let mut rx = Receiver::new(Variant::Xmodem, Timing::default(), "f");
        rx.crc = false;
        let l = run(&mut tx, &mut rx, none());
        assert_eq!(received(&l.events_rx)[0].data, data);
        assert!(!tx.crc);
    }

    #[test]
    fn corruption_and_loss_are_retried() {
        let data = pattern(5000, 3);
        for variant in [Variant::Xmodem, Variant::Xmodem1k, Variant::Ymodem] {
            let mut n = 0u32;
            let fault: Fault = Box::new(move |bytes, to_rx| {
                n += 1;
                if to_rx && n.is_multiple_of(7) && bytes.len() > 10 {
                    bytes[5] ^= 0x55; // damaged payload
                }
                if !to_rx && n.is_multiple_of(11) {
                    bytes.clear(); // lost ACK
                }
            });
            let (l, got) = roundtrip(variant, vec![file("f", data.clone())], fault);
            assert_eq!(got[0].data, data, "{variant:?}");
            assert!(
                l.events_rx.iter().any(|e| matches!(e, Event::Retry(_))),
                "{variant:?}"
            );
        }
    }

    #[test]
    fn first_block_survives_a_repeated_start_request() {
        // X1: the receiver repeats `C` while block 1 is on its way; nothing is lost or skipped.
        let data = pattern(200, 2);
        assert_ne!(data.last(), Some(&SUB), "XMODEM cannot keep a trailing SUB");
        let mut tx = Sender::new(Variant::Xmodem, vec![file("f", data.clone())], Timing::default()).unwrap();
        tx.start(0);
        let block1 = tx.input(b"CC", 0).bytes;
        assert_eq!(block1.len(), 133, "the second C crossed block 1 and is ignored");
        tx.drained(0);
        assert!(tx.input(b"C", 10).bytes.is_empty(), "also after it was sent");
        let mut rx = Receiver::new(Variant::Xmodem, Timing::default(), "f");
        rx.start(0);
        let ack = rx.input(&block1, 0).bytes;
        assert_eq!(ack, [ACK]);
        let block2 = tx.input(&ack, 0).bytes;
        assert_eq!(block2[..3], [SOH, 2, !2]);
        // A stale ACK arriving while block 2 is still being written must not skip it.
        assert!(tx.input(&[ACK], 0).bytes.is_empty());
        tx.drained(0);
        let o = tx.input(&rx.input(&block2, 0).bytes, 0);
        assert_eq!(o.bytes, [EOT]);
        tx.drained(0);
        let o = rx.input(&o.bytes, 0);
        tx.input(&o.bytes, 0);
        assert!(tx.is_done());
        assert_eq!(received(&o.events)[0].data, data);
    }

    #[test]
    fn timeouts_and_cancel() {
        let mut tx = Sender::new(Variant::Xmodem, vec![file("f", vec![1])], Timing::default()).unwrap();
        tx.start(0);
        let o = tx.tick(60_000);
        assert!(matches!(o.events[..], [Event::Failed(_)]));
        assert!(o.bytes.starts_with(&[CAN, CAN]));

        let mut rx = Receiver::new(Variant::Ymodem, Timing::default(), "f");
        let mut sent = rx.start(0).bytes;
        let mut now = 0;
        while !rx.is_done() {
            rx.drained(now);
            now = rx.deadline().unwrap();
            sent.extend(rx.tick(now).bytes);
        }
        assert!(now >= 60_000);
        let mut expected = vec![b'C'; 20];
        expected.extend(cancel_sequence());
        assert_eq!(sent, expected, "start requests every 3 s, then cancel");

        let mut rx = Receiver::new(Variant::Xmodem, Timing::default(), "f");
        rx.start(0);
        let o = rx.input(&[CAN, CAN], 0);
        assert_eq!(o.events, [Event::Failed("cancelled by the sender".into())]);
        let mut tx = Sender::new(Variant::Ymodem, vec![file("f", vec![1])], Timing::default()).unwrap();
        let o = tx.cancel();
        assert!(tx.is_done() && o.bytes.starts_with(&[CAN; 8]));
    }

    #[test]
    fn batch_ends_when_the_sender_skips_the_end_header() {
        let mut rx = Receiver::new(Variant::YmodemG, Timing::default(), "f");
        rx.start(0);
        let mut tx = Sender::new(
            Variant::YmodemG,
            vec![file("a", vec![1, 2, 3])],
            Timing::default(),
        )
        .unwrap();
        tx.start(0);
        // Run until the receiver asks for the next header, then let the sender vanish.
        let mut to_tx = vec![WANT_G];
        let mut got = Vec::new();
        for _ in 0..20 {
            let o = tx.input(&to_tx, 0);
            tx.drained(0);
            let r = rx.input(&o.bytes, 0);
            got.extend(r.events);
            to_tx = r.bytes;
            if rx.complete_if_closed() {
                break;
            }
        }
        assert_eq!(received(&got).len(), 1);
        assert!(rx.complete_if_closed());
        let mut now = 0;
        while !rx.is_done() {
            rx.drained(now);
            now = rx.deadline().unwrap();
            got.extend(rx.tick(now).events);
        }
        assert_eq!(got.last(), Some(&Event::Finished));
        assert!(now < 15_000, "gives up on the end header after a few requests");
    }

    #[test]
    fn a_lost_final_ack_completes_the_upload() {
        let data = pattern(300, 4);
        let mut tx = Sender::new(Variant::Xmodem, vec![file("f", data.clone())], Timing::default()).unwrap();
        let mut rx = Receiver::new(Variant::Xmodem, Timing::default(), "f");
        // Drop everything the receiver sends after it saw the EOT.
        let l = run(
            &mut tx,
            &mut rx,
            Box::new(|b, to_rx| {
                if !to_rx && b.first() == Some(&ACK) && b.len() == 1 {
                    // keep ACKs of data blocks; the EOT ACK is lost below via the receiver state
                }
            }),
        );
        assert_eq!(received(&l.events_rx)[0].data, data);
        let mut tx = Sender::new(Variant::Xmodem, vec![file("f", vec![1])], Timing::default()).unwrap();
        tx.start(0);
        tx.input(b"C", 0);
        tx.drained(0);
        tx.input(&[ACK], 0);
        tx.drained(0);
        assert!(tx.complete_if_closed(), "only the EOT is unacknowledged");
        let mut events = Vec::new();
        let mut now = 0;
        while !tx.is_done() {
            now = tx.deadline().unwrap();
            events.extend(tx.tick(now).events);
            tx.drained(now);
        }
        assert_eq!(events.last(), Some(&Event::Finished), "{:?}", brief(&events));
        assert!(now <= 5_000, "{now}");
    }

    #[test]
    fn ymodem_g_aborts_on_errors() {
        let data = pattern(4000, 2);
        let mut tx = Sender::new(Variant::YmodemG, vec![file("f", data)], Timing::default()).unwrap();
        let mut rx = Receiver::new(Variant::YmodemG, Timing::default(), "f");
        let mut hit = false;
        let l = run(
            &mut tx,
            &mut rx,
            Box::new(move |b, to_rx| {
                if to_rx && !hit && b.len() > 1000 {
                    b[500] ^= 1;
                    hit = true;
                }
            }),
        );
        assert!(
            l.events_rx
                .iter()
                .any(|e| matches!(e, Event::Failed(m) if m.contains("YMODEM-G"))),
            "{:?}",
            brief(&l.events_rx)
        );
        assert!(l.events_tx.iter().any(|e| matches!(e, Event::Failed(_))));
    }

    #[test]
    fn header_format() {
        let h = header_payload(Some(&file("fw.bin", vec![0; 1234])));
        assert!(h.starts_with(b"fw.bin\x001234 14524770400 100644\x00"));
        assert_eq!(h.len(), 128);
        assert_eq!(
            parse_header(&h),
            Some(("fw.bin".into(), Some(1234), Some(1_700_000_000)))
        );
        assert_eq!(parse_header(&header_payload(None)), None);
        assert_eq!(header_payload(Some(&file(&"n".repeat(200), vec![]))).len(), 1024);
        assert_eq!(Variant::parse("XMODEM-1K"), Some(Variant::Xmodem1k));
        for v in Variant::ALL {
            assert_eq!(Variant::parse(v.id()), Some(v));
        }
    }
}
