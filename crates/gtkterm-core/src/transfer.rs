//! File transfers on a session: XMODEM/YMODEM through the sans-IO machines in
//! [`gtkterm_protocols::xmodem`], ZMODEM through lrzsz (`sz`/`rz`) with its stdin/stdout pumped
//! to and from the port.
//!
//! While a transfer runs the session hands it all received data and refuses other
//! transmissions; protocol traffic is not recorded in the terminal history.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use bytes::Bytes;
use gtkterm_protocols::xmodem::{self, Event, FileData, Output, Receiver, Sender, Timing, Transfer, Variant};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// Transfer protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferProtocol {
    /// XMODEM (CRC, falls back to the checksum).
    Xmodem,
    /// XMODEM-1K.
    Xmodem1k,
    /// YMODEM batch.
    Ymodem,
    /// YMODEM-G streaming.
    YmodemG,
    /// ZMODEM via lrzsz.
    Zmodem,
}

impl TransferProtocol {
    /// All protocols in menu order.
    pub const ALL: [Self; 5] = [
        Self::Xmodem,
        Self::Xmodem1k,
        Self::Ymodem,
        Self::YmodemG,
        Self::Zmodem,
    ];

    /// Display name.
    pub fn label(self) -> &'static str {
        self.variant().map_or("ZMODEM", Variant::label)
    }

    /// Identifier for configs and MCP.
    pub fn id(self) -> &'static str {
        self.variant().map_or("zmodem", Variant::id)
    }

    /// Parse an identifier.
    pub fn parse(s: &str) -> Option<Self> {
        if matches!(s.trim().to_ascii_lowercase().as_str(), "zmodem" | "z") {
            return Some(Self::Zmodem);
        }
        Some(match Variant::parse(s)? {
            Variant::Xmodem => Self::Xmodem,
            Variant::Xmodem1k => Self::Xmodem1k,
            Variant::Ymodem => Self::Ymodem,
            Variant::YmodemG => Self::YmodemG,
        })
    }

    /// The native variant (`None` for ZMODEM).
    pub fn variant(self) -> Option<Variant> {
        match self {
            Self::Xmodem => Some(Variant::Xmodem),
            Self::Xmodem1k => Some(Variant::Xmodem1k),
            Self::Ymodem => Some(Variant::Ymodem),
            Self::YmodemG => Some(Variant::YmodemG),
            Self::Zmodem => None,
        }
    }

    /// Carries file names (the receiver names the files itself).
    pub fn batch(self) -> bool {
        self.variant().is_none_or(Variant::batch)
    }

    /// The protocol can be used (ZMODEM needs `sz` and `rz` in `PATH`).
    pub fn available(self) -> bool {
        self != Self::Zmodem || (which("sz").is_some() && which("rz").is_some())
    }
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(name))
            .find(|f| f.is_file())
    })
}

/// What to transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferRequest {
    /// Send files (XMODEM: exactly one).
    Send {
        /// Protocol.
        protocol: TransferProtocol,
        /// Files to send.
        files: Vec<PathBuf>,
    },
    /// Receive files into a directory.
    Receive {
        /// Protocol.
        protocol: TransferProtocol,
        /// Target directory (created if missing).
        dir: PathBuf,
        /// File name for XMODEM, which carries none (default: `xmodem-<time>.bin`).
        name: Option<String>,
        /// Replace existing files instead of adding ` (1)`, ` (2)`, ... to the name.
        overwrite: bool,
    },
}

impl TransferRequest {
    /// The protocol.
    pub fn protocol(&self) -> TransferProtocol {
        match self {
            Self::Send { protocol, .. } | Self::Receive { protocol, .. } => *protocol,
        }
    }
}

/// Direction of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    /// Uploading to the device.
    Send,
    /// Downloading from the device.
    Receive,
}

/// Transfer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    /// In progress.
    Running,
    /// Completed.
    Finished,
    /// Stopped by an error or the other side.
    Failed,
    /// Cancelled by us.
    Cancelled,
}

/// A file sent or received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferredFile {
    /// Name on the wire.
    pub name: String,
    /// Local path (for received files: where it was saved).
    pub path: PathBuf,
    /// Size in bytes.
    pub bytes: u64,
}

/// Progress and result of a transfer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransferStatus {
    /// Protocol.
    pub protocol: TransferProtocol,
    /// Direction.
    pub direction: TransferDirection,
    /// State.
    pub state: TransferState,
    /// Error or informational message.
    pub message: Option<String>,
    /// Current file name.
    pub file: Option<String>,
    /// Index of the current file.
    pub file_index: usize,
    /// Number of files when known (sending).
    pub files_total: Option<usize>,
    /// Bytes of the current file.
    pub bytes: u64,
    /// Size of the current file, if known.
    pub size: Option<u64>,
    /// Bytes of all files so far.
    pub total_bytes: u64,
    /// Repeated blocks and timeouts.
    pub retries: u32,
    /// Milliseconds since the start.
    pub elapsed_ms: u64,
    /// Completed files.
    pub files: Vec<TransferredFile>,
}

impl TransferStatus {
    /// Bytes per second over the whole transfer.
    pub fn rate(&self) -> f64 {
        if self.elapsed_ms == 0 {
            0.0
        } else {
            self.total_bytes as f64 * 1000.0 / self.elapsed_ms as f64
        }
    }

    /// Progress of the current file (0.0–1.0) when the size is known.
    pub fn fraction(&self) -> Option<f64> {
        self.size
            .filter(|s| *s > 0)
            .map(|s| (self.bytes as f64 / s as f64).min(1.0))
    }

    /// One-line description for status bars.
    pub fn summary(&self) -> String {
        let dir = match self.direction {
            TransferDirection::Send => "↑",
            TransferDirection::Receive => "↓",
        };
        let file = self.file.as_deref().unwrap_or("…");
        let progress = match self.fraction() {
            Some(f) => format!("{:.0}%", f * 100.0),
            None => human(self.bytes),
        };
        match self.state {
            TransferState::Running => format!(
                "{dir} {} {file} {progress} {}/s",
                self.protocol.label(),
                human(self.rate() as u64)
            ),
            TransferState::Finished => format!(
                "{} {} finished: {} file(s), {} in {:.1} s",
                self.protocol.label(),
                match self.direction {
                    TransferDirection::Send => "upload",
                    TransferDirection::Receive => "download",
                },
                self.files.len(),
                human(self.total_bytes),
                self.elapsed_ms as f64 / 1000.0
            ),
            TransferState::Failed | TransferState::Cancelled => format!(
                "{} {}: {}",
                self.protocol.label(),
                if self.state == TransferState::Cancelled {
                    "cancelled"
                } else {
                    "failed"
                },
                self.message.as_deref().unwrap_or("unknown error")
            ),
        }
    }
}

fn human(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KiB", n as f64 / 1024.0),
        _ => format!("{:.1} MiB", n as f64 / 1_048_576.0),
    }
}

/// A safe local file name for a name received from the device: no directories, no leading
/// dots, no control characters.
pub fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || c == ':' { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_owned();
    if cleaned.is_empty() {
        "received.bin".to_owned()
    } else {
        cleaned.chars().take(200).collect()
    }
}

/// `dir/name`, or `dir/name (1).ext`, ... if that exists and `overwrite` is false.
pub fn unique_path(dir: &Path, name: &str, overwrite: bool) -> PathBuf {
    let first = dir.join(name);
    if overwrite || !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..10_000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{stem} ({}){ext}", std::process::id())))
}

/// Result of feeding the engine: bytes to write to the port and whether the status changed.
#[derive(Debug, Default)]
pub(crate) struct Step {
    pub(crate) write: Vec<u8>,
    pub(crate) changed: bool,
}

enum Engine {
    Native(Box<dyn Transfer>),
    External(External),
}

struct External {
    child: tokio::process::Child,
    stdin: mpsc::UnboundedSender<Bytes>,
    events: mpsc::Receiver<ExternalEvent>,
    events_closed: bool,
    done: bool,
}

pub(crate) enum ExternalEvent {
    Output(Bytes),
    Stderr(String),
    Exited(Result<std::process::ExitStatus, String>),
}

/// The running transfer of a session.
pub(crate) struct ActiveTransfer {
    engine: Engine,
    pub(crate) status: TransferStatus,
    started: Instant,
    dir: Option<PathBuf>,
    overwrite: bool,
    /// The machine reported the end; finalised once the last bytes were written.
    pub(crate) finishing: bool,
    /// The port went away under an lrzsz transfer: its stdin was closed and it gets until this
    /// instant to exit on its own (its exit status says whether the files are complete).
    close_deadline: Option<(tokio::time::Instant, String)>,
}

impl std::fmt::Debug for ActiveTransfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveTransfer")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

fn mtime_secs(meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

impl ActiveTransfer {
    /// Prepare a transfer: read the files to send or spawn lrzsz. Returns the first bytes to write.
    pub(crate) async fn start(req: TransferRequest) -> Result<(Self, Step), String> {
        let protocol = req.protocol();
        let (direction, files_total, dir, overwrite) = match &req {
            TransferRequest::Send { files, .. } => {
                if files.is_empty() {
                    return Err("no files to send".into());
                }
                (TransferDirection::Send, Some(files.len()), None, false)
            }
            TransferRequest::Receive { dir, overwrite, .. } => {
                tokio::fs::create_dir_all(dir)
                    .await
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
                (TransferDirection::Receive, None, Some(dir.clone()), *overwrite)
            }
        };
        let status = TransferStatus {
            protocol,
            direction,
            state: TransferState::Running,
            message: None,
            file: None,
            file_index: 0,
            files_total,
            bytes: 0,
            size: None,
            total_bytes: 0,
            retries: 0,
            elapsed_ms: 0,
            files: Vec::new(),
        };
        let engine = match (protocol.variant(), &req) {
            (Some(variant), TransferRequest::Send { files, .. }) => {
                let mut data = Vec::with_capacity(files.len());
                for path in files {
                    let meta = tokio::fs::metadata(path)
                        .await
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    if !meta.is_file() {
                        return Err(format!("{} is not a file", path.display()));
                    }
                    if meta.len() > xmodem::MAX_FILE_SIZE {
                        return Err(format!(
                            "{} is larger than {} MiB",
                            path.display(),
                            xmodem::MAX_FILE_SIZE >> 20
                        ));
                    }
                    data.push(FileData {
                        name: path
                            .file_name()
                            .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned()),
                        data: tokio::fs::read(path)
                            .await
                            .map_err(|e| format!("{}: {e}", path.display()))?,
                        mtime: mtime_secs(&meta),
                    });
                }
                Engine::Native(Box::new(Sender::new(variant, data, Timing::default())?))
            }
            (Some(variant), TransferRequest::Receive { name, .. }) => {
                let default = name.clone().unwrap_or_else(|| {
                    format!("xmodem-{}.bin", jiff::Zoned::now().strftime("%Y%m%d_%H%M%S"))
                });
                Engine::Native(Box::new(Receiver::new(variant, Timing::default(), default)))
            }
            (None, req) => Engine::External(spawn_lrzsz(req)?),
        };
        let mut t = Self {
            engine,
            status,
            started: Instant::now(),
            dir,
            overwrite,
            finishing: false,
            close_deadline: None,
        };
        let out = match &mut t.engine {
            Engine::Native(m) => m.start(0),
            Engine::External(_) => Output::default(),
        };
        let step = t.apply(out);
        Ok((t, step))
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// When `tick` must run.
    pub(crate) fn deadline(&self) -> Option<tokio::time::Instant> {
        if let Some((at, _)) = &self.close_deadline {
            return Some(*at);
        }
        match &self.engine {
            Engine::Native(m) if !self.finishing => m.deadline().map(|ms| {
                tokio::time::Instant::from_std(self.started) + std::time::Duration::from_millis(ms)
            }),
            _ => None,
        }
    }

    pub(crate) fn input(&mut self, data: &[u8]) -> Step {
        let now = self.now_ms();
        match &mut self.engine {
            Engine::Native(m) => {
                let out = m.input(data, now);
                self.apply(out)
            }
            Engine::External(x) => {
                let _ = x.stdin.send(Bytes::copy_from_slice(data));
                Step::default()
            }
        }
    }

    pub(crate) fn drained(&mut self) -> Step {
        let now = self.now_ms();
        match &mut self.engine {
            Engine::Native(m) => {
                let out = m.drained(now);
                self.apply(out)
            }
            Engine::External(_) => Step::default(),
        }
    }

    pub(crate) fn tick(&mut self) -> Step {
        if let Some((at, reason)) = self.close_deadline.clone() {
            if tokio::time::Instant::now() >= at {
                self.close_deadline = None;
                self.abort(&reason);
                return Step {
                    write: Vec::new(),
                    changed: true,
                };
            }
            return Step::default();
        }
        let now = self.now_ms();
        match &mut self.engine {
            Engine::Native(m) => {
                let out = m.tick(now);
                self.apply(out)
            }
            Engine::External(_) => Step::default(),
        }
    }

    pub(crate) fn cancel(&mut self) -> Step {
        let step = match &mut self.engine {
            Engine::Native(m) => {
                let out = m.cancel();
                self.apply(out)
            }
            Engine::External(x) => {
                let _ = x.child.start_kill();
                x.done = true;
                Step {
                    write: xmodem::cancel_sequence(),
                    changed: true,
                }
            }
        };
        self.status.state = TransferState::Cancelled;
        self.status.message = Some("cancelled".into());
        self.finishing = true;
        step
    }

    /// The port closed under the transfer. Returns `true` if the transfer ended now; an lrzsz
    /// child first gets a chance to exit by itself.
    pub(crate) fn port_closed(&mut self, reason: &str) -> bool {
        if let Engine::External(x) = &mut self.engine
            && !x.done
            && self.status.state == TransferState::Running
        {
            let (closed, _) = mpsc::unbounded_channel();
            x.stdin = closed;
            self.close_deadline = Some((
                tokio::time::Instant::now() + std::time::Duration::from_secs(3),
                reason.to_owned(),
            ));
            return false;
        }
        self.abort(reason);
        true
    }

    /// Port closed or failed under the transfer.
    pub(crate) fn abort(&mut self, reason: &str) {
        match &mut self.engine {
            Engine::External(x) => {
                let _ = x.child.start_kill();
            }
            Engine::Native(m) if m.complete_if_closed() && self.status.state == TransferState::Running => {
                self.record_sent();
                self.status.state = TransferState::Finished;
                self.status.message = Some(format!(
                    "{reason} during the closing handshake; all data was acknowledged"
                ));
                self.finishing = true;
                return;
            }
            Engine::Native(_) => {}
        }
        if self.status.state == TransferState::Running {
            self.status.state = TransferState::Failed;
            self.status.message = Some(reason.to_owned());
        }
        self.finishing = true;
    }

    /// Next event of an lrzsz child; pending forever for native transfers.
    pub(crate) async fn external_event(&mut self) -> ExternalEvent {
        match &mut self.engine {
            Engine::External(x) if !x.done => loop {
                // Output first: the exit status may arrive before the last bytes were read.
                tokio::select! {
                    biased;
                    ev = x.events.recv(), if !x.events_closed => match ev {
                        Some(ev) => return ev,
                        None => x.events_closed = true,
                    },
                    status = x.child.wait(), if x.events_closed => {
                        x.done = true;
                        return ExternalEvent::Exited(status.map_err(|e| e.to_string()));
                    }
                }
            },
            _ => std::future::pending().await,
        }
    }

    pub(crate) fn on_external(&mut self, ev: ExternalEvent) -> Step {
        let mut step = Step::default();
        match ev {
            ExternalEvent::Output(b) => step.write = b.to_vec(),
            ExternalEvent::Stderr(line) => step.changed = self.parse_lrzsz(&line),
            ExternalEvent::Exited(r) => {
                if let Engine::External(x) = &mut self.engine {
                    x.done = true;
                }
                // Drain what is left of the output first.
                let mut rest = Vec::new();
                if let Engine::External(x) = &mut self.engine {
                    while let Ok(ev) = x.events.try_recv() {
                        rest.push(ev);
                    }
                }
                for ev in rest {
                    match ev {
                        ExternalEvent::Output(b) => step.write.extend_from_slice(&b),
                        ExternalEvent::Stderr(text) => {
                            self.parse_lrzsz(&text);
                        }
                        ExternalEvent::Exited(_) => {}
                    }
                }
                let closed = self.close_deadline.take();
                if self.status.state == TransferState::Running {
                    match r {
                        Ok(s) if s.success() => {
                            if let Some((_, reason)) = closed {
                                self.status.message = Some(format!("{reason}; lrzsz reported success"));
                            }
                            self.complete_current();
                            self.status.state = TransferState::Finished;
                        }
                        Ok(s) => {
                            self.status.state = TransferState::Failed;
                            self.status.message = Some(format!("lrzsz exited with {s}"));
                        }
                        Err(e) => {
                            self.status.state = TransferState::Failed;
                            self.status.message = Some(e);
                        }
                    }
                }
                self.finishing = true;
                step.changed = true;
            }
        }
        step
    }

    fn parse_lrzsz(&mut self, text: &str) -> bool {
        let mut changed = false;
        for line in text.split(['\r', '\n']) {
            changed |= self.parse_lrzsz_line(line);
        }
        changed
    }

    fn parse_lrzsz_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        for key in ["Sending: ", "Receiving: "] {
            if let Some(i) = line.find(key) {
                let name = line[i + key.len()..].trim().to_owned();
                if self.status.file.is_some() {
                    self.complete_current();
                    self.status.file_index += 1;
                }
                self.status.file = Some(name);
                self.status.bytes = 0;
                self.status.size = None;
                return true;
            }
        }
        for key in ["Bytes Sent:", "Bytes received:"] {
            if let Some(i) = line.find(key) {
                let rest = &line[i + key.len()..];
                let mut parts = rest.split('/');
                let done = parts.next().and_then(|s| s.trim().parse::<u64>().ok());
                let total = parts
                    .next()
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u64>().ok());
                if let Some(done) = done {
                    let delta = done.saturating_sub(self.status.bytes);
                    self.status.total_bytes += delta;
                    self.status.bytes = done;
                    self.status.size = total;
                    return true;
                }
            }
        }
        false
    }

    /// lrzsz: record the current file as completed.
    fn complete_current(&mut self) {
        let Some(name) = self.status.file.clone() else {
            return;
        };
        if self.status.files.iter().any(|f| f.name == name) {
            return;
        }
        let path = match (&self.dir, self.status.direction) {
            (Some(d), TransferDirection::Receive) => d.join(sanitize_name(&name)),
            _ => PathBuf::from(&name),
        };
        self.status.files.push(TransferredFile {
            name,
            path,
            bytes: self.status.size.unwrap_or(self.status.bytes),
        });
    }

    fn apply(&mut self, out: Output) -> Step {
        let mut changed = false;
        for ev in out.events {
            changed = true;
            match ev {
                Event::FileStarted { index, name, size } => {
                    self.status.file_index = index;
                    self.status.file = Some(name);
                    self.status.size = size;
                    self.status.bytes = 0;
                }
                Event::Progress { index, bytes, size } => {
                    self.status.file_index = index;
                    let delta = bytes.saturating_sub(self.status.bytes);
                    self.status.total_bytes += delta;
                    self.status.bytes = bytes;
                    if size.is_some() {
                        self.status.size = size;
                    }
                }
                Event::Retry(_) => self.status.retries += 1,
                Event::FileReceived(f) => self.save(f),
                Event::FileSent { index } => {
                    self.record_sent();
                    self.status.file_index = index;
                }
                Event::Finished => {
                    if self.status.state == TransferState::Running {
                        self.status.state = TransferState::Finished;
                    }
                    self.finishing = true;
                }
                Event::Failed(msg) => {
                    if self.status.state == TransferState::Running {
                        self.status.state = TransferState::Failed;
                        self.status.message = Some(msg);
                    }
                    self.finishing = true;
                }
            }
        }
        self.status.elapsed_ms = self.now_ms();
        Step {
            write: out.bytes,
            changed,
        }
    }

    /// Sending: add the current file to the completed list (once).
    fn record_sent(&mut self) {
        if self.status.direction != TransferDirection::Send {
            return;
        }
        let Some(name) = self.status.file.clone() else {
            return;
        };
        if self.status.files.len() > self.status.file_index {
            return;
        }
        self.status.files.push(TransferredFile {
            name,
            path: PathBuf::new(),
            bytes: self.status.bytes,
        });
    }

    fn save(&mut self, f: FileData) {
        let Some(dir) = &self.dir else { return };
        let name = sanitize_name(&f.name);
        let path = unique_path(dir, &name, self.overwrite);
        let bytes = f.data.len() as u64;
        match std::fs::write(&path, &f.data) {
            Ok(()) => {
                if let Some(m) = f.mtime {
                    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(m);
                    if let Ok(file) = std::fs::File::options().write(true).open(&path) {
                        let _ = file.set_modified(t);
                    }
                }
                self.status.files.push(TransferredFile {
                    name: f.name,
                    path,
                    bytes,
                });
            }
            Err(e) => {
                self.status.state = TransferState::Failed;
                self.status.message = Some(format!("cannot save {}: {e}", path.display()));
            }
        }
    }

    pub(crate) fn finish_status(&mut self) -> TransferStatus {
        self.status.elapsed_ms = self.now_ms();
        self.status.clone()
    }
}

fn spawn_lrzsz(req: &TransferRequest) -> Result<External, String> {
    let mut cmd = match req {
        TransferRequest::Send { files, .. } => {
            let mut c = tokio::process::Command::new("sz");
            c.args(["-b", "-vv", "--"]).args(files);
            c
        }
        TransferRequest::Receive { dir, overwrite, .. } => {
            let mut c = tokio::process::Command::new("rz");
            // Restricted: no absolute paths or `..` in received names.
            c.args(["-b", "-vv", "-R", if *overwrite { "-y" } else { "-E" }])
                .current_dir(dir);
            c
        }
    };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start lrzsz ({e}); install the lrzsz package for ZMODEM"))?;
    let (tx, events) = mpsc::channel(64);
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let mut stdin = child.stdin.take().expect("piped");
    let out_tx = tx.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16_384];
        while let Ok(n) = stdout.read(&mut buf).await {
            if n == 0
                || out_tx
                    .send(ExternalEvent::Output(Bytes::copy_from_slice(&buf[..n])))
                    .await
                    .is_err()
            {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0
                || tx
                    .send(ExternalEvent::Stderr(
                        String::from_utf8_lossy(&buf[..n]).into_owned(),
                    ))
                    .await
                    .is_err()
            {
                break;
            }
        }
    });
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<Bytes>();
    tokio::spawn(async move {
        while let Some(b) = stdin_rx.recv().await {
            if stdin.write_all(&b).await.is_err() {
                break;
            }
        }
    });
    Ok(External {
        child,
        stdin: stdin_tx,
        events,
        events_closed: false,
        done: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("C:\\x\\fw.bin"), "fw.bin");
        assert_eq!(sanitize_name(".bashrc"), "bashrc");
        assert_eq!(sanitize_name("a\nb"), "a_b");
        assert_eq!(sanitize_name(""), "received.bin");
        assert_eq!(sanitize_name("/"), "received.bin");
    }

    #[test]
    fn unique_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.bin", false), dir.path().join("a.bin"));
        std::fs::write(dir.path().join("a.bin"), b"x").unwrap();
        std::fs::write(dir.path().join("a (1).bin"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.bin", false),
            dir.path().join("a (2).bin")
        );
        assert_eq!(unique_path(dir.path(), "a.bin", true), dir.path().join("a.bin"));
        std::fs::write(dir.path().join("README"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "README", false),
            dir.path().join("README (1)")
        );
    }

    #[test]
    fn protocol_ids() {
        for p in TransferProtocol::ALL {
            assert_eq!(TransferProtocol::parse(p.id()), Some(p));
        }
        assert!(!TransferProtocol::Xmodem.batch());
        assert!(TransferProtocol::Zmodem.batch());
    }
}
