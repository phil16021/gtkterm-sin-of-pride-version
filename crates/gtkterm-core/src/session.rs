//! A session owns one connection and everything derived from its data.
//!
//! [`Session`] is a cheap, cloneable handle. Commands go to an actor task over a channel;
//! read access (history, Sunshine data, status) goes straight to shared state behind short
//! mutexes, so the GUI and the MCP server never wait for the actor to answer a query.
//! Every change is broadcast as a [`SessionEvent`].
//!
//! Transmission runs in a separate writer task so a slow device (CTS low, line pacing) never
//! blocks reception. Bytes are recorded as TX chunks only after they were written.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bytes::Bytes;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use regex::bytes::Regex;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::{debug, info, trace, warn};

use crate::chunks::{Chunk, ChunkStore, Direction, RxRead};
use crate::config::{Connection, TerminalConfig};
use crate::decoding::{
    DecodedFrame, DecoderConfig, DecoderMode, DecoderStatus, Decoding, FrameSource, FramesRead,
};
use crate::logging::{LogStatus, LogWriter};
use crate::port::{self, ModemLines, PortError, PortKind, PortParts, PortRx, PortWriter, SerialControl};
use crate::signals::{SignalHistory, SignalSample};
use crate::sunshine::{SunshineStore, SunshineUpdate};
use crate::transfer::{ActiveTransfer, Step, TransferRequest, TransferState, TransferStatus};
use gtkterm_protocols::logfmt::{LogFormat, LogMeta};

/// Session identifier, unique within a [`crate::manager::SessionManager`].
pub type SessionId = u32;

/// `expect` re-searches this many bytes before newly received data, so matches that started
/// earlier are still found; longer matches spanning a read boundary are not.
const EXPECT_OVERLAP: usize = 64 * 1024;
/// How long closing the log or the port waits for queued sends to be reported.
const SETTLE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long the writer waits for the configured "wait char" before sending the next line.
const WAIT_CHAR_TIMEOUT: Duration = Duration::from_secs(2);
/// Modem line polling interval.
const MODEM_POLL: Duration = Duration::from_millis(50);
/// Decoder poll interval while a gap-framed protocol (Modbus RTU) is active.
const DECODE_POLL: Duration = Duration::from_millis(10);
/// Capacity of the event broadcast; slow subscribers lag and resync from the stores.
const EVENT_CAPACITY: usize = 4096;

/// Connection state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum PortState {
    /// Not connected.
    Closed,
    /// Connecting.
    Opening,
    /// Connected.
    Open,
    /// Closed because of an error.
    Error(String),
}

/// Snapshot of a session for status bars and MCP.
#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    /// Session ID.
    pub id: SessionId,
    /// Display name (usually the profile).
    pub name: String,
    /// Connection state.
    pub state: PortState,
    /// Endpoint description of the open port, or of the last connection attempt.
    pub description: String,
    /// Backend kind while open.
    pub kind: Option<PortKind>,
    /// When the port was opened.
    pub opened_at: Option<Timestamp>,
    /// Bytes received since the session was created.
    pub rx_bytes: u64,
    /// Bytes transmitted since the session was created.
    pub tx_bytes: u64,
    /// Transmit jobs waiting or in progress.
    pub tx_queue: usize,
    /// Last known modem lines (serial ports that support them).
    pub modem: Option<ModemLines>,
    /// Session log, if logging.
    pub log: Option<LogStatus>,
    /// Protocol decoder.
    pub decoder: DecoderStatus,
    /// Running or last file transfer.
    pub transfer: Option<TransferStatus>,
    /// Kernel line counters (serial ports whose driver has them).
    pub counters: Option<crate::port::LineCounters>,
}

/// Something that happened in a session.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Connection state changed.
    State(PortState),
    /// Data received or transmitted.
    Data(Chunk),
    /// Informational message from the backend.
    Info(String),
    /// Modem lines changed.
    Modem(ModemLines),
    /// Sunshine charts changed.
    Sunshine(SunshineUpdate),
    /// History was cleared.
    Cleared,
    /// Frames were decoded.
    Frames(Vec<DecodedFrame>),
    /// Decoder settings changed or a protocol was detected.
    Decoder(DecoderStatus),
    /// File transfer progress or result.
    Transfer(TransferStatus),
    /// New signal monitor samples.
    Signals(Vec<SignalSample>),
}

/// Session errors.
#[derive(Debug, Error)]
pub enum SessionError {
    /// Port failure.
    #[error(transparent)]
    Port(#[from] PortError),
    /// No port is open.
    #[error("port is not open")]
    NotOpen,
    /// The session actor stopped.
    #[error("session has shut down")]
    Shutdown,
    /// `expect` timed out.
    #[error("timed out after {0:?} waiting for a match")]
    Timeout(Duration),
    /// Writing failed; the port was closed.
    #[error("write failed: {0}")]
    Write(String),
    /// Log file error.
    #[error("log file: {0}")]
    Log(#[from] std::io::Error),
    /// No log is running.
    #[error("not logging")]
    NotLogging,
    /// A file transfer owns the port.
    #[error("a file transfer is in progress")]
    Busy,
    /// A file transfer could not start.
    #[error("{0}")]
    Transfer(String),
}

/// Successful [`Session::expect`].
#[derive(Debug, Clone, Serialize)]
pub struct ExpectMatch {
    /// The matched text (lossy UTF-8).
    pub matched: String,
    /// Capture groups 1..n.
    pub captures: Vec<Option<String>>,
    /// Received data between the start cursor and the match.
    pub before: String,
    /// Cursor just after the match.
    pub cursor: u64,
}

enum Command {
    Open(Connection, oneshot::Sender<Result<(), SessionError>>),
    Attach(port::OpenPort, oneshot::Sender<()>),
    Close(oneshot::Sender<()>),
    Send(Bytes, Option<oneshot::Sender<Result<(), SessionError>>>),
    Control(ControlOp, oneshot::Sender<Result<(), SessionError>>),
    Log(LogOp, oneshot::Sender<Result<Option<LogStatus>, SessionError>>),
    Transfer(TransferRequest, oneshot::Sender<Result<(), SessionError>>),
    CancelTransfer(oneshot::Sender<bool>),
}

/// Log control operations.
#[derive(Debug, Clone)]
pub enum LogOp {
    /// Start logging to a file (replaces a running log).
    Start {
        /// Target file.
        path: std::path::PathBuf,
        /// Format.
        format: LogFormat,
        /// Append to an existing raw/text log instead of truncating.
        append: bool,
        /// Endpoint written into structured headers; default: the current port description.
        port_label: Option<String>,
    },
    /// Pause (`true`) or resume (`false`).
    Pause(bool),
    /// Truncate the file and continue.
    Clear,
    /// Finish the file.
    Stop,
}

enum ControlOp {
    Dtr(bool),
    Rts(bool),
    Break(Duration),
    Flush,
}

#[derive(Debug)]
struct Shared {
    status: Mutex<SessionStatus>,
    chunks: Mutex<ChunkStore>,
    sunshine: Mutex<SunshineStore>,
    terminal: Mutex<TerminalConfig>,
    connection: Mutex<Option<Connection>>,
    decoding: Mutex<Decoding>,
    transfer: Mutex<Option<TransferStatus>>,
    signals: Mutex<SignalHistory>,
    tx_queue: AtomicUsize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Handle to a session.
#[derive(Debug, Clone)]
pub struct Session {
    shared: Arc<Shared>,
    cmd: mpsc::Sender<Command>,
    events: broadcast::Sender<SessionEvent>,
    pacing: watch::Sender<Pacing>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pacing {
    delay: Duration,
    wait_char: Option<u8>,
}

impl Pacing {
    fn from_terminal(t: &TerminalConfig) -> Self {
        Self {
            delay: Duration::from_millis(t.wait_delay_ms.into()),
            wait_char: t.wait_char,
        }
    }

    fn active(self) -> bool {
        !self.delay.is_zero() || self.wait_char.is_some()
    }
}

impl Session {
    /// Create a session and spawn its actor. Must run inside a tokio runtime.
    pub fn spawn(id: SessionId, name: impl Into<String>, terminal: TerminalConfig) -> Self {
        let pacing = Pacing::from_terminal(&terminal);
        let shared = Arc::new(Shared {
            status: Mutex::new(SessionStatus {
                id,
                name: name.into(),
                state: PortState::Closed,
                description: String::new(),
                kind: None,
                opened_at: None,
                rx_bytes: 0,
                tx_bytes: 0,
                tx_queue: 0,
                modem: None,
                log: None,
                decoder: Decoding::default().status(),
                transfer: None,
                counters: None,
            }),
            chunks: Mutex::new(ChunkStore::default()),
            sunshine: Mutex::new(SunshineStore::new()),
            terminal: Mutex::new(terminal),
            connection: Mutex::new(None),
            decoding: Mutex::new(Decoding::default()),
            transfer: Mutex::new(None),
            signals: Mutex::default(),
            tx_queue: AtomicUsize::new(0),
        });
        let (cmd, cmd_rx) = mpsc::channel(64);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (pacing, pacing_rx) = watch::channel(pacing);
        let actor = Actor {
            shared: Arc::clone(&shared),
            events: events.clone(),
            pacing: pacing_rx,
            port: None,
            log: None,
            transfer: None,
            transfer_writes: 0,
            transfer_held: Vec::new(),
            transfer_emitted: std::time::Instant::now(),
        };
        tokio::spawn(actor.run(cmd_rx));
        Self {
            shared,
            cmd,
            events,
            pacing,
        }
    }

    /// Session ID.
    pub fn id(&self) -> SessionId {
        lock(&self.shared.status).id
    }

    /// Current status.
    pub fn status(&self) -> SessionStatus {
        let mut s = lock(&self.shared.status).clone();
        s.tx_queue = self.shared.tx_queue.load(Ordering::Relaxed);
        s.decoder = self.decoder_status();
        s.transfer = self.transfer_status();
        s
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Connection settings of the last `open` (`None` before the first open or for attached ports).
    pub fn connection(&self) -> Option<Connection> {
        lock(&self.shared.connection).clone()
    }

    /// Terminal settings.
    pub fn terminal(&self) -> TerminalConfig {
        lock(&self.shared.terminal).clone()
    }

    /// Replace terminal settings (pacing applies to the next transmitted line).
    pub fn set_terminal(&self, terminal: TerminalConfig) {
        self.pacing.send_replace(Pacing::from_terminal(&terminal));
        *lock(&self.shared.terminal) = terminal;
    }

    async fn request<T>(&self, make: impl FnOnce(oneshot::Sender<T>) -> Command) -> Result<T, SessionError> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(make(tx))
            .await
            .map_err(|_| SessionError::Shutdown)?;
        rx.await.map_err(|_| SessionError::Shutdown)
    }

    /// Open (or reopen) the connection.
    pub async fn open(&self, connection: Connection) -> Result<(), SessionError> {
        self.request(|tx| Command::Open(connection, tx)).await?
    }

    /// Use an already open port (loopback ports in tests, custom backends).
    pub async fn attach(&self, port: port::OpenPort) -> Result<(), SessionError> {
        self.request(|tx| Command::Attach(port, tx)).await
    }

    /// Close the connection; waits until the device is released.
    pub async fn close(&self) -> Result<(), SessionError> {
        self.request(Command::Close).await
    }

    /// Queue bytes for transmission and wait until they were written.
    pub async fn send(&self, data: impl Into<Bytes>) -> Result<(), SessionError> {
        let data = data.into();
        self.request(|tx| Command::Send(data, Some(tx))).await?
    }

    /// Queue bytes without waiting (keyboard input).
    pub fn send_nowait(&self, data: impl Into<Bytes>) -> Result<(), SessionError> {
        self.cmd
            .try_send(Command::Send(data.into(), None))
            .map_err(|_| SessionError::Shutdown)
    }

    /// Set DTR.
    pub async fn set_dtr(&self, on: bool) -> Result<(), SessionError> {
        self.request(|tx| Command::Control(ControlOp::Dtr(on), tx))
            .await?
    }

    /// Set RTS.
    pub async fn set_rts(&self, on: bool) -> Result<(), SessionError> {
        self.request(|tx| Command::Control(ControlOp::Rts(on), tx))
            .await?
    }

    /// Send a break.
    pub async fn send_break(&self, duration: Duration) -> Result<(), SessionError> {
        self.request(|tx| Command::Control(ControlOp::Break(duration), tx))
            .await?
    }

    /// Send a break as long as the connection's `break_ms` (250 ms by default).
    pub async fn send_default_break(&self) -> Result<(), SessionError> {
        let ms = self
            .connection()
            .and_then(|c| c.serial().map(|s| s.break_ms))
            .unwrap_or(250);
        self.send_break(Duration::from_millis(ms.max(1).into())).await
    }

    /// Discard data waiting in the driver's input and output buffers (serial ports).
    pub async fn flush(&self) -> Result<(), SessionError> {
        self.request(|tx| Command::Control(ControlOp::Flush, tx)).await?
    }

    /// Control the session log; returns the log status afterwards (`Stop` returns the final one).
    pub async fn log(&self, op: LogOp) -> Result<Option<LogStatus>, SessionError> {
        self.request(|tx| Command::Log(op, tx)).await?
    }

    /// Run `f` with the chunk history.
    pub fn with_chunks<R>(&self, f: impl FnOnce(&ChunkStore) -> R) -> R {
        f(&lock(&self.shared.chunks))
    }

    /// Read received bytes from `cursor`.
    pub fn read_rx(&self, cursor: u64, max: usize) -> RxRead {
        lock(&self.shared.chunks).read_rx(cursor, max)
    }

    /// Current end of the received byte stream (a cursor that only sees new data).
    pub fn rx_cursor(&self) -> u64 {
        lock(&self.shared.chunks).rx_total()
    }

    /// Clear the history and the Sunshine charts.
    pub fn clear(&self) {
        lock(&self.shared.chunks).clear();
        lock(&self.shared.sunshine).clear();
        let _ = self.events.send(SessionEvent::Cleared);
    }

    /// Start a file transfer; progress and the result arrive as [`SessionEvent::Transfer`].
    pub async fn start_transfer(&self, req: TransferRequest) -> Result<(), SessionError> {
        self.request(|tx| Command::Transfer(req, tx)).await?
    }

    /// Run a file transfer to the end; `progress` sees every status update.
    pub async fn transfer(
        &self,
        req: TransferRequest,
        mut progress: impl FnMut(&TransferStatus),
    ) -> Result<TransferStatus, SessionError> {
        let mut events = self.subscribe();
        self.start_transfer(req).await?;
        loop {
            match events.recv().await {
                Ok(SessionEvent::Transfer(st)) => {
                    progress(&st);
                    if st.state != TransferState::Running {
                        return Ok(st);
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if let Some(st) = self
                        .transfer_status()
                        .filter(|s| s.state != TransferState::Running)
                    {
                        return Ok(st);
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return Err(SessionError::Shutdown),
            }
        }
    }

    /// Cancel the running transfer; `false` if none was running.
    pub async fn cancel_transfer(&self) -> bool {
        self.request(Command::CancelTransfer).await.unwrap_or(false)
    }

    /// Signal monitor samples of the last 300 s (or since `since`, with the state before it).
    pub fn signal_history(&self, since: Option<Timestamp>) -> Vec<SignalSample> {
        lock(&self.shared.signals).since(since)
    }

    /// The signal history as CSV (`Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals`).
    pub fn signals_csv(&self) -> String {
        lock(&self.shared.signals).to_csv(&TimeZone::system())
    }

    /// Clear the signal history.
    pub fn clear_signals(&self) {
        lock(&self.shared.signals).clear();
    }

    /// Pause or resume the signal history.
    pub fn pause_signals(&self, paused: bool) {
        lock(&self.shared.signals).set_paused(paused);
    }

    /// The running or last transfer.
    pub fn transfer_status(&self) -> Option<TransferStatus> {
        lock(&self.shared.transfer).clone()
    }

    /// Decoder state.
    pub fn decoder_status(&self) -> DecoderStatus {
        lock(&self.shared.decoding).status()
    }

    /// Select the protocol decoder; `signals` also records modem line changes as frames.
    pub fn set_decoder(&self, mode: DecoderMode, signals: bool) -> DecoderStatus {
        let status = {
            let mut d = lock(&self.shared.decoding);
            d.set_mode(mode);
            d.set_signals(signals);
            d.status()
        };
        let _ = self.events.send(SessionEvent::Decoder(status.clone()));
        status
    }

    /// Decoder settings (for saving into a profile).
    pub fn decoder_config(&self) -> DecoderConfig {
        lock(&self.shared.decoding).config()
    }

    /// Apply decoder settings from a profile.
    pub fn apply_decoder_config(&self, c: DecoderConfig) -> DecoderStatus {
        self.set_decoder(c.mode, c.signals)
    }

    /// Decoded frames from sequence number `since`.
    pub fn frames_since(&self, since: u64, max: usize) -> FramesRead {
        lock(&self.shared.decoding).store.since(since, max)
    }

    /// The newest `n` decoded frames.
    pub fn last_frames(&self, n: usize) -> Vec<DecodedFrame> {
        lock(&self.shared.decoding).store.last(n)
    }

    /// Drop all decoded frames.
    pub fn clear_frames(&self) {
        let status = {
            let mut d = lock(&self.shared.decoding);
            d.store.clear();
            d.status()
        };
        let _ = self.events.send(SessionEvent::Decoder(status));
    }

    /// Wait for a decoded frame with `seq >= since` that satisfies `pred`.
    pub async fn expect_frame(
        &self,
        since: u64,
        timeout: Duration,
        pred: impl Fn(&DecodedFrame) -> bool,
    ) -> Result<DecodedFrame, SessionError> {
        let mut events = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut next = since;
        loop {
            let read = self.frames_since(next, usize::MAX);
            next = read.next_seq;
            if let Some(f) = read.frames.into_iter().find(|f| pred(f)) {
                return Ok(f);
            }
            loop {
                match tokio::time::timeout_at(deadline, events.recv()).await {
                    Err(_) => return Err(SessionError::Timeout(timeout)),
                    Ok(Ok(SessionEvent::Frames(_)) | Err(broadcast::error::RecvError::Lagged(_))) => break,
                    Ok(Ok(_)) => {}
                    Ok(Err(broadcast::error::RecvError::Closed)) => return Err(SessionError::Shutdown),
                }
            }
        }
    }

    /// Run `f` with the Sunshine store.
    pub fn with_sunshine<R>(&self, f: impl FnOnce(&SunshineStore) -> R) -> R {
        f(&lock(&self.shared.sunshine))
    }

    /// Run `f` with mutable access to the Sunshine store.
    pub fn with_sunshine_mut<R>(&self, f: impl FnOnce(&mut SunshineStore) -> R) -> R {
        f(&mut lock(&self.shared.sunshine))
    }

    /// Wait until received data starting at `cursor` matches `re`.
    pub async fn expect(
        &self,
        re: &Regex,
        cursor: u64,
        timeout: Duration,
    ) -> Result<ExpectMatch, SessionError> {
        let mut events = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        // Everything received since `cursor`, appended as it arrives; each round only searches
        // the new bytes plus EXPECT_OVERLAP before them (a match cannot end in old data, or it
        // would have been found already), so a fast stream costs linear, not quadratic, work.
        let first = self.read_rx(cursor, usize::MAX);
        let mut start = first.next_cursor - first.data.len() as u64;
        let mut buf = first.data;
        let mut searched = 0usize;
        loop {
            let from = searched.saturating_sub(EXPECT_OVERLAP);
            if let Some(caps) = re.captures_at(&buf, from) {
                let m = caps.get(0).expect("group 0 always exists");
                let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
                return Ok(ExpectMatch {
                    matched: text(m.as_bytes()),
                    captures: caps
                        .iter()
                        .skip(1)
                        .map(|c| c.map(|c| text(c.as_bytes())))
                        .collect(),
                    before: text(&buf[..m.start()]),
                    cursor: start + m.end() as u64,
                });
            }
            searched = buf.len();
            loop {
                match tokio::time::timeout_at(deadline, events.recv()).await {
                    Err(_) => return Err(SessionError::Timeout(timeout)),
                    Ok(Ok(SessionEvent::Data(c))) if c.dir == Direction::Rx => break,
                    Ok(Ok(SessionEvent::Cleared) | Err(broadcast::error::RecvError::Lagged(_))) => break,
                    Ok(Ok(_)) => {}
                    Ok(Err(broadcast::error::RecvError::Closed)) => return Err(SessionError::Shutdown),
                }
            }
            let more = self.read_rx(start + buf.len() as u64, usize::MAX);
            if more.lost > 0 {
                // History moved past what we had (cleared, or evicted): continue from what is left.
                start = more.next_cursor - more.data.len() as u64;
                buf = more.data;
                searched = 0;
            } else {
                buf.extend_from_slice(&more.data);
            }
        }
    }
}

struct ActivePort {
    kind: PortKind,
    rx: mpsc::Receiver<PortRx>,
    control: Option<Arc<dyn SerialControl>>,
    modem_supported: bool,
    jobs: mpsc::UnboundedSender<TxJob>,
    written: mpsc::Receiver<WriterEvent>,
    writer_task: tokio::task::JoinHandle<()>,
    closer: port::PortCloser,
}

struct TxJob {
    data: Bytes,
    done: Option<oneshot::Sender<Result<(), SessionError>>>,
    /// File transfer data: no pacing, not recorded as a chunk.
    raw: bool,
}

enum WriterEvent {
    Written(Bytes),
    RawWritten(usize),
    /// One write could not be delivered; the port stays open.
    Rejected {
        error: PortError,
        raw: bool,
    },
    Failed(PortError),
}

/// The writer's report for a failed write.
fn write_error(error: PortError, raw: bool) -> WriterEvent {
    if error.is_fatal() {
        WriterEvent::Failed(error)
    } else {
        WriterEvent::Rejected { error, raw }
    }
}

struct Actor {
    shared: Arc<Shared>,
    events: broadcast::Sender<SessionEvent>,
    pacing: watch::Receiver<Pacing>,
    port: Option<ActivePort>,
    log: Option<LogWriter>,
    transfer: Option<ActiveTransfer>,
    /// Transfer writes queued but not yet written.
    transfer_writes: usize,
    /// Received while transfer writes were pending; delivered after them, because the device
    /// cannot have answered bytes that were not written yet (the reader task may simply have
    /// been faster than the writer's completion report).
    transfer_held: Vec<u8>,
    transfer_emitted: std::time::Instant,
}

async fn transfer_event(t: &mut Option<ActiveTransfer>) -> crate::transfer::ExternalEvent {
    match t {
        Some(t) => t.external_event().await,
        None => std::future::pending().await,
    }
}

impl Actor {
    fn emit(&self, ev: SessionEvent) {
        let _ = self.events.send(ev);
    }

    fn set_state(&self, state: PortState) {
        lock(&self.shared.status).state = state.clone();
        self.emit(SessionEvent::State(state));
    }

    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Command>) {
        let mut modem_tick = tokio::time::interval(MODEM_POLL);
        modem_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut flush_tick = tokio::time::interval(Duration::from_millis(500));
        flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut decode_tick = tokio::time::interval(DECODE_POLL);
        decode_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let poll_modem = self.port.as_ref().is_some_and(|p| p.modem_supported);
            let poll_decoder = lock(&self.shared.decoding).needs_poll();
            let transfer_deadline = self.transfer.as_ref().and_then(ActiveTransfer::deadline);
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(cmd) => self.handle(cmd).await,
                    None => break,
                },
                signal = recv_port(&mut self.port) => match signal {
                    PortSignal::Rx(rx) => self.on_port_rx(rx).await,
                    PortSignal::Written(ev) => self.on_written(ev).await,
                },
                _ = modem_tick.tick(), if poll_modem => self.poll_modem(),
                _ = flush_tick.tick(), if self.log.is_some() => self.flush_log(),
                () = tokio::time::sleep_until(transfer_deadline.unwrap_or_else(tokio::time::Instant::now)),
                    if transfer_deadline.is_some() => {
                    if let Some(t) = self.transfer.as_mut() {
                        let step = t.tick();
                        self.transfer_step(step);
                    }
                }
                ev = transfer_event(&mut self.transfer) => {
                    if let Some(t) = self.transfer.as_mut() {
                        let step = t.on_external(ev);
                        self.transfer_step(step);
                    }
                }
                _ = decode_tick.tick(), if poll_decoder => {
                    let frames = lock(&self.shared.decoding).poll(Timestamp::now());
                    self.emit_frames(frames);
                }
            }
        }
        self.teardown(PortState::Closed).await;
        let _ = self.log_op(LogOp::Stop);
        debug!("session actor stopped");
    }

    async fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Open(conn, reply) => {
                let _ = reply.send(self.open(conn).await);
            }
            Command::Attach(port, reply) => {
                self.teardown(PortState::Closed).await;
                self.activate(port);
                let _ = reply.send(());
            }
            Command::Close(reply) => {
                self.settle_writes().await;
                self.teardown(PortState::Closed).await;
                let _ = reply.send(());
            }
            Command::Send(data, done) if self.transfer.is_some() => {
                drop(data);
                if let Some(done) = done {
                    let _ = done.send(Err(SessionError::Busy));
                }
            }
            Command::Send(data, done) => match &self.port {
                Some(p) => {
                    self.shared.tx_queue.fetch_add(1, Ordering::Relaxed);
                    if let Err(e) = p.jobs.send(TxJob {
                        data,
                        done,
                        raw: false,
                    }) {
                        self.shared.tx_queue.fetch_sub(1, Ordering::Relaxed);
                        if let Some(done) = e.0.done {
                            let _ = done.send(Err(SessionError::NotOpen));
                        }
                    }
                }
                None => {
                    if let Some(done) = done {
                        let _ = done.send(Err(SessionError::NotOpen));
                    }
                }
            },
            Command::Log(op, reply) => {
                if matches!(op, LogOp::Stop) {
                    self.settle_writes().await;
                }
                let _ = reply.send(self.log_op(op));
            }
            Command::Transfer(req, reply) => {
                let _ = reply.send(self.start_transfer(req).await);
            }
            Command::CancelTransfer(reply) => {
                let running = self.transfer.as_mut().map(ActiveTransfer::cancel);
                let _ = reply.send(running.is_some());
                if let Some(step) = running {
                    self.transfer_step(step);
                }
            }
            Command::Control(op, reply) => {
                let result = self.control(op).await;
                if result.is_ok() {
                    self.poll_modem();
                }
                let _ = reply.send(result);
            }
        }
    }

    async fn control(&self, op: ControlOp) -> Result<(), SessionError> {
        let port = self.port.as_ref().ok_or(SessionError::NotOpen)?;
        let ctl = port
            .control
            .clone()
            .ok_or_else(|| PortError::Unsupported(format!("{:?} ports have no control lines", port.kind)))?;
        match op {
            ControlOp::Dtr(on) => ctl.set_dtr(on)?,
            ControlOp::Rts(on) => ctl.set_rts(on)?,
            ControlOp::Break(d) => {
                ctl.set_break(true)?;
                tokio::time::sleep(d).await;
                ctl.set_break(false)?;
            }
            ControlOp::Flush => ctl.flush()?,
        }
        Ok(())
    }

    async fn open(&mut self, conn: Connection) -> Result<(), SessionError> {
        self.teardown(PortState::Closed).await;
        lock(&self.shared.status).description = conn.summary();
        lock(&self.shared.decoding).set_baud(conn.serial().map(|s| s.baudrate));
        *lock(&self.shared.connection) = Some(conn.clone());
        self.set_state(PortState::Opening);
        match port::open(&conn).await {
            Ok(p) => {
                self.activate(p);
                Ok(())
            }
            Err(e) => {
                self.set_state(PortState::Error(e.to_string()));
                Err(e.into())
            }
        }
    }

    fn activate(&mut self, opened: port::OpenPort) {
        let PortParts {
            kind,
            description,
            rx,
            writer,
            control,
            closer,
        } = opened.into_parts();
        let modem = control.as_ref().and_then(|c| c.modem_lines().ok());
        let (jobs, jobs_rx) = mpsc::unbounded_channel();
        let (written_tx, written) = mpsc::channel(64);
        let writer_task = tokio::spawn(writer_loop(
            writer,
            jobs_rx,
            written_tx,
            self.pacing.clone(),
            self.events.subscribe(),
            Arc::clone(&self.shared),
        ));
        {
            let mut st = lock(&self.shared.status);
            st.description.clone_from(&description);
            st.kind = Some(kind);
            st.opened_at = Some(Timestamp::now());
            st.modem = modem;
        }
        info!(%description, "port opened");
        self.port = Some(ActivePort {
            kind,
            rx,
            control,
            modem_supported: modem.is_some(),
            jobs,
            written,
            writer_task,
            closer,
        });
        lock(&self.shared.decoding).restart();
        self.set_state(PortState::Open);
        if let Some(m) = modem {
            self.emit(SessionEvent::Modem(m));
            self.decode_modem(m);
            let counters = self
                .port
                .as_ref()
                .and_then(|p| p.control.as_ref())
                .and_then(|c| c.counters().ok());
            let samples = lock(&self.shared.signals).observe(Timestamp::now(), m, counters);
            if !samples.is_empty() {
                self.emit(SessionEvent::Signals(samples));
            }
        }
    }

    async fn teardown(&mut self, state: PortState) {
        if let Some(t) = self.transfer.as_mut() {
            self.transfer_writes = 0;
            self.transfer_held.clear();
            if t.port_closed("the port was closed") {
                self.finish_transfer();
            }
        }
        let Some(port) = self.port.take() else { return };
        let ActivePort {
            jobs,
            writer_task,
            closer,
            rx,
            control,
            written,
            ..
        } = port;
        drop((jobs, rx, control, written));
        writer_task.abort();
        let _ = writer_task.await;
        closer.close().await;
        self.shared.tx_queue.store(0, Ordering::Relaxed);
        let updates = lock(&self.shared.sunshine).flush(Timestamp::now());
        for u in updates {
            self.emit(SessionEvent::Sunshine(u));
        }
        {
            let mut st = lock(&self.shared.status);
            st.kind = None;
            st.opened_at = None;
            st.modem = None;
            st.counters = None;
        }
        self.set_state(state);
    }

    async fn on_port_rx(&mut self, rx: Option<PortRx>) {
        match rx {
            Some(PortRx::Data(data)) if self.transfer.is_some() => {
                lock(&self.shared.status).rx_bytes += data.len() as u64;
                trace!(held = self.transfer_writes > 0, data = %preview(&data), "transfer rx");
                if self.transfer_writes > 0 {
                    self.transfer_held.extend_from_slice(&data);
                } else if let Some(t) = self.transfer.as_mut() {
                    let step = t.input(&data);
                    self.transfer_step(step);
                }
            }
            Some(PortRx::Data(data)) => {
                let ts = Timestamp::now();
                let updates = lock(&self.shared.sunshine).feed(&data, ts);
                let chunk = lock(&self.shared.chunks).push(Direction::Rx, data, ts);
                lock(&self.shared.status).rx_bytes += chunk.data.len() as u64;
                self.log_chunk(&chunk);
                let frames = self.decode(FrameSource::Rx, &chunk.data, ts);
                self.emit(SessionEvent::Data(chunk));
                for u in updates {
                    self.emit(SessionEvent::Sunshine(u));
                }
                self.emit_frames(frames);
            }
            Some(PortRx::Info(msg)) => self.emit(SessionEvent::Info(msg)),
            Some(PortRx::Closed(err)) => {
                let state = err.map_or(PortState::Closed, |e| PortState::Error(e.to_string()));
                self.teardown(state).await;
            }
            None => self.teardown(PortState::Closed).await,
        }
    }

    /// Record writes that already reached the port (the device may have answered them) before
    /// the log closes or the port goes away: their reports travel on another channel than
    /// commands. Waits up to [`SETTLE_TIMEOUT`] for queued sends.
    async fn settle_writes(&mut self) {
        let deadline = tokio::time::Instant::now() + SETTLE_TIMEOUT;
        loop {
            let Some(p) = self.port.as_mut() else { return };
            let ev = match p.written.try_recv() {
                Ok(ev) => ev,
                Err(_) if self.shared.tx_queue.load(Ordering::Relaxed) == 0 => return,
                Err(_) => match tokio::time::timeout_at(deadline, p.written.recv()).await {
                    Ok(Some(ev)) => ev,
                    _ => return,
                },
            };
            let failed = matches!(ev, WriterEvent::Failed(_));
            self.on_written(Some(ev)).await;
            if failed {
                return;
            }
        }
    }

    async fn on_written(&mut self, ev: Option<WriterEvent>) {
        match ev {
            Some(WriterEvent::Written(data)) => {
                let ts = Timestamp::now();
                let chunk = lock(&self.shared.chunks).push(Direction::Tx, data, ts);
                lock(&self.shared.status).tx_bytes += chunk.data.len() as u64;
                self.log_chunk(&chunk);
                let frames = self.decode(FrameSource::Tx, &chunk.data, ts);
                self.emit(SessionEvent::Data(chunk));
                self.emit_frames(frames);
            }
            Some(WriterEvent::RawWritten(n)) => {
                lock(&self.shared.status).tx_bytes += n as u64;
                self.transfer_writes = self.transfer_writes.saturating_sub(1);
                if self.transfer_writes == 0 {
                    if let Some(t) = self.transfer.as_mut() {
                        let step = t.drained();
                        self.transfer_step(step);
                    }
                    let held = std::mem::take(&mut self.transfer_held);
                    if self.transfer_writes == 0 && !held.is_empty() {
                        if let Some(t) = self.transfer.as_mut() {
                            let step = t.input(&held);
                            self.transfer_step(step);
                        }
                    } else {
                        self.transfer_held = held;
                    }
                }
            }
            Some(WriterEvent::Rejected { error, raw }) => {
                debug!(%error, "write rejected, port stays open");
                if raw {
                    // A transfer write that went nowhere; the protocol's timeouts handle it.
                    self.transfer_writes = self.transfer_writes.saturating_sub(1);
                }
                self.emit(SessionEvent::Info(format!("not sent: {error}")));
            }
            Some(WriterEvent::Failed(e)) => {
                warn!(error = %e, "write failed, closing port");
                self.teardown(PortState::Error(e.to_string())).await;
            }
            None => {}
        }
    }

    async fn start_transfer(&mut self, req: TransferRequest) -> Result<(), SessionError> {
        if self.transfer.is_some() {
            return Err(SessionError::Busy);
        }
        if self.port.is_none() {
            return Err(SessionError::NotOpen);
        }
        let protocol = req.protocol();
        let (t, step) = ActiveTransfer::start(req).await.map_err(SessionError::Transfer)?;
        info!(protocol = protocol.id(), "file transfer started");
        self.transfer = Some(t);
        self.transfer_writes = 0;
        self.transfer_held.clear();
        self.transfer_emitted = std::time::Instant::now();
        self.publish_transfer();
        self.transfer_step(step);
        Ok(())
    }

    fn publish_transfer(&mut self) {
        let Some(t) = self.transfer.as_ref() else { return };
        let st = t.status.clone();
        *lock(&self.shared.transfer) = Some(st.clone());
        self.transfer_emitted = std::time::Instant::now();
        self.emit(SessionEvent::Transfer(st));
    }

    fn transfer_step(&mut self, step: Step) {
        if !step.write.is_empty() {
            trace!(data = %preview(&step.write), "transfer tx");
            // Without a port (it closed under an lrzsz child that is finishing) the bytes are dropped.
            if let Some(p) = &self.port {
                self.shared.tx_queue.fetch_add(1, Ordering::Relaxed);
                let job = TxJob {
                    data: Bytes::from(step.write),
                    done: None,
                    raw: true,
                };
                if p.jobs.send(job).is_ok() {
                    self.transfer_writes += 1;
                } else {
                    self.shared.tx_queue.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
        if step.changed && self.transfer_emitted.elapsed() >= Duration::from_millis(100) {
            self.publish_transfer();
        }
        if self.transfer.as_ref().is_some_and(|t| t.finishing) && self.transfer_writes == 0 {
            self.finish_transfer();
        }
    }

    fn finish_transfer(&mut self) {
        let Some(mut t) = self.transfer.take() else { return };
        let st = t.finish_status();
        info!(summary = %st.summary(), "file transfer ended");
        *lock(&self.shared.transfer) = Some(st.clone());
        self.emit(SessionEvent::Info(st.summary()));
        self.emit(SessionEvent::Transfer(st));
    }

    fn decode(&self, source: FrameSource, data: &[u8], ts: Timestamp) -> Vec<DecodedFrame> {
        let mut d = lock(&self.shared.decoding);
        let detecting = d.status().detecting;
        let frames = d.feed(source, data, ts);
        if detecting && let Some(p) = d.protocol() {
            let status = d.status();
            drop(d);
            info!(protocol = p.id(), "decoder detected protocol");
            self.emit(SessionEvent::Info(format!("decoder: detected {}", p.label())));
            self.emit(SessionEvent::Decoder(status));
        }
        frames
    }

    fn emit_frames(&self, frames: Vec<DecodedFrame>) {
        if !frames.is_empty() {
            self.emit(SessionEvent::Frames(frames));
        }
    }

    fn decode_modem(&self, m: ModemLines) {
        let frame = lock(&self.shared.decoding).modem(m, Timestamp::now());
        self.emit_frames(frame.into_iter().collect());
    }

    fn log_op(&mut self, op: LogOp) -> Result<Option<LogStatus>, SessionError> {
        let result = match op {
            LogOp::Start {
                path,
                format,
                append,
                port_label,
            } => {
                if let Some(old) = self.log.take() {
                    let _ = old.finish();
                }
                let meta = {
                    let st = lock(&self.shared.status);
                    LogMeta {
                        start_time: Timestamp::now()
                            .to_zoned(TimeZone::system())
                            .strftime("%Y-%m-%dT%H:%M:%S%:z")
                            .to_string(),
                        port: port_label.unwrap_or_else(|| st.description.clone()),
                        profile: st.name.clone(),
                    }
                };
                let writer = LogWriter::create(&path, format, meta, append)?;
                let status = writer.status().clone();
                self.log = Some(writer);
                Ok(Some(status))
            }
            LogOp::Pause(paused) => {
                let w = self.log.as_mut().ok_or(SessionError::NotLogging)?;
                w.set_paused(paused)?;
                Ok(Some(w.status().clone()))
            }
            LogOp::Clear => {
                let w = self.log.as_mut().ok_or(SessionError::NotLogging)?;
                w.clear()?;
                Ok(Some(w.status().clone()))
            }
            LogOp::Stop => {
                let w = self.log.take().ok_or(SessionError::NotLogging)?;
                Ok(Some(w.finish()?))
            }
        };
        lock(&self.shared.status).log = self.log.as_ref().map(|w| w.status().clone());
        result
    }

    fn log_chunk(&mut self, chunk: &Chunk) {
        let Some(w) = self.log.as_mut() else { return };
        if let Err(e) = w.write(chunk, &TimeZone::system()) {
            warn!(error = %e, "log write failed, logging stopped");
            self.log = None;
            lock(&self.shared.status).log = None;
            self.emit(SessionEvent::Info(format!("logging stopped: {e}")));
        }
    }

    fn flush_log(&mut self) {
        if let Some(w) = self.log.as_mut() {
            let _ = w.flush();
            lock(&self.shared.status).log = Some(w.status().clone());
        }
    }

    fn poll_modem(&mut self) {
        let Some(port) = self.port.as_mut() else { return };
        let Some(ctl) = port.control.as_ref() else { return };
        match ctl.modem_lines() {
            Ok(m) => {
                let counters = ctl.counters().ok();
                lock(&self.shared.status).counters = counters;
                let samples = lock(&self.shared.signals).observe(Timestamp::now(), m, counters);
                if !samples.is_empty() {
                    self.emit(SessionEvent::Signals(samples));
                }
                let changed = {
                    let mut st = lock(&self.shared.status);
                    let changed = st.modem != Some(m);
                    st.modem = Some(m);
                    changed
                };
                if changed {
                    self.emit(SessionEvent::Modem(m));
                }
                self.decode_modem(m);
            }
            Err(_) => port.modem_supported = false,
        }
    }
}

enum PortSignal {
    Rx(Option<PortRx>),
    Written(Option<WriterEvent>),
}

/// Next thing from the open port: received data or a writer report. Pending while closed.
async fn recv_port(port: &mut Option<ActivePort>) -> PortSignal {
    let Some(p) = port else {
        return std::future::pending().await;
    };
    tokio::select! {
        rx = p.rx.recv() => PortSignal::Rx(rx),
        ev = p.written.recv() => PortSignal::Written(ev),
    }
}

/// Hex of the first and last bytes, for trace logs.
fn preview(d: &[u8]) -> String {
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
    if d.len() <= 16 {
        hex(d)
    } else {
        format!(
            "{} … {} ({} bytes)",
            hex(&d[..8]),
            hex(&d[d.len() - 4..]),
            d.len()
        )
    }
}

/// Split `data` into lines for paced transmission; each piece ends after `\n` or a lone `\r`.
fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        let end_of_line = b == b'\n' || (b == b'\r' && data.get(i + 1) != Some(&b'\n'));
        if end_of_line {
            out.push(&data[start..=i]);
            start = i + 1;
        }
    }
    if start < data.len() {
        out.push(&data[start..]);
    }
    out
}

async fn wait_for_byte(events: &mut broadcast::Receiver<SessionEvent>, byte: u8) {
    let wait = async {
        loop {
            match events.recv().await {
                Ok(SessionEvent::Data(c)) if c.dir == Direction::Rx && c.data.contains(&byte) => return,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    };
    if tokio::time::timeout(WAIT_CHAR_TIMEOUT, wait).await.is_err() {
        debug!(byte, "wait char not received, continuing");
    }
}

async fn writer_loop(
    mut writer: Box<dyn PortWriter>,
    mut jobs: mpsc::UnboundedReceiver<TxJob>,
    written: mpsc::Sender<WriterEvent>,
    pacing: watch::Receiver<Pacing>,
    mut events: broadcast::Receiver<SessionEvent>,
    shared: Arc<Shared>,
) {
    while let Some(job) = jobs.recv().await {
        if job.raw {
            let result = writer.write_all(&job.data).await;
            shared.tx_queue.fetch_sub(1, Ordering::Relaxed);
            match result {
                Ok(()) => {
                    let _ = written.send(WriterEvent::RawWritten(job.data.len())).await;
                }
                Err(e) => {
                    let _ = written.send(write_error(e, true)).await;
                }
            }
            continue;
        }
        let p = *pacing.borrow();
        let pieces = if p.active() {
            split_lines(&job.data)
        } else {
            vec![&job.data[..]]
        };
        let mut result = Ok(());
        for piece in pieces {
            if p.wait_char.is_some() {
                events = events.resubscribe(); // only bytes that arrive after this line count
            }
            if let Err(e) = writer.write_all(piece).await {
                let msg = e.to_string();
                let _ = written.send(write_error(e, false)).await;
                result = Err(SessionError::Write(msg));
                break;
            }
            let _ = written
                .send(WriterEvent::Written(job.data.slice_ref(piece)))
                .await;
            if piece.ends_with(b"\n") || piece.ends_with(b"\r") {
                if !p.delay.is_zero() {
                    tokio::time::sleep(p.delay).await;
                }
                if let Some(byte) = p.wait_char {
                    wait_for_byte(&mut events, byte).await;
                }
            }
        }
        shared.tx_queue.fetch_sub(1, Ordering::Relaxed);
        if let Some(done) = job.done {
            let _ = done.send(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_splitting() {
        assert_eq!(
            split_lines(b"a\r\nb\rc\nd"),
            vec![&b"a\r\n"[..], b"b\r", b"c\n", b"d"]
        );
        assert_eq!(split_lines(b""), Vec::<&[u8]>::new());
        assert_eq!(split_lines(b"\n\n"), vec![&b"\n"[..], b"\n"]);
    }
}
