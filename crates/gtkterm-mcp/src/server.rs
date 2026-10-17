//! The MCP server: tools, resources and prompts over a [`SessionManager`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gtkterm_core::manager::SessionManager;
use gtkterm_core::ports::{PortInfo, list_ports};
use gtkterm_core::session::{LogOp, Session, SessionError, SessionStatus};
use gtkterm_core::sunshine::{ChartData, SunshineStats};
use gtkterm_protocols::downsample::lttb_indices;
use gtkterm_protocols::logfmt::{LogFormat, session_file_name};
use gtkterm_protocols::sunshine::ChartKind;
use regex::bytes::Regex;
use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    Implementation, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams, PromptMessage,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate, Role, ServerCapabilities, ServerConfig,
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
    BreakParams, ChartParams, ChartTypeParams, ClearChartParams, ExpectParams, GetSeriesParams,
    LogStartParams, OpenPortParams, PauseParams, ReadParams, ReadResult, SearchHit, SearchParams, SendParams,
    SessionArg, SignalParams,
};

/// Read caps protecting the agent's context window and the app.
const MAX_READ: usize = 1 << 20;
const MAX_WAIT: Duration = Duration::from_secs(60);
const MAX_EXPECT: Duration = Duration::from_secs(600);

/// Tools that change device state or files; hidden in read-only mode.
pub const WRITE_TOOLS: &[&str] = &[
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
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            read_only: false,
            log_dir: default_log_dir(),
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

/// GTKTerm MCP server. Cheap to clone; all clones share the session manager.
#[derive(Debug, Clone)]
pub struct GtkTermMcp {
    manager: Arc<SessionManager>,
    options: ServerOptions,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
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
            options,
            tool_router,
            prompt_router: Self::prompt_router(),
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
        let ms = p.duration_ms.unwrap_or(250).clamp(1, 5_000);
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

    /// Pause or resume a Sunshine chart.
    #[tool(annotations(title = "Pause chart", idempotent_hint = true))]
    async fn sunshine_pause(&self, Parameters(p): Parameters<PauseParams>) -> Result<Json<Done>, String> {
        let session = self.session(p.session)?;
        let found = session.with_sunshine_mut(|s| s.chart_mut(&p.id).map(|c| c.paused = p.paused).is_some());
        if !found {
            return Err(format!("no chart {:?}", p.id));
        }
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
        match &p.id {
            Some(id) => {
                let found = session.with_sunshine_mut(|s| s.chart_mut(id).map(ChartData::clear).is_some());
                if !found {
                    return Err(format!("no chart {id:?}"));
                }
            }
            None => session.with_sunshine_mut(gtkterm_core::sunshine::SunshineStore::clear),
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
                "Scan a Modbus RTU bus{focus}. Open the port (usually 19200 8E1 or 9600 8N1). For unit IDs 1-10 send \
                 'read holding registers' (function 0x03, address 0, count 2) using send with encoding hex; append \
                 the CRC-16/MODBUS (poly 0xA001, init 0xFFFF, low byte first). Use read with timeout_ms 300 and \
                 encoding hex after each request. List responding units, decoded register values and exceptions."
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
                "gtkterm://docs/sunshine-protocol",
                "sunshine-protocol",
                "Sunshine packet format",
                "text/markdown",
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
