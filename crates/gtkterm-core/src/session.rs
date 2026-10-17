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
use tracing::{debug, info, warn};

use crate::chunks::{Chunk, ChunkStore, Direction, RxRead};
use crate::config::{Connection, TerminalConfig};
use crate::logging::{LogStatus, LogWriter};
use crate::port::{self, ModemLines, PortError, PortKind, PortParts, PortRx, PortWriter, SerialControl};
use crate::sunshine::{SunshineStore, SunshineUpdate};
use gtkterm_protocols::logfmt::{LogFormat, LogMeta};

/// Session identifier, unique within a [`crate::manager::SessionManager`].
pub type SessionId = u32;

/// How long the writer waits for the configured "wait char" before sending the next line.
const WAIT_CHAR_TIMEOUT: Duration = Duration::from_secs(2);
/// Modem line polling interval.
const MODEM_POLL: Duration = Duration::from_millis(100);
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
}

#[derive(Debug)]
struct Shared {
    status: Mutex<SessionStatus>,
    chunks: Mutex<ChunkStore>,
    sunshine: Mutex<SunshineStore>,
    terminal: Mutex<TerminalConfig>,
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
            }),
            chunks: Mutex::new(ChunkStore::default()),
            sunshine: Mutex::new(SunshineStore::new()),
            terminal: Mutex::new(terminal),
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
        s
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
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
        loop {
            let read = self.read_rx(cursor, usize::MAX);
            let start = read.next_cursor - read.data.len() as u64;
            if let Some(caps) = re.captures(&read.data) {
                let m = caps.get(0).expect("group 0 always exists");
                let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
                return Ok(ExpectMatch {
                    matched: text(m.as_bytes()),
                    captures: caps
                        .iter()
                        .skip(1)
                        .map(|c| c.map(|c| text(c.as_bytes())))
                        .collect(),
                    before: text(&read.data[..m.start()]),
                    cursor: start + m.end() as u64,
                });
            }
            loop {
                match tokio::time::timeout_at(deadline, events.recv()).await {
                    Err(_) => return Err(SessionError::Timeout(timeout)),
                    Ok(Ok(SessionEvent::Data(c))) if c.dir == Direction::Rx => break,
                    Ok(Ok(SessionEvent::Cleared) | Err(broadcast::error::RecvError::Lagged(_))) => break,
                    Ok(Ok(_)) => {}
                    Ok(Err(broadcast::error::RecvError::Closed)) => return Err(SessionError::Shutdown),
                }
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
}

enum WriterEvent {
    Written(Bytes),
    Failed(PortError),
}

struct Actor {
    shared: Arc<Shared>,
    events: broadcast::Sender<SessionEvent>,
    pacing: watch::Receiver<Pacing>,
    port: Option<ActivePort>,
    log: Option<LogWriter>,
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
        loop {
            let poll_modem = self.port.as_ref().is_some_and(|p| p.modem_supported);
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
                self.teardown(PortState::Closed).await;
                let _ = reply.send(());
            }
            Command::Send(data, done) => match &self.port {
                Some(p) => {
                    self.shared.tx_queue.fetch_add(1, Ordering::Relaxed);
                    if let Err(e) = p.jobs.send(TxJob { data, done }) {
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
                let _ = reply.send(self.log_op(op));
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
        }
        Ok(())
    }

    async fn open(&mut self, conn: Connection) -> Result<(), SessionError> {
        self.teardown(PortState::Closed).await;
        lock(&self.shared.status).description = conn.summary();
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
        self.set_state(PortState::Open);
        if let Some(m) = modem {
            self.emit(SessionEvent::Modem(m));
        }
    }

    async fn teardown(&mut self, state: PortState) {
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
        }
        self.set_state(state);
    }

    async fn on_port_rx(&mut self, rx: Option<PortRx>) {
        match rx {
            Some(PortRx::Data(data)) => {
                let ts = Timestamp::now();
                let updates = lock(&self.shared.sunshine).feed(&data, ts);
                let chunk = lock(&self.shared.chunks).push(Direction::Rx, data, ts);
                lock(&self.shared.status).rx_bytes += chunk.data.len() as u64;
                self.log_chunk(&chunk);
                self.emit(SessionEvent::Data(chunk));
                for u in updates {
                    self.emit(SessionEvent::Sunshine(u));
                }
            }
            Some(PortRx::Info(msg)) => self.emit(SessionEvent::Info(msg)),
            Some(PortRx::Closed(err)) => {
                let state = err.map_or(PortState::Closed, |e| PortState::Error(e.to_string()));
                self.teardown(state).await;
            }
            None => self.teardown(PortState::Closed).await,
        }
    }

    async fn on_written(&mut self, ev: Option<WriterEvent>) {
        match ev {
            Some(WriterEvent::Written(data)) => {
                let chunk = lock(&self.shared.chunks).push(Direction::Tx, data, Timestamp::now());
                lock(&self.shared.status).tx_bytes += chunk.data.len() as u64;
                self.log_chunk(&chunk);
                self.emit(SessionEvent::Data(chunk));
            }
            Some(WriterEvent::Failed(e)) => {
                warn!(error = %e, "write failed, closing port");
                self.teardown(PortState::Error(e.to_string())).await;
            }
            None => {}
        }
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
                let changed = {
                    let mut st = lock(&self.shared.status);
                    let changed = st.modem != Some(m);
                    st.modem = Some(m);
                    changed
                };
                if changed {
                    self.emit(SessionEvent::Modem(m));
                }
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
                let _ = written.send(WriterEvent::Failed(e)).await;
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
