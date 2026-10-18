//! The MCP server: tools, resources and prompts over a [`SessionManager`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gtkterm_core::decoding::{DecodedFrame, DecoderMode, FrameSource};
use gtkterm_core::manager::SessionManager;
use gtkterm_core::ports::{PortInfo, list_ports};
use gtkterm_core::script::{Script, ScriptError, ScriptOptions};
use gtkterm_core::session::{LogOp, Session, SessionError, SessionStatus};
use gtkterm_core::sunshine::{ChartData, SunshineStats};
use gtkterm_core::transfer::{TransferProtocol, TransferRequest, TransferState, TransferStatus};
use gtkterm_protocols::decode::{Check, Protocol, modbus};
use gtkterm_protocols::downsample::lttb_indices;
use gtkterm_protocols::logfmt::{LogFormat, session_file_name};
use gtkterm_protocols::sunshine::ChartKind;
use regex::bytes::Regex;
use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    Implementation, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ProgressNotificationParam, PromptMessage, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Role, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, prompt, prompt_handler, prompt_router, tool, tool_handler,
    tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::params::{
    AutoBaudParams, BreakParams, ChartParams, ChartTypeParams, ClearChartParams, DecoderSetParams,
    ExpectParams, FileReceiveParams, FileSendParams, GetFramesParams, GetSeriesParams, LogStartParams,
    MacroRunParams, MacroSaveParams, ModbusFraming, ModbusRequestParams, OpenPortParams, PauseParams,
    PngParams, PresetApplyParams, ProfileParams, ProfileSaveParams, ReadParams, ReadResult,
    ScriptCheckParams, ScriptRunParams, SearchHit, SearchParams, SendParams, SessionArg, SignalHistoryParams,
    SignalParams,
};

/// Read caps protecting the agent's context window and the app.
const MAX_READ: usize = 1 << 20;
const MAX_WAIT: Duration = Duration::from_secs(60);
const MAX_EXPECT: Duration = Duration::from_secs(600);

/// Tools that change device state or files; hidden in read-only mode.
pub const WRITE_TOOLS: &[&str] = &[
    "script_run",
    "script_stop",
    "port_flush",
    "port_auto_baud",
    "file_send",
    "file_receive",
    "file_cancel",
    "decoder_set",
    "modbus_request",
    "macro_run",
    "macro_save",
    "preset_apply",
    "profile_save",
    "open_port",
    "close_port",
    "send",
    "expect",
    "send_break",
    "set_signals",
    "clear_buffer",
    "sunshine_set_chart_type",
    "sunshine_pause",
    "sunshine_clear",
    "log_start",
    "log_stop",
];

/// Server options.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Hide and refuse every tool in [`WRITE_TOOLS`].
    pub read_only: bool,
    /// Directory session logs are written to.
    pub log_dir: PathBuf,
    /// Received files are saved here; relative names for `file_send` resolve here.
    pub transfer_dir: PathBuf,
    /// Further directories `file_send` may read from (`--mcp-allow-dir`).
    pub allowed_dirs: Vec<PathBuf>,
    /// Scripts may use `exec` (`--mcp-allow-exec`).
    pub allow_exec: bool,
    /// Every tool call is appended to this JSONL file (`None`, the default: no audit log; the
    /// application uses [`default_audit_path`]).
    pub audit: Option<PathBuf>,
    /// Asked before write tools run (the GUI's confirmation dialog); `None`: no questions.
    pub approver: Option<crate::Approver>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            read_only: false,
            log_dir: default_log_dir(),
            transfer_dir: default_transfer_dir(),
            allowed_dirs: Vec::new(),
            allow_exec: false,
            audit: None,
            approver: None,
        }
    }
}

/// Default log directory: `$XDG_DATA_HOME/gtkterm-sunshine/logs`.
pub fn default_log_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map_or_else(|| PathBuf::from("."), |d| d.data_dir().to_owned())
        .join("gtkterm-sunshine")
        .join("logs")
}

/// Default audit log of agent tool calls: `$XDG_DATA_HOME/gtkterm-sunshine/mcp-audit.jsonl`.
pub fn default_audit_path() -> PathBuf {
    default_log_dir().with_file_name("mcp-audit.jsonl")
}

/// Default transfer directory: `$XDG_DATA_HOME/gtkterm-sunshine/transfers`.
pub fn default_transfer_dir() -> PathBuf {
    default_log_dir().with_file_name("transfers")
}

/// `file_send` / `file_receive` result.
#[derive(Debug, Serialize, JsonSchema)]
struct TransferResult {
    /// One-line summary.
    summary: String,
    /// Protocol.
    protocol: String,
    /// Files sent or received, with local paths for received files.
    files: Vec<serde_json::Value>,
    /// Bytes transferred.
    bytes: u64,
    /// Duration in ms.
    elapsed_ms: u64,
    /// Repeated blocks and timeouts.
    retries: u32,
    /// Note about the end of the transfer, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// GTKTerm MCP server. Cheap to clone; all clones share the session manager.
#[derive(Debug, Clone)]
pub struct GtkTermMcp {
    manager: Arc<SessionManager>,
    options: ServerOptions,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
    audit: Option<Arc<crate::guard::AuditLog>>,
    /// Clients the user allowed until the server stops, by reported name and version.
    /// Kept across server instances: in stateless HTTP mode every request gets a new one.
    approved: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// One entry of `list_ports`.
#[derive(Debug, Serialize, JsonSchema)]
struct PortEntry {
    /// Device path.
    path: String,
    /// usb_serial, usb_acm, uart, bluetooth or pty.
    class: String,
    /// Description with vendor/model/serial number.
    label: String,
}

/// `list_ports` result.
#[derive(Debug, Serialize, JsonSchema)]
struct PortList {
    /// Serial devices found on this machine.
    ports: Vec<PortEntry>,
    /// Existing sessions with their state.
    sessions: Vec<serde_json::Value>,
    /// Saved profile names usable with open_port.
    profiles: Vec<String>,
}

/// Generic acknowledgement.
#[derive(Debug, Serialize, JsonSchema)]
struct Done {
    /// What happened.
    message: String,
    /// Session status afterwards.
    status: serde_json::Value,
}

/// `send` result.
#[derive(Debug, Serialize, JsonSchema)]
struct SendResult {
    /// Bytes written to the device.
    bytes: usize,
    /// RX cursor at the time of sending: pass it to `read` or `expect` to see the reply.
    rx_cursor: u64,
}

/// `expect` result.
#[derive(Debug, Serialize, JsonSchema)]
struct ExpectResult {
    /// The matched text.
    matched: String,
    /// Capture groups 1..n (null for groups that did not participate).
    captures: Vec<Option<String>>,
    /// Received text between the start cursor and the match.
    before: String,
    /// Cursor just after the match.
    cursor: u64,
}

/// Summary of one chart.
#[derive(Debug, Serialize, JsonSchema)]
struct ChartSummary {
    /// Packet ID.
    id: String,
    /// line, bar, pie or scatter.
    kind: String,
    /// Packets received.
    packets: u64,
    /// Whether the chart is paused.
    paused: bool,
    /// Samples kept per series.
    capacity: usize,
    /// Series summaries.
    series: Vec<SeriesSummary>,
}

/// Summary of one series.
#[derive(Debug, Serialize, JsonSchema)]
struct SeriesSummary {
    /// Series name.
    name: String,
    /// Samples retained.
    samples: usize,
    /// Samples ever received.
    total: u64,
    /// Latest values.
    last: Option<Vec<f64>>,
    /// Minimum of the first value over retained samples.
    min: Option<f64>,
    /// Maximum of the first value over retained samples.
    max: Option<f64>,
    /// Mean of the first value over retained samples.
    mean: Option<f64>,
    /// "#RRGGBB" colour from the packet, if any.
    color: Option<String>,
}

/// `sunshine_list_series` result.
#[derive(Debug, Serialize, JsonSchema)]
struct ChartList {
    /// Charts in creation order.
    charts: Vec<ChartSummary>,
    /// Packet counters.
    stats: StatsOut,
    /// Last rejected packet line with the reason.
    last_error: Option<String>,
}

/// Packet counters.
#[derive(Debug, Serialize, JsonSchema)]
struct StatsOut {
    /// Valid packets.
    packets: u64,
    /// Rejected lines.
    errors: u64,
    /// Overlong lines discarded.
    overflows: u64,
}

impl From<SunshineStats> for StatsOut {
    fn from(s: SunshineStats) -> Self {
        Self {
            packets: s.packets,
            errors: s.errors,
            overflows: s.overflows,
        }
    }
}

/// Points of one series.
#[derive(Debug, Serialize, JsonSchema)]
struct SeriesPoints {
    /// Series name.
    name: String,
    /// Samples before downsampling.
    available: usize,
    /// Per-series sample numbers.
    index: Vec<u64>,
    /// Seconds since the chart's first packet.
    t: Vec<f64>,
    /// Values per sample (one value for plain series, several for X/Y data).
    values: Vec<Vec<f64>>,
}

/// `sunshine_get_series` result.
#[derive(Debug, Serialize, JsonSchema)]
struct SeriesData {
    /// Packet ID.
    id: String,
    /// Chart type.
    kind: String,
    /// Series.
    series: Vec<SeriesPoints>,
}

/// Text result.
#[derive(Debug, Serialize, JsonSchema)]
struct TextOut {
    /// The content.
    text: String,
}

/// `search_buffer` result.
#[derive(Debug, Serialize, JsonSchema)]
struct SearchOut {
    /// Matches, oldest first.
    matches: Vec<SearchHit>,
    /// True if more matches exist than were returned.
    truncated: bool,
}

/// Prompt argument with a free-form focus.
#[derive(Debug, Deserialize, JsonSchema)]
struct FocusArgs {
    /// Optional focus, e.g. a device name, a packet ID or a symptom.
    #[serde(default)]
    focus: Option<String>,
}

fn kind_name(k: ChartKind) -> String {
    format!("{k:?}").to_lowercase()
}

/// A decoded frame as returned to agents.
#[derive(Debug, Serialize, JsonSchema)]
struct FrameOut {
    /// Sequence number.
    seq: u64,
    /// Completion time (RFC 3339).
    time: String,
    /// rx, tx or lines (modem line change).
    source: String,
    /// Protocol id.
    protocol: String,
    /// One-line decoding.
    summary: String,
    /// none, ok or failed (with received/computed).
    check: serde_json::Value,
    /// Decoded fields.
    fields: serde_json::Map<String, serde_json::Value>,
    /// Frame bytes as hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<String>,
}

fn frame_out(f: &DecodedFrame, bytes: bool) -> FrameOut {
    FrameOut {
        seq: f.seq,
        time: f.time.to_string(),
        source: serde_json::to_value(f.source)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        protocol: f.frame.protocol.id().to_owned(),
        summary: f.frame.summary.clone(),
        check: serde_json::to_value(&f.frame.check).unwrap_or_default(),
        fields: f
            .frame
            .fields
            .iter()
            .map(|x| (x.name.clone(), serde_json::Value::String(x.value.clone())))
            .collect(),
        bytes: bytes.then(|| gtkterm_protocols::hex::to_hex(&f.frame.bytes)),
    }
}

/// `decoder_get_frames` result.
#[derive(Debug, Serialize, JsonSchema)]
struct FramesResult {
    /// Frames, oldest first.
    frames: Vec<FrameOut>,
    /// Pass as `since` to get only newer frames.
    next_seq: u64,
    /// Frames that were requested but already dropped from the ring.
    dropped: u64,
    /// Decoder state.
    decoder: serde_json::Value,
}

/// `modbus_request` result.
#[derive(Debug, Serialize, JsonSchema)]
struct ModbusResult {
    /// Framing used.
    framing: ModbusFraming,
    /// Bytes sent, as hex.
    request: String,
    /// Bytes of the response frame, as hex (absent for broadcasts).
    #[serde(skip_serializing_if = "Option::is_none")]
    response_bytes: Option<String>,
    /// Decoded response summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    /// Parsed response: bits, registers, written or exception.
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<serde_json::Value>,
    /// Round trip in ms.
    elapsed_ms: u64,
    /// The session's decoder was switched to this framing.
    decoder_changed: bool,
}

fn framing_protocol(f: ModbusFraming) -> Protocol {
    match f {
        ModbusFraming::Rtu => Protocol::ModbusRtu,
        ModbusFraming::Ascii => Protocol::ModbusAscii,
        ModbusFraming::Tcp => Protocol::ModbusTcp,
    }
}

fn status_json(s: &SessionStatus) -> serde_json::Value {
    serde_json::to_value(s).unwrap_or_default()
}

fn summarize(c: &ChartData) -> ChartSummary {
    ChartSummary {
        id: c.id.clone(),
        kind: kind_name(c.kind),
        packets: c.packets,
        paused: c.paused,
        capacity: c.capacity,
        series: c
            .series
            .iter()
            .map(|s| {
                let firsts: Vec<f64> = s
                    .samples
                    .iter()
                    .filter_map(|x| x.values.first().copied())
                    .collect();
                let range = s.y_range();
                SeriesSummary {
                    name: s.name.clone(),
                    samples: s.samples.len(),
                    total: s.total,
                    last: s.last().map(|x| x.values.clone()),
                    min: range.map(|r| r.0),
                    max: range.map(|r| r.1),
                    mean: (!firsts.is_empty()).then(|| firsts.iter().sum::<f64>() / firsts.len() as f64),
                    color: s.color.map(|c| format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)),
                }
            })
            .collect(),
    }
}

impl GtkTermMcp {
    /// New server over `manager`.
    pub fn new(manager: Arc<SessionManager>, options: ServerOptions) -> Self {
        let mut tool_router = Self::tool_router();
        if options.read_only {
            for name in WRITE_TOOLS {
                tool_router.remove_route(name);
            }
        }
        Self {
            manager,
            audit: options
                .audit
                .clone()
                .map(|p| Arc::new(crate::guard::AuditLog::new(p))),
            options,
            tool_router,
            prompt_router: Self::prompt_router(),
            approved: Arc::default(),
        }
    }

    /// Path of the audit log, if one is written.
    pub fn audit_path(&self) -> Option<&Path> {
        self.audit.as_deref().map(crate::guard::AuditLog::path)
    }

    /// Ask the approver (if any) whether `tool` may run. `Err` carries the refusal message.
    async fn approve(
        &self,
        tool: &str,
        args: Option<&serde_json::Map<String, serde_json::Value>>,
        client: Option<&str>,
    ) -> Result<(), String> {
        let Some(approver) = &self.options.approver else {
            return Ok(());
        };
        let key = client.unwrap_or("unknown client").to_owned();
        let allowed = || {
            self.approved
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        if !WRITE_TOOLS.contains(&tool) || allowed().contains(&key) {
            return Ok(());
        }
        let request = crate::ApprovalRequest {
            tool: tool.to_owned(),
            arguments: crate::guard::compact_arguments(args, 400),
            client: client.map(str::to_owned),
        };
        match approver.ask(request).await {
            crate::Approval::Always => {
                allowed().insert(key);
                Ok(())
            }
            crate::Approval::Once => Ok(()),
            crate::Approval::Deny => Err(format!(
                "The user denied {tool}. Ask them before trying again, or use read-only tools."
            )),
        }
    }

    /// The session manager.
    pub fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }

    fn session(&self, id: Option<u32>) -> Result<Session, String> {
        match id {
            Some(id) => self.manager.get(id).ok_or_else(|| format!("no session {id}")),
            None => Ok(self.manager.first_or_create()),
        }
    }

    fn done(session: &Session, message: impl Into<String>) -> Json<Done> {
        Json(Done {
            message: message.into(),
            status: status_json(&session.status()),
        })
    }

    fn chart<R>(session: &Session, id: &str, f: impl FnOnce(&ChartData) -> R) -> Result<R, String> {
        session.with_sunshine(|s| s.chart(id).map(f)).ok_or_else(|| {
            let ids = session.with_sunshine(|s| s.charts().iter().map(|c| c.id.clone()).collect::<Vec<_>>());
            format!("no chart {id:?}; available: {ids:?}")
        })
    }

    /// A file `file_send` may read: inside the transfer directory or an allowed directory, after
    /// resolving symlinks.
    fn readable_path(&self, p: &str) -> Result<PathBuf, String> {
        let path = Path::new(p);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            self.options.transfer_dir.join(path)
        };
        let real = path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let allowed = std::iter::once(&self.options.transfer_dir)
            .chain(&self.options.allowed_dirs)
            .filter_map(|d| d.canonicalize().ok())
            .any(|d| real.starts_with(&d));
        if !allowed {
            return Err(format!(
                "{} is outside the transfer directory {} (allow more with --mcp-allow-dir)",
                real.display(),
                self.options.transfer_dir.display()
            ));
        }
        Ok(real)
    }

    /// Directories file-using tools and scripts may touch.
    fn file_dirs(&self) -> Vec<PathBuf> {
        let mut d = vec![self.options.transfer_dir.clone()];
        d.extend(self.options.allowed_dirs.iter().cloned());
        d
    }

    async fn run_transfer(
        &self,
        session: &Session,
        req: TransferRequest,
        total: Option<u64>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<TransferResult>, String> {
        let token = ctx.meta.get_progress_token();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TransferStatus>();
        let peer = ctx.peer.clone();
        let forward = tokio::spawn(async move {
            while let Some(st) = rx.recv().await {
                if let Some(t) = &token {
                    let mut p = ProgressNotificationParam::new(t.clone(), st.total_bytes as f64)
                        .with_message(st.summary());
                    if let Some(total) = total {
                        p = p.with_total(total as f64);
                    }
                    let _ = peer.notify_progress(p).await;
                }
            }
        });
        let fut = Box::pin(session.transfer(req, move |st| {
            let _ = tx.send(st.clone());
        }));
        let mut fut = fut;
        let st = tokio::select! {
            r = &mut fut => r,
            () = ctx.ct.cancelled() => {
                session.cancel_transfer().await;
                (&mut fut).await
            }
        };
        drop(fut);
        let _ = forward.await;
        let st = st.map_err(err)?;
        if st.state != TransferState::Finished {
            return Err(st.summary());
        }
        Ok(Json(TransferResult {
            summary: st.summary(),
            protocol: st.protocol.id().to_owned(),
            files: st
                .files
                .iter()
                .map(|f| serde_json::to_value(f).unwrap_or_default())
                .collect(),
            bytes: st.total_bytes,
            elapsed_ms: st.elapsed_ms,
            retries: st.retries,
            message: st.message,
        }))
    }

    fn log_path(
        &self,
        session: &Session,
        format: LogFormat,
        file_name: Option<&str>,
    ) -> Result<PathBuf, String> {
        let name = if let Some(n) = file_name {
            if Path::new(n).components().count() != 1 || n.starts_with('.') || n.contains(['/', '\\']) {
                return Err(format!(
                    "{n:?} must be a plain file name inside the log directory"
                ));
            }
            n.to_owned()
        } else {
            let st = session.status();
            let stamp = jiff::Zoned::now().strftime("%Y%m%d_%H%M%S").to_string();
            session_file_name(&stamp, &st.description, &st.name, format)
        };
        Ok(self.options.log_dir.join(name))
    }
}

#[tool_router]
impl GtkTermMcp {
    /// List serial ports, sessions and saved profiles.
    #[tool(annotations(title = "List ports", read_only_hint = true))]
    async fn list_ports(&self) -> Result<Json<PortList>, String> {
        let ports = tokio::task::spawn_blocking(list_ports).await.map_err(err)?;
        Ok(Json(PortList {
            ports: ports
                .iter()
                .map(|p: &PortInfo| PortEntry {
                    path: p.path.clone(),
                    class: serde_json::to_value(p.class)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    label: p.label(),
                })
                .collect(),
            sessions: self
                .manager
                .list()
                .iter()
                .map(|s| status_json(&s.status()))
                .collect(),
            profiles: self.manager.with_config(|c| c.profiles.keys().cloned().collect()),
        }))
    }

    /// Open a serial device, TCP or UDP connection (reopens if already open).
    #[tool(annotations(title = "Open port", destructive_hint = false, open_world_hint = true))]
    async fn open_port(&self, Parameters(p): Parameters<OpenPortParams>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let connection = match (p.connection, p.profile) {
            (Some(c), _) => c.into_connection()?,
            (None, Some(profile)) => {
                let prof = self
                    .manager
                    .with_config(|c| c.profile(&profile).cloned())
                    .map_err(err)?;
                session.set_terminal(prof.terminal);
                session.apply_decoder_config(prof.decoder);
                prof.connection
            }
            (None, None) => return Err("give either `connection` or `profile`".into()),
        };
        session.open(connection).await.map_err(err)?;
        Ok(Self::done(
            &session,
            format!("opened {}", session.status().description),
        ))
    }

    /// Close the connection of a session.
    #[tool(annotations(title = "Close port", idempotent_hint = true))]
    async fn close_port(&self, Parameters(p): Parameters<SessionArg>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        session.close().await.map_err(err)?;
        Ok(Self::done(&session, "closed"))
    }

    /// Connection state, endpoint, byte counters, modem lines and logging state.
    #[tool(annotations(title = "Get status", read_only_hint = true))]
    async fn get_status(
        &self,
        Parameters(p): Parameters<SessionArg>,
    ) -> Result<Json<serde_json::Value>, String> {
        Ok(Json(status_json(&self.session(p.session)?.status())))
    }

    /// Send data to the device. Returns an rx_cursor to read the reply from.
    #[tool(annotations(title = "Send data", open_world_hint = true))]
    async fn send(&self, Parameters(p): Parameters<SendParams>) -> Result<Json<SendResult>, String> {
        let session = self.session(p.session)?;
        let mut data = p.encoding.decode(&p.data)?;
        data.extend_from_slice(p.line_ending.bytes());
        let rx_cursor = session.rx_cursor();
        let bytes = data.len();
        session.send(data).await.map_err(err)?;
        Ok(Json(SendResult { bytes, rx_cursor }))
    }

    /// Read received data from a cursor; optionally wait for data to arrive.
    #[tool(annotations(title = "Read data", read_only_hint = true))]
    async fn read(
        &self,
        Parameters(p): Parameters<ReadParams>,
        ct: CancellationToken,
    ) -> Result<Json<ReadResult>, String> {
        let session = self.session(p.session)?;
        let cursor = p.cursor.unwrap_or_else(|| session.rx_cursor());
        let max = p.max_bytes.unwrap_or(4096).clamp(1, MAX_READ);
        let wait = Duration::from_millis(p.timeout_ms.unwrap_or(0)).min(MAX_WAIT);
        let mut events = session.subscribe();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let r = session.read_rx(cursor, max);
            if !r.data.is_empty() || tokio::time::Instant::now() >= deadline {
                return Ok(Json(ReadResult {
                    bytes: r.data.len(),
                    data: p.encoding.encode(&r.data),
                    next_cursor: r.next_cursor,
                    lost: r.lost,
                }));
            }
            tokio::select! {
                () = ct.cancelled() => return Err("cancelled".into()),
                _ = tokio::time::timeout_at(deadline, events.recv()) => {}
            }
        }
    }

    /// Optionally send data, then wait until received data matches a regular expression.
    #[tool(annotations(title = "Expect", open_world_hint = true))]
    async fn expect(
        &self,
        Parameters(p): Parameters<ExpectParams>,
        ct: CancellationToken,
    ) -> Result<Json<ExpectResult>, String> {
        let session = self.session(p.session)?;
        let re = Regex::new(&p.pattern).map_err(|e| format!("invalid pattern: {e}"))?;
        let cursor = p.cursor.unwrap_or_else(|| session.rx_cursor());
        if let Some(send) = &p.send {
            let mut data = p.encoding.decode(send)?;
            data.extend_from_slice(p.line_ending.bytes());
            session.send(data).await.map_err(err)?;
        }
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(5_000)).min(MAX_EXPECT);
        let result = tokio::select! {
            () = ct.cancelled() => return Err("cancelled".into()),
            r = session.expect(&re, cursor, timeout) => r,
        };
        match result {
            Ok(m) => Ok(Json(ExpectResult {
                matched: m.matched,
                captures: m.captures,
                before: m.before,
                cursor: m.cursor,
            })),
            Err(SessionError::Timeout(_)) => {
                let got = session.read_rx(cursor, 2048);
                Err(format!(
                    "timed out after {} ms without a match for {:?}; received so far: {:?}",
                    timeout.as_millis(),
                    p.pattern,
                    String::from_utf8_lossy(&got.data)
                ))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Send a serial break condition.
    #[tool(annotations(title = "Send break"))]
    async fn send_break(&self, Parameters(p): Parameters<BreakParams>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let default = session
            .connection()
            .and_then(|c| c.serial().map(|s| u64::from(s.break_ms)))
            .unwrap_or(250);
        let ms = p.duration_ms.unwrap_or(default).clamp(1, 5_000);
        session.send_break(Duration::from_millis(ms)).await.map_err(err)?;
        Ok(Self::done(&session, format!("sent {ms} ms break")))
    }

    /// Set the DTR and/or RTS output lines of a serial port.
    #[tool(annotations(title = "Set signals", idempotent_hint = true))]
    async fn set_signals(&self, Parameters(p): Parameters<SignalParams>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        if let Some(dtr) = p.dtr {
            session.set_dtr(dtr).await.map_err(err)?;
        }
        if let Some(rts) = p.rts {
            session.set_rts(rts).await.map_err(err)?;
        }
        Ok(Self::done(&session, "signals updated"))
    }

    /// Read the modem lines (DTR, RTS, CTS, DSR, DCD, RI) of a serial port.
    #[tool(annotations(title = "Get signals", read_only_hint = true))]
    async fn get_signals(
        &self,
        Parameters(p): Parameters<SessionArg>,
    ) -> Result<Json<serde_json::Value>, String> {
        let st = self.session(p.session)?.status();
        st.modem
            .map(|m| Json(serde_json::to_value(m).unwrap_or_default()))
            .ok_or_else(|| {
                "no modem lines available (port closed, not serial, or a pseudo terminal)".to_owned()
            })
    }

    /// Clear the received history and all Sunshine charts of a session.
    #[tool(annotations(title = "Clear buffer", destructive_hint = true))]
    async fn clear_buffer(&self, Parameters(p): Parameters<SessionArg>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        session.clear();
        Ok(Self::done(&session, "history cleared"))
    }

    /// Search the retained received data with a regular expression.
    #[tool(annotations(title = "Search buffer", read_only_hint = true))]
    async fn search_buffer(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<Json<SearchOut>, String> {
        let session = self.session(p.session)?;
        let re = Regex::new(&p.pattern).map_err(|e| format!("invalid pattern: {e}"))?;
        let max = p.max_matches.unwrap_or(50).clamp(1, 1000);
        let all = session.read_rx(0, usize::MAX);
        let base = all.next_cursor - all.data.len() as u64;
        let mut matches = Vec::new();
        let mut truncated = false;
        for m in re.find_iter(&all.data) {
            if matches.len() == max {
                truncated = true;
                break;
            }
            let line_start = all.data[..m.start()]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |i| i + 1);
            let line_end = all.data[m.end()..]
                .iter()
                .position(|&b| b == b'\n' || b == b'\r')
                .map_or(all.data.len(), |i| m.end() + i);
            matches.push(SearchHit {
                offset: base + m.start() as u64,
                text: String::from_utf8_lossy(m.as_bytes()).into_owned(),
                line: String::from_utf8_lossy(&all.data[line_start..line_end])
                    .chars()
                    .take(300)
                    .collect(),
            });
        }
        Ok(Json(SearchOut { matches, truncated }))
    }

    /// List Sunshine charts with per-series statistics (min/max/mean/last).
    #[tool(annotations(title = "List Sunshine series", read_only_hint = true))]
    async fn sunshine_list_series(
        &self,
        Parameters(p): Parameters<SessionArg>,
    ) -> Result<Json<ChartList>, String> {
        let session = self.session(p.session)?;
        Ok(Json(session.with_sunshine(|s| ChartList {
            charts: s.charts().iter().map(summarize).collect(),
            stats: s.stats().into(),
            last_error: s.last_error().map(str::to_owned),
        })))
    }

    /// Get the samples of a Sunshine chart, downsampled for the context window.
    #[tool(annotations(title = "Get Sunshine series", read_only_hint = true))]
    async fn sunshine_get_series(
        &self,
        Parameters(p): Parameters<GetSeriesParams>,
    ) -> Result<Json<SeriesData>, String> {
        let session = self.session(p.session)?;
        let max_points = p.max_points.unwrap_or(200).clamp(2, 5000);
        Self::chart(&session, &p.id, |c| SeriesData {
            id: c.id.clone(),
            kind: kind_name(c.kind),
            series: c
                .series
                .iter()
                .filter(|s| p.series.as_ref().is_none_or(|names| names.contains(&s.name)))
                .map(|s| {
                    let skip = p.last_n.map_or(0, |n| s.samples.len().saturating_sub(n));
                    let samples: Vec<_> = s.samples.iter().skip(skip).collect();
                    let first_ms = c.first_ts.as_millisecond();
                    let xy: Vec<(f64, f64)> = samples
                        .iter()
                        .map(|x| (x.index as f64, x.values.first().copied().unwrap_or(0.0)))
                        .collect();
                    let keep = lttb_indices(&xy, max_points);
                    SeriesPoints {
                        name: s.name.clone(),
                        available: samples.len(),
                        index: keep.iter().map(|&i| samples[i].index).collect(),
                        t: keep
                            .iter()
                            .map(|&i| (samples[i].ts.as_millisecond() - first_ms) as f64 / 1000.0)
                            .collect(),
                        values: keep.iter().map(|&i| samples[i].values.clone()).collect(),
                    }
                })
                .collect(),
        })
        .map(Json)
    }

    /// Choose the chart type (line, bar, pie, scatter) for a packet ID.
    #[tool(annotations(title = "Set chart type", idempotent_hint = true))]
    async fn sunshine_set_chart_type(
        &self,
        Parameters(p): Parameters<ChartTypeParams>,
    ) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let kind = ChartKind::ALL
            .into_iter()
            .find(|k| kind_name(*k) == p.kind.to_lowercase())
            .ok_or_else(|| format!("unknown chart type {:?}; use line, bar, pie or scatter", p.kind))?;
        session.with_sunshine_mut(|s| s.set_kind(&p.id, kind));
        session.notify_sunshine(gtkterm_core::sunshine::SunshineUpdate::ChartUpdated(p.id.clone()));
        Ok(Self::done(
            &session,
            format!("chart {} is now {}", p.id, kind_name(kind)),
        ))
    }

    /// Export a Sunshine chart as CSV (timestamp, elapsed seconds, series, index, values).
    #[tool(annotations(title = "Export CSV", read_only_hint = true))]
    async fn sunshine_export_csv(
        &self,
        Parameters(p): Parameters<ChartParams>,
    ) -> Result<Json<TextOut>, String> {
        let session = self.session(p.session)?;
        Self::chart(&session, &p.id, |c| Json(TextOut { text: c.to_csv() }))
    }

    /// Render a Sunshine chart as a PNG image, to look at the data the way the user sees it.
    #[tool(annotations(title = "Chart image", read_only_hint = true))]
    async fn sunshine_export_png(
        &self,
        Parameters(p): Parameters<PngParams>,
    ) -> Result<rmcp::model::CallToolResult, String> {
        let session = self.session(p.session)?;
        let chart = Self::chart(&session, &p.id, Clone::clone)?;
        #[cfg(feature = "png")]
        {
            use base64::Engine as _;
            use gtkterm_charts::render::{Style, render_png};
            let style = if p.dark { Style::dark() } else { Style::light() };
            let width = p.width.unwrap_or(800).clamp(200, 2000);
            let height = p.height.unwrap_or(450).clamp(150, 1500);
            let png = tokio::task::spawn_blocking(move || {
                render_png(
                    &chart,
                    &gtkterm_charts::model::ViewState::default(),
                    &style,
                    width,
                    height,
                )
                .map_err(err)
            })
            .await
            .map_err(err)??;
            let data = base64::engine::general_purpose::STANDARD.encode(png);
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::ContentBlock::image(data, "image/png"),
            ]))
        }
        #[cfg(not(feature = "png"))]
        {
            let _ = (chart, p.width, p.height, p.dark);
            Err("this build has no chart rendering (feature `png`); use sunshine_get_series".into())
        }
    }

    /// List macros with their shortcuts and steps (in the editor syntax accepted by macro_save).
    #[tool(annotations(title = "List macros", read_only_hint = true))]
    async fn macro_list(&self) -> Result<Json<serde_json::Value>, String> {
        let macros: Vec<serde_json::Value> = self.manager.with_macros(|s| {
            s.list()
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.name,
                        "shortcut": m.shortcut,
                        "text": gtkterm_core::macros::format_text(&m.commands),
                    })
                })
                .collect()
        });
        Ok(Json(serde_json::json!({ "macros": macros })))
    }

    /// Run a macro in a session and wait until it finishes (expect steps fail on timeout).
    #[tool(annotations(title = "Run macro", open_world_hint = true))]
    async fn macro_run(
        &self,
        Parameters(p): Parameters<MacroRunParams>,
        ct: CancellationToken,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        let id = session.id();
        let run = self.manager.run_macro(&session, &p.name);
        tokio::pin!(run);
        let result = tokio::select! {
            r = &mut run => r,
            () = ct.cancelled() => {
                self.manager.stop_macro(id);
                run.await
            }
        };
        match result {
            Ok(report) => Ok(Json(serde_json::to_value(report).unwrap_or_default())),
            Err(gtkterm_core::macros::MacroError::Step {
                step,
                message,
                report,
            }) => Err(format!(
                "step {step} failed: {message}; completed steps: {}; received: {:?}",
                serde_json::to_string(&report.steps).unwrap_or_default(),
                report.received
            )),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Create or replace a macro from editor text.
    #[tool(annotations(title = "Save macro", idempotent_hint = true))]
    async fn macro_save(
        &self,
        Parameters(p): Parameters<MacroSaveParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let commands = gtkterm_core::macros::parse_text(&p.text).map_err(err)?;
        let m = gtkterm_core::macros::Macro {
            name: p.name.trim().to_owned(),
            shortcut: p.shortcut.filter(|s| !s.trim().is_empty()),
            commands,
        };
        let name = m.name.clone();
        let steps = m.commands.len();
        self.manager
            .update_macros(|s| s.upsert(Some(&name), m))
            .map_err(err)?;
        Ok(Json(serde_json::json!({ "saved": name, "steps": steps })))
    }

    /// List serial presets (built-in and user defined).
    #[tool(annotations(title = "List presets", read_only_hint = true))]
    async fn preset_list(&self) -> Result<Json<serde_json::Value>, String> {
        let presets: Vec<serde_json::Value> = self.manager.with_config(|c| {
            gtkterm_core::presets::all_presets(&c.presets)
                .into_iter()
                .map(|(name, p, builtin)| {
                    serde_json::json!({
                        "name": name,
                        "settings": p.summary(),
                        "flow_control": p.flow_control,
                        "description": p.description,
                        "builtin": builtin,
                    })
                })
                .collect()
        });
        Ok(Json(serde_json::json!({ "presets": presets })))
    }

    /// Apply a preset's line settings to the session's serial port (reopens it).
    #[tool(annotations(title = "Apply preset"))]
    async fn preset_apply(&self, Parameters(p): Parameters<PresetApplyParams>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let preset = self
            .manager
            .with_config(|c| gtkterm_core::presets::find_preset(&c.presets, &p.name))
            .ok_or_else(|| format!("no preset {:?}; see preset_list", p.name))?;
        let mut conn = session
            .connection()
            .ok_or("the session has no connection yet; open a serial port first")?;
        let gtkterm_core::config::Connection::Serial(cfg) = &mut conn else {
            return Err("presets apply to serial ports only".into());
        };
        preset.apply(cfg);
        session.open(conn).await.map_err(err)?;
        Ok(Self::done(
            &session,
            format!("applied {} ({})", p.name, preset.summary()),
        ))
    }

    /// List saved profiles.
    #[tool(annotations(title = "List profiles", read_only_hint = true))]
    async fn profile_list(&self) -> Result<Json<serde_json::Value>, String> {
        let profiles: Vec<serde_json::Value> = self.manager.with_config(|c| {
            c.profiles
                .iter()
                .map(|(n, p)| serde_json::json!({ "name": n, "endpoint": p.connection.summary() }))
                .collect()
        });
        Ok(Json(serde_json::json!({ "profiles": profiles })))
    }

    /// Get a profile's settings (connection and terminal).
    #[tool(annotations(title = "Get profile", read_only_hint = true))]
    async fn profile_get(
        &self,
        Parameters(p): Parameters<ProfileParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let profile = self
            .manager
            .with_config(|c| c.profile(&p.name).cloned())
            .map_err(err)?;
        Ok(Json(serde_json::to_value(profile).map_err(err)?))
    }

    /// Save the session's current connection and terminal settings as a profile.
    #[tool(annotations(title = "Save profile", idempotent_hint = true))]
    async fn profile_save(
        &self,
        Parameters(p): Parameters<ProfileSaveParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        let connection = session
            .connection()
            .ok_or("the session has no connection settings yet")?;
        let profile = gtkterm_core::config::Profile {
            connection,
            terminal: session.terminal(),
            decoder: session.decoder_config(),
        };
        profile.validate().map_err(err)?;
        self.manager
            .with_config_mut(|c| c.profile_mut(&p.name).map(|slot| *slot = profile))
            .map_err(err)?;
        self.manager.save_config().map_err(err)?;
        Ok(Json(serde_json::json!({ "saved": p.name })))
    }

    /// Select the protocol decoder of a session. Transmitted data is decoded as requests and received
    /// data as responses; read the frames with decoder_get_frames.
    #[tool(annotations(title = "Set decoder", idempotent_hint = true))]
    async fn decoder_set(
        &self,
        Parameters(p): Parameters<DecoderSetParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        let mode: DecoderMode = p.mode.parse()?;
        if p.clear {
            session.clear_frames();
        }
        let status = session.set_decoder(mode, p.signals);
        Ok(Json(serde_json::to_value(status).map_err(err)?))
    }

    /// Decoded protocol frames (Modbus, NMEA, AT, CAN, EtherCAT, ...) with checksum results.
    /// Poll with `since` = the previous `next_seq` to get only new frames.
    #[tool(annotations(title = "Get decoded frames", read_only_hint = true))]
    async fn decoder_get_frames(
        &self,
        Parameters(p): Parameters<GetFramesParams>,
        ct: CancellationToken,
    ) -> Result<Json<FramesResult>, String> {
        let session = self.session(p.session)?;
        let max = p.max.unwrap_or(100).clamp(1, 1000);
        let since = p
            .since
            .unwrap_or_else(|| session.decoder_status().next_seq.saturating_sub(max as u64));
        let wait = Duration::from_millis(p.timeout_ms.unwrap_or(0)).min(MAX_WAIT);
        if !wait.is_zero() && session.frames_since(since, 1).frames.is_empty() {
            tokio::select! {
                () = ct.cancelled() => return Err("cancelled".into()),
                _ = session.expect_frame(since, wait, |_| true) => {}
            }
        }
        let read = session.frames_since(since, max);
        let bytes = p.include_bytes.unwrap_or(true);
        Ok(Json(FramesResult {
            frames: read.frames.iter().map(|f| frame_out(f, bytes)).collect(),
            next_seq: read.next_seq,
            dropped: read.dropped,
            decoder: serde_json::to_value(session.decoder_status()).map_err(err)?,
        }))
    }

    /// Send a Modbus request (RTU, ASCII or TCP framing built with CRC/LRC/MBAP) and wait for the
    /// decoded response. Switches the session's decoder to the framing's protocol if needed.
    #[tool(annotations(title = "Modbus request", open_world_hint = true))]
    async fn modbus_request(
        &self,
        Parameters(p): Parameters<ModbusRequestParams>,
        ct: CancellationToken,
    ) -> Result<Json<ModbusResult>, String> {
        let session = self.session(p.session)?;
        if p.unit > 247 {
            return Err(format!("unit {} is out of range 0-247", p.unit));
        }
        let status = session.decoder_status();
        let framing = p.framing.unwrap_or(match status.protocol {
            Some(Protocol::ModbusAscii) => ModbusFraming::Ascii,
            Some(Protocol::ModbusTcp) => ModbusFraming::Tcp,
            _ => ModbusFraming::Rtu,
        });
        let protocol = framing_protocol(framing);
        let decoder_changed = status.protocol != Some(protocol);
        if decoder_changed {
            session.set_decoder(DecoderMode::Protocol(protocol), status.signals);
        }
        let request: modbus::Request = p.request.into();
        let pdu = request.pdu()?;
        let transaction = u16::try_from(session.decoder_status().next_seq & 0xFFFF).unwrap_or(0);
        let bytes = match framing {
            ModbusFraming::Rtu => modbus::rtu_frame(p.unit, &pdu),
            ModbusFraming::Ascii => modbus::ascii_frame(p.unit, &pdu),
            ModbusFraming::Tcp => modbus::tcp_frame(transaction, p.unit, &pdu),
        };
        let since = session.decoder_status().next_seq;
        let start = std::time::Instant::now();
        session.send(bytes.clone()).await.map_err(err)?;
        let mut out = ModbusResult {
            framing,
            request: gtkterm_protocols::hex::to_hex(&bytes),
            response_bytes: None,
            summary: None,
            response: None,
            elapsed_ms: 0,
            decoder_changed,
        };
        if p.unit == 0 {
            return Ok(Json(out));
        }
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(1000)).min(MAX_WAIT);
        let matches = |f: &DecodedFrame| {
            f.source == FrameSource::Rx
                && f.frame.protocol == protocol
                && (matches!(f.frame.check, Check::Failed { .. })
                    || modbus::application_data(protocol, &f.frame.bytes).is_some_and(|(unit, pdu, tid)| {
                        unit == p.unit
                            && (framing != ModbusFraming::Tcp || tid == Some(transaction))
                            && !matches!(modbus::parse_response(&request, &pdu), Ok(None))
                    }))
        };
        let result = tokio::select! {
            () = ct.cancelled() => return Err("cancelled".into()),
            r = session.expect_frame(since, timeout, matches) => r,
        };
        let frame = match result {
            Ok(f) => f,
            Err(SessionError::Timeout(_)) => {
                let seen: Vec<String> = session
                    .frames_since(since, 20)
                    .frames
                    .iter()
                    .filter(|f| f.source != FrameSource::Tx)
                    .map(|f| f.frame.summary.clone())
                    .collect();
                return Err(format!(
                    "no response from unit {} within {} ms{}",
                    p.unit,
                    timeout.as_millis(),
                    if seen.is_empty() {
                        String::new()
                    } else {
                        format!("; other frames received: {}", seen.join(" | "))
                    }
                ));
            }
            Err(e) => return Err(e.to_string()),
        };
        out.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        out.response_bytes = Some(gtkterm_protocols::hex::to_hex(&frame.frame.bytes));
        out.summary = Some(frame.frame.summary.clone());
        if let Check::Failed { received, computed } = &frame.frame.check {
            return Err(format!(
                "response failed its check (received {received}, computed {computed}): {}",
                frame.frame.summary
            ));
        }
        let (_, pdu, _) =
            modbus::application_data(protocol, &frame.frame.bytes).ok_or("malformed response")?;
        let response = modbus::parse_response(&request, &pdu)?.ok_or("unexpected response")?;
        out.response = Some(serde_json::to_value(response).map_err(err)?);
        Ok(Json(out))
    }

    /// Upload files to the device with XMODEM, XMODEM-1K, YMODEM, YMODEM-G or ZMODEM. Start the
    /// receiver on the device first. Sends progress notifications; cancelling the call aborts.
    #[tool(annotations(title = "Send file", open_world_hint = true))]
    async fn file_send(
        &self,
        Parameters(p): Parameters<FileSendParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<TransferResult>, String> {
        let session = self.session(p.session)?;
        let protocol: TransferProtocol = p.protocol.into();
        if !protocol.available() {
            return Err("ZMODEM needs lrzsz (sz and rz) installed".into());
        }
        let files = p
            .paths
            .iter()
            .map(|f| self.readable_path(f))
            .collect::<Result<Vec<_>, _>>()?;
        let total = files
            .iter()
            .filter_map(|f| std::fs::metadata(f).ok())
            .map(|m| m.len())
            .sum();
        self.run_transfer(
            &session,
            TransferRequest::Send { protocol, files },
            Some(total),
            ctx,
        )
        .await
    }

    /// Download files from the device into the transfer directory (YMODEM/ZMODEM keep the
    /// device's file names, XMODEM uses `name`). Sends progress notifications.
    #[tool(annotations(title = "Receive file", open_world_hint = true))]
    async fn file_receive(
        &self,
        Parameters(p): Parameters<FileReceiveParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<TransferResult>, String> {
        let session = self.session(p.session)?;
        let protocol: TransferProtocol = p.protocol.into();
        if !protocol.available() {
            return Err("ZMODEM needs lrzsz (sz and rz) installed".into());
        }
        let mut dir = self.options.transfer_dir.clone();
        if let Some(sub) = &p.subdir {
            let rel = Path::new(sub);
            if rel
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(format!("{sub:?} must be a relative path without `..`"));
            }
            dir = dir.join(rel);
        }
        if let Some(n) = &p.name
            && (n.contains(['/', '\\']) || n.starts_with('.') || n.is_empty())
        {
            return Err(format!("{n:?} must be a plain file name"));
        }
        let req = TransferRequest::Receive {
            protocol,
            dir,
            name: p.name,
            overwrite: p.overwrite,
        };
        self.run_transfer(&session, req, None, ctx).await
    }

    /// Cancel the running file transfer of a session (also one started in the window).
    #[tool(annotations(title = "Cancel transfer", idempotent_hint = true))]
    async fn file_cancel(&self, Parameters(p): Parameters<SessionArg>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let msg = if session.cancel_transfer().await {
            "transfer cancelled"
        } else {
            "no transfer was running"
        };
        Ok(Self::done(&session, msg))
    }

    /// Run an automation script (send/expect/delay/set_dtr/if_match/goto/…; read
    /// gtkterm://docs/script-language) and wait for it. `log` lines arrive as progress
    /// notifications. Failures report the line, the log so far and the received data.
    #[tool(annotations(title = "Run script", open_world_hint = true))]
    async fn script_run(
        &self,
        Parameters(p): Parameters<ScriptRunParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        let allowed = self.file_dirs();
        let script = match (&p.script, &p.path) {
            (Some(text), None) => {
                Script::parse_with("<script>", text, Some(&self.options.transfer_dir), Some(&allowed))
            }
            (None, Some(path)) => {
                let path = self.readable_path(path)?;
                Script::load(&path, Some(&allowed))
            }
            _ => return Err("give either `script` or `path`".into()),
        }
        .map_err(err)?;
        let opts = ScriptOptions {
            allow_exec: self.options.allow_exec,
            allowed_dirs: Some(allowed),
            vars: p.vars,
        };
        let token = ctx.meta.get_progress_token();
        let peer = ctx.peer.clone();
        let mut events = self.manager.subscribe();
        let id = session.id();
        let forward = tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            let mut n = 0u32;
            loop {
                let ev = match events.recv().await {
                    Ok(ev) => ev,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                };
                match ev {
                    gtkterm_core::manager::ManagerEvent::ScriptLog { session, message } if session == id => {
                        n += 1;
                        if let Some(t) = &token {
                            let _ = peer
                                .notify_progress(
                                    ProgressNotificationParam::new(t.clone(), f64::from(n))
                                        .with_message(message),
                                )
                                .await;
                        }
                    }
                    gtkterm_core::manager::ManagerEvent::ScriptFinished { session, .. } if session == id => {
                        break;
                    }
                    _ => {}
                }
            }
        });
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(600_000));
        let run = self.manager.run_script(&session, &script, &opts);
        // Dropping `run` (timeout, cancel) releases the session's script slot.
        let result = tokio::select! {
            r = tokio::time::timeout(timeout, run) => r,
            () = ctx.ct.cancelled() => {
                forward.abort();
                return Err("cancelled".into());
            }
        };
        let Ok(result) = result else {
            forward.abort();
            return Err(format!(
                "the script did not finish within {} ms",
                timeout.as_millis()
            ));
        };
        let _ = forward.await;
        match result {
            Ok(report) => Ok(Json(serde_json::to_value(report).map_err(err)?)),
            Err(ScriptError::Failed { at, message, report }) => Err(format!(
                "{at}: {message}\nlog: {:?}\nreceived (tail): {:?}",
                report.log,
                report
                    .received
                    .chars()
                    .rev()
                    .take(1000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            )),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Check a script for syntax errors without running it.
    #[tool(annotations(title = "Check script", read_only_hint = true))]
    async fn script_check(
        &self,
        Parameters(p): Parameters<ScriptCheckParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let allowed = self.file_dirs();
        let s = Script::parse_with(
            "<script>",
            &p.script,
            Some(&self.options.transfer_dir),
            Some(&allowed),
        )
        .map_err(err)?;
        Ok(Json(serde_json::json!({
            "statements": s.len(),
            "uses_exec": s.uses_exec(),
            "exec_allowed": self.options.allow_exec,
        })))
    }

    /// Stop the script or macro running in a session.
    #[tool(annotations(title = "Stop script", idempotent_hint = true))]
    async fn script_stop(&self, Parameters(p): Parameters<SessionArg>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let msg = if self.manager.stop_macro(session.id()) {
            "stopped"
        } else {
            "nothing was running"
        };
        Ok(Self::done(&session, msg))
    }

    /// Modem line history (DTR, RTS, CTS, DSR, DCD, RI) of the last seconds, polled every
    /// 50 ms; pulses shorter than that are recovered from the driver's counters.
    #[tool(annotations(title = "Signal history", read_only_hint = true))]
    async fn signal_history(
        &self,
        Parameters(p): Parameters<SignalHistoryParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        if p.csv {
            return Ok(Json(serde_json::json!({ "csv": session.signals_csv() })));
        }
        let secs = p.seconds.unwrap_or(60).clamp(1, 300);
        let since =
            jiff::Timestamp::now() - jiff::SignedDuration::from_secs(i64::try_from(secs).unwrap_or(60));
        let samples = session.signal_history(Some(since));
        Ok(Json(
            serde_json::json!({ "samples": samples, "count": samples.len() }),
        ))
    }

    /// Discard data waiting in the serial driver's input and output buffers.
    #[tool(annotations(title = "Flush port", idempotent_hint = true))]
    async fn port_flush(&self, Parameters(p): Parameters<SessionArg>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        session.flush().await.map_err(err)?;
        Ok(Self::done(&session, "flushed"))
    }

    /// Experimental: find the baud rate of a device that is sending text by trying common
    /// rates; the port is reopened at the best one.
    #[tool(annotations(title = "Detect baud rate", open_world_hint = true))]
    async fn port_auto_baud(
        &self,
        Parameters(p): Parameters<AutoBaudParams>,
        ct: CancellationToken,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        let candidates = p
            .candidates
            .unwrap_or_else(|| gtkterm_core::autobaud::CANDIDATES.to_vec());
        let listen = Duration::from_millis(p.listen_ms.unwrap_or(400).clamp(50, 5_000));
        let r = gtkterm_core::autobaud::auto_baud(&session, &candidates, listen, &ct)
            .await
            .map_err(err)?;
        Ok(Json(serde_json::to_value(r).map_err(err)?))
    }

    /// Pause or resume a Sunshine chart.
    #[tool(annotations(title = "Pause chart", idempotent_hint = true))]
    async fn sunshine_pause(&self, Parameters(p): Parameters<PauseParams>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let found = session.with_sunshine_mut(|s| s.chart_mut(&p.id).map(|c| c.paused = p.paused).is_some());
        if !found {
            return Err(format!("no chart {:?}", p.id));
        }
        session.notify_sunshine(gtkterm_core::sunshine::SunshineUpdate::ChartUpdated(p.id.clone()));
        Ok(Self::done(
            &session,
            format!("chart {} {}", p.id, if p.paused { "paused" } else { "resumed" }),
        ))
    }

    /// Clear one Sunshine chart's samples, or remove all charts.
    #[tool(annotations(title = "Clear charts", destructive_hint = true))]
    async fn sunshine_clear(
        &self,
        Parameters(p): Parameters<ClearChartParams>,
    ) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        if let Some(id) = &p.id {
            let found = session.with_sunshine_mut(|s| s.chart_mut(id).map(ChartData::clear).is_some());
            if !found {
                return Err(format!("no chart {id:?}"));
            }
            session.notify_sunshine(gtkterm_core::sunshine::SunshineUpdate::ChartUpdated(id.clone()));
        } else {
            session.with_sunshine_mut(gtkterm_core::sunshine::SunshineStore::clear);
            session.notify_sunshine(gtkterm_core::sunshine::SunshineUpdate::ChartsChanged);
        }
        Ok(Self::done(&session, "cleared"))
    }

    /// Start logging received (and, except for raw logs, sent) data to a file in the log directory.
    #[tool(annotations(title = "Start log"))]
    async fn log_start(
        &self,
        Parameters(p): Parameters<LogStartParams>,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        let format = match p.format.as_deref() {
            Some(f) => LogFormat::parse(f)
                .ok_or_else(|| format!("unknown format {f:?}; use raw, text, csv, json or xml"))?,
            None => LogFormat::Raw,
        };
        let path = self.log_path(&session, format, p.file_name.as_deref())?;
        let st = session
            .log(LogOp::Start {
                path,
                format,
                append: false,
                port_label: None,
            })
            .await
            .map_err(err)?;
        Ok(Json(serde_json::to_value(st).unwrap_or_default()))
    }

    /// Stop logging and close the log file.
    #[tool(annotations(title = "Stop log"))]
    async fn log_stop(
        &self,
        Parameters(p): Parameters<SessionArg>,
    ) -> Result<Json<serde_json::Value>, String> {
        let session = self.session(p.session)?;
        let st = session.log(LogOp::Stop).await.map_err(err)?;
        Ok(Json(serde_json::to_value(st).unwrap_or_default()))
    }

    /// Current log file, format and size.
    #[tool(annotations(title = "Log status", read_only_hint = true))]
    async fn log_status(
        &self,
        Parameters(p): Parameters<SessionArg>,
    ) -> Result<Json<serde_json::Value>, String> {
        let st = self.session(p.session)?.status();
        Ok(Json(
            serde_json::json!({ "log": st.log, "log_dir": self.options.log_dir }),
        ))
    }
}

const SUNSHINE_DOC: &str = include_str!("sunshine_protocol.md");
const SCRIPT_DOC: &str = crate::SCRIPT_LANGUAGE;

#[prompt_router]
impl GtkTermMcp {
    /// Find out what is connected to a serial port: try baud rates and line endings, identify the protocol.
    #[prompt(name = "debug-serial-device")]
    async fn debug_serial_device(&self, Parameters(a): Parameters<FocusArgs>) -> Vec<PromptMessage> {
        let focus = a.focus.map(|f| format!(" Focus: {f}.")).unwrap_or_default();
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Use the GTKTerm tools to identify the device on a serial port.{focus} Steps: call list_ports; \
                 open the most likely port at 115200 8N1; read for 2 s; if the output is garbage or empty, close and \
                 retry at 9600, 19200, 38400, 57600, 230400 and 921600; try sending an empty line with cr and with \
                 crlf and look for a prompt; if it looks like a modem, send AT. Report baud rate, line ending, \
                 protocol (plain text, NMEA, Modbus, AT, Sunshine packets, binary) and a sample of the output."
            ),
        )]
    }

    /// Analyse the live Sunshine data series: trends, ranges, anomalies.
    #[prompt(name = "analyze-sunshine-data")]
    async fn analyze_sunshine_data(&self, Parameters(a): Parameters<FocusArgs>) -> Vec<PromptMessage> {
        let focus = a
            .focus
            .map(|f| format!(" Concentrate on {f}."))
            .unwrap_or_default();
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Call sunshine_list_series, then sunshine_get_series for each chart (max_points 300).{focus} \
                 Describe each series: range, mean, trend, noise, periodicity, outliers and correlations between \
                 series. Point out anything that looks like a sensor or firmware problem."
            ),
        )]
    }

    /// Probe a Modbus RTU device: build requests with CRC and decode replies.
    #[prompt(name = "modbus-scan")]
    async fn modbus_scan(&self, Parameters(a): Parameters<FocusArgs>) -> Vec<PromptMessage> {
        let focus = a.focus.map(|f| format!(" ({f})")).unwrap_or_default();
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Scan a Modbus RTU bus{focus}. Open the port (usually 19200 8E1 or 9600 8N1; preset_list has a Modbus \
                 preset). For unit IDs 1-10 call modbus_request with function read_holding_registers, address 0, \
                 count 2 and timeout_ms 300; it builds the CRC and decodes the reply. List responding units, register \
                 values and exceptions. decoder_get_frames shows everything on the bus, including CRC failures."
            ),
        )]
    }

    /// Check an AT-command modem or module.
    #[prompt(name = "at-modem-check")]
    async fn at_modem_check(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Use expect with line_ending cr to send AT, ATI, AT+GMR, AT+CSQ and AT+CREG? one by one, each with \
             pattern '(OK|ERROR)\\r\\n' and timeout_ms 2000. Summarise model, firmware, signal quality and \
             registration state.",
        )]
    }

    /// Write firmware code that emits Sunshine packets for live plotting.
    #[prompt(name = "write-sunshine-firmware-snippet")]
    async fn write_sunshine_firmware_snippet(
        &self,
        Parameters(a): Parameters<FocusArgs>,
    ) -> Vec<PromptMessage> {
        let target = a
            .focus
            .unwrap_or_else(|| "C for a microcontroller (printf over UART)".into());
        vec![PromptMessage::new_text(
            Role::User,
            format!("Write {target} code that prints Sunshine packets as specified below.\n\n{SUNSHINE_DOC}"),
        )]
    }
}

fn resource(uri: &str, name: &str, description: &str, mime: &str) -> Resource {
    Resource::new(uri, name)
        .with_description(description)
        .with_mime_type(mime)
}

// The handler macros and the synchronous resource listings implement async trait methods
// without awaiting anything; that is intended.
#[allow(clippy::unused_async_trait_impl)]
#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for GtkTermMcp {
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, ErrorData> {
        let started = std::time::Instant::now();
        let tool = request.name.to_string();
        let args = request.arguments.clone();
        let client = context.peer.peer_info().map(|i| {
            let c = &i.client_info;
            if c.version.is_empty() {
                c.name.clone()
            } else {
                format!("{} {}", c.name, c.version)
            }
        });
        let audit = |outcome: &str, error: Option<String>| {
            if let Some(log) = &self.audit {
                log.write(&crate::guard::AuditEntry {
                    time: jiff::Timestamp::now(),
                    tool: &tool,
                    client: client.as_deref(),
                    arguments: crate::guard::audit_arguments(args.as_ref()),
                    outcome,
                    error,
                    ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                });
            }
        };
        if let Err(msg) = self.approve(&tool, args.as_ref(), client.as_deref()).await {
            audit("denied", Some(msg.clone()));
            return Ok(rmcp::model::CallToolResult::error(vec![rmcp::model::ContentBlock::text(msg)]).into());
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(tcc).await;
        match &result {
            Ok(rmcp::model::CallToolResponse::Complete(r)) if r.is_error == Some(true) => {
                let text = r
                    .content
                    .iter()
                    .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                    .collect::<Vec<_>>()
                    .join(" ");
                audit("error", Some(text));
            }
            Ok(_) => audit("ok", None),
            Err(e) => audit("error", Some(e.message.to_string())),
        }
        result
    }

    fn get_info(&self) -> ServerConfig {
        let mode = if self.options.read_only {
            " The server is read-only: data can be read but not sent."
        } else {
            ""
        };
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().enable_prompts().build())
            .with_server_info(Implementation::new("gtkterm-sunshine", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!(
                "GTKTerm Sunshine drives serial ports, TCP and UDP connections. Typical flow: list_ports, open_port, \
                 then send/read/expect. `read` and `expect` use byte cursors: keep `next_cursor`/`cursor` from the \
                 previous call to continue without missing data. Lines that look like {{ID|Name|value...}} are \
                 Sunshine packets and are plotted; inspect them with sunshine_list_series and sunshine_get_series. \
                 The protocol is described in the resource gtkterm://docs/sunshine-protocol.{mode}"
            ))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut resources = vec![
            resource(
                "gtkterm://ports",
                "ports",
                "Serial ports, sessions and profiles",
                "application/json",
            ),
            resource(
                "gtkterm://profiles",
                "profiles",
                "Saved connection profiles (TOML)",
                "application/toml",
            ),
            resource(
                "gtkterm://docs/script-language",
                "script-language",
                "Automation script language reference",
                "text/markdown",
            ),
            resource(
                "gtkterm://docs/sunshine-protocol",
                "sunshine-protocol",
                "Sunshine packet format",
                "text/markdown",
            ),
            resource(
                "gtkterm://macros",
                "macros",
                "Macros in editor syntax",
                "text/plain",
            ),
        ];
        for s in self.manager.list() {
            let id = s.id();
            resources.push(resource(
                &format!("gtkterm://session/{id}/status"),
                &format!("session-{id}-status"),
                "Session status",
                "application/json",
            ));
            resources.push(resource(
                &format!("gtkterm://session/{id}/rx"),
                &format!("session-{id}-rx"),
                "Last 64 KiB of received data",
                "text/plain",
            ));
            resources.push(resource(
                &format!("gtkterm://session/{id}/signals"),
                &format!("session-{id}-signals"),
                "Modem line history of the last 300 s (CSV)",
                "text/csv",
            ));
            resources.push(resource(
                &format!("gtkterm://session/{id}/frames"),
                &format!("session-{id}-frames"),
                "Last 200 decoded protocol frames",
                "application/json",
            ));
            resources.push(resource(
                &format!("gtkterm://session/{id}/sunshine"),
                &format!("session-{id}-sunshine"),
                "Sunshine chart summaries",
                "application/json",
            ));
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("gtkterm://session/{id}/status", "session-status")
                .with_mime_type("application/json"),
            ResourceTemplate::new("gtkterm://session/{id}/rx", "session-rx").with_mime_type("text/plain"),
            ResourceTemplate::new("gtkterm://session/{id}/signals", "session-signals")
                .with_mime_type("text/csv"),
            ResourceTemplate::new("gtkterm://session/{id}/frames", "session-frames")
                .with_mime_type("application/json"),
            ResourceTemplate::new("gtkterm://session/{id}/sunshine", "session-sunshine")
                .with_mime_type("application/json"),
            ResourceTemplate::new(
                "gtkterm://session/{id}/sunshine/{packet_id}",
                "sunshine-chart-csv",
            )
            .with_mime_type("text/csv"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri.as_str();
        let not_found = || ErrorData::resource_not_found(format!("unknown resource {uri}"), None);
        let text = |t: String| Ok(ReadResourceResult::new(vec![ResourceContents::text(t, uri)]).into());
        match uri {
            "gtkterm://docs/sunshine-protocol" => return text(SUNSHINE_DOC.to_owned()),
            "gtkterm://docs/script-language" => return text(SCRIPT_DOC.to_owned()),
            "gtkterm://macros" => {
                let all = self.manager.with_macros(|s| {
                    s.list()
                        .iter()
                        .map(|m| {
                            let key = m
                                .shortcut
                                .as_deref()
                                .map(|k| format!(" ({k})"))
                                .unwrap_or_default();
                            format!(
                                "# {}{key}
{}",
                                m.name,
                                gtkterm_core::macros::format_text(&m.commands)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(
                            "
",
                        )
                });
                return text(all);
            }
            "gtkterm://profiles" => {
                return text(self.manager.with_config(|c| c.to_toml().unwrap_or_default()));
            }
            "gtkterm://ports" => {
                let list = self
                    .list_ports()
                    .await
                    .map_err(|e| ErrorData::internal_error(e, None))?
                    .0;
                return text(serde_json::to_string_pretty(&list).unwrap_or_default());
            }
            _ => {}
        }
        let rest = uri.strip_prefix("gtkterm://session/").ok_or_else(not_found)?;
        let (id, what) = rest.split_once('/').ok_or_else(not_found)?;
        let session = id
            .parse()
            .ok()
            .and_then(|id| self.manager.get(id))
            .ok_or_else(not_found)?;
        match what.split_once('/') {
            None if what == "status" => {
                text(serde_json::to_string_pretty(&session.status()).unwrap_or_default())
            }
            None if what == "rx" => {
                let end = session.rx_cursor();
                let r = session.read_rx(end.saturating_sub(64 * 1024), 64 * 1024);
                text(String::from_utf8_lossy(&r.data).into_owned())
            }
            None if what == "signals" => text(session.signals_csv()),
            None if what == "frames" => {
                let frames: Vec<FrameOut> = session
                    .last_frames(200)
                    .iter()
                    .map(|f| frame_out(f, true))
                    .collect();
                text(serde_json::to_string_pretty(&frames).unwrap_or_default())
            }
            None if what == "sunshine" => {
                let charts: Vec<_> = session.with_sunshine(|s| s.charts().iter().map(summarize).collect());
                text(serde_json::to_string_pretty(&charts).unwrap_or_default())
            }
            Some(("sunshine", packet_id)) => {
                let csv = session
                    .with_sunshine(|s| s.chart(packet_id).map(ChartData::to_csv))
                    .ok_or_else(not_found)?;
                text(csv)
            }
            _ => Err(not_found()),
        }
    }
}
