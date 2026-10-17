//! Tool parameter and result types (their doc comments become the JSON schema descriptions).

use base64::Engine as _;
use gtkterm_core::config::{Connection, FlowControl, Parity, SerialConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How bytes are represented in tool arguments and results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Encoding {
    /// UTF-8 text (invalid bytes are replaced when reading). Escapes like \n are NOT interpreted.
    #[default]
    Text,
    /// Hex bytes such as "DE AD BE EF" or "0x01 0x02".
    Hex,
    /// Standard base64.
    Base64,
}

impl Encoding {
    pub(crate) fn decode(self, data: &str) -> Result<Vec<u8>, String> {
        match self {
            Self::Text => Ok(data.as_bytes().to_vec()),
            Self::Hex => gtkterm_protocols::hex::parse_hex(data).map_err(|e| e.to_string()),
            Self::Base64 => base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| e.to_string()),
        }
    }

    pub(crate) fn encode(self, data: &[u8]) -> String {
        match self {
            Self::Text => String::from_utf8_lossy(data).into_owned(),
            Self::Hex => gtkterm_protocols::hex::to_hex(data),
            Self::Base64 => base64::engine::general_purpose::STANDARD.encode(data),
        }
    }
}

/// Line ending appended to sent data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LineEnding {
    /// Nothing appended.
    #[default]
    None,
    /// Carriage return (\r) — typical for AT modems.
    Cr,
    /// Line feed (\n).
    Lf,
    /// \r\n.
    Crlf,
}

impl LineEnding {
    pub(crate) fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Cr => b"\r",
            Self::Lf => b"\n",
            Self::Crlf => b"\r\n",
        }
    }
}

/// Which session to act on. Omit to use the default (first) session.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub(crate) struct SessionArg {
    /// Session id from `list_ports` / `get_status`; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
}

/// Connection to open.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ConnectionSpec {
    /// A serial device.
    Serial {
        /// Device path, e.g. /dev/ttyUSB0 (see list_ports).
        port: String,
        /// Baud rate, default 115200. Any rate is accepted (e.g. 250000).
        #[serde(default)]
        baudrate: Option<u32>,
        /// Data bits 5-8, default 8.
        #[serde(default)]
        bits: Option<u8>,
        /// none (default), odd, even, mark or space.
        #[serde(default)]
        parity: Option<String>,
        /// 1 (default) or 2.
        #[serde(default)]
        stopbits: Option<u8>,
        /// none (default), xon, rts or rs485.
        #[serde(default)]
        flow_control: Option<String>,
    },
    /// Connect to a TCP server.
    TcpClient {
        /// Host name or address.
        host: String,
        /// TCP port.
        port: u16,
    },
    /// Listen for TCP clients.
    TcpServer {
        /// TCP port to listen on.
        port: u16,
        /// Bind address, default 0.0.0.0.
        #[serde(default)]
        bind: Option<String>,
    },
    /// UDP socket.
    Udp {
        /// Local port (0 = any).
        #[serde(default)]
        local_port: u16,
        /// Remote host; if omitted, the first sender becomes the remote.
        #[serde(default)]
        remote_host: Option<String>,
        /// Remote port.
        #[serde(default)]
        remote_port: u16,
    },
}

impl ConnectionSpec {
    pub(crate) fn into_connection(self) -> Result<Connection, String> {
        Ok(match self {
            Self::Serial {
                port,
                baudrate,
                bits,
                parity,
                stopbits,
                flow_control,
            } => {
                let d = SerialConfig::default();
                Connection::Serial(SerialConfig {
                    port,
                    baudrate: baudrate.unwrap_or(d.baudrate),
                    bits: bits.unwrap_or(d.bits),
                    stopbits: stopbits.unwrap_or(d.stopbits),
                    parity: match parity {
                        Some(p) => Parity::parse(&p).ok_or_else(|| format!("unknown parity {p:?}"))?,
                        None => d.parity,
                    },
                    flow_control: match flow_control {
                        Some(f) => {
                            FlowControl::parse(&f).ok_or_else(|| format!("unknown flow control {f:?}"))?
                        }
                        None => d.flow_control,
                    },
                    ..d
                })
            }
            Self::TcpClient { host, port } => Connection::TcpClient {
                host,
                port,
                connect_timeout_ms: 5_000,
            },
            Self::TcpServer { port, bind } => Connection::TcpServer {
                bind: bind.unwrap_or_else(|| "0.0.0.0".into()),
                port,
            },
            Self::Udp {
                local_port,
                remote_host,
                remote_port,
            } => Connection::Udp {
                bind: "0.0.0.0".into(),
                local_port,
                remote_host,
                remote_port,
            },
        })
    }
}

/// `open_port` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct OpenPortParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Connection to open. Either this or `profile` is required.
    #[serde(default)]
    pub(crate) connection: Option<ConnectionSpec>,
    /// Name of a saved profile to open instead of `connection`.
    #[serde(default)]
    pub(crate) profile: Option<String>,
}

/// `send` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct SendParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Data to send, interpreted according to `encoding`.
    pub(crate) data: String,
    /// text (default), hex or base64.
    #[serde(default)]
    pub(crate) encoding: Encoding,
    /// Line ending appended after the data: none (default), cr, lf or crlf.
    #[serde(default)]
    pub(crate) line_ending: LineEnding,
}

/// `read` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ReadParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Byte cursor from a previous call (`next_cursor`). Omit to read only data arriving from now on.
    #[serde(default)]
    pub(crate) cursor: Option<u64>,
    /// Maximum bytes to return (default 4096, at most 1048576).
    #[serde(default)]
    pub(crate) max_bytes: Option<usize>,
    /// If no data is available yet, wait up to this long for some (default 0, at most 60000).
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    /// text (default), hex or base64.
    #[serde(default)]
    pub(crate) encoding: Encoding,
}

/// `read` result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct ReadResult {
    /// Received data.
    pub(crate) data: String,
    /// Number of bytes returned.
    pub(crate) bytes: usize,
    /// Pass this as `cursor` next time.
    pub(crate) next_cursor: u64,
    /// Bytes that were evicted from history before they could be read.
    pub(crate) lost: u64,
}

/// `expect` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ExpectParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Regular expression (Rust regex syntax) matched against received bytes.
    pub(crate) pattern: String,
    /// Optional data to send first (same encoding rules as `send`).
    #[serde(default)]
    pub(crate) send: Option<String>,
    /// Encoding of `send`.
    #[serde(default)]
    pub(crate) encoding: Encoding,
    /// Line ending appended to `send`.
    #[serde(default)]
    pub(crate) line_ending: LineEnding,
    /// Start matching at this cursor; default: data arriving after the call starts.
    #[serde(default)]
    pub(crate) cursor: Option<u64>,
    /// Timeout in ms (default 5000, at most 600000).
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
}

/// `send_break` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct BreakParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Break duration in ms (default 250, at most 5000).
    #[serde(default)]
    pub(crate) duration_ms: Option<u64>,
}

/// `set_signals` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct SignalParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Set DTR high (true) or low (false); omit to leave unchanged.
    #[serde(default)]
    pub(crate) dtr: Option<bool>,
    /// Set RTS high (true) or low (false); omit to leave unchanged.
    #[serde(default)]
    pub(crate) rts: Option<bool>,
}

/// `search_buffer` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct SearchParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Regular expression searched in the retained received data.
    pub(crate) pattern: String,
    /// Maximum matches (default 50, at most 1000).
    #[serde(default)]
    pub(crate) max_matches: Option<usize>,
}

/// A search hit.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct SearchHit {
    /// Byte offset of the match in the RX stream (usable as a `read` cursor).
    pub(crate) offset: u64,
    /// Matched text.
    pub(crate) text: String,
    /// The line containing the match.
    pub(crate) line: String,
}

/// Chart selection for Sunshine tools.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ChartParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Packet ID (chart) as shown by sunshine_list_series.
    pub(crate) id: String,
}

/// `sunshine_get_series` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct GetSeriesParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Packet ID (chart).
    pub(crate) id: String,
    /// Only these series (default: all).
    #[serde(default)]
    pub(crate) series: Option<Vec<String>>,
    /// Only the most recent N samples of each series.
    #[serde(default)]
    pub(crate) last_n: Option<usize>,
    /// Downsample (LTTB) to at most this many points per series (default 200, at most 5000).
    #[serde(default)]
    pub(crate) max_points: Option<usize>,
}

/// `sunshine_set_chart_type` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ChartTypeParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Packet ID (chart); the type is also remembered for charts that do not exist yet.
    pub(crate) id: String,
    /// line, bar, pie or scatter.
    pub(crate) kind: String,
}

/// `sunshine_pause` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct PauseParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Packet ID (chart).
    pub(crate) id: String,
    /// true to freeze the chart, false to continue.
    pub(crate) paused: bool,
}

/// `sunshine_clear` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ClearChartParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Packet ID to clear; omit to remove all charts.
    #[serde(default)]
    pub(crate) id: Option<String>,
}

/// `log_start` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct LogStartParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// raw (default), text, csv, json or xml.
    #[serde(default)]
    pub(crate) format: Option<String>,
    /// File name inside the log directory; default: <timestamp>_<port>_<profile>.<ext>.
    #[serde(default)]
    pub(crate) file_name: Option<String>,
}

/// `sunshine_export_png` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct PngParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Packet ID (chart).
    pub(crate) id: String,
    /// Image width in pixels (default 800, 200-2000).
    #[serde(default)]
    pub(crate) width: Option<i32>,
    /// Image height in pixels (default 450, 150-1500).
    #[serde(default)]
    pub(crate) height: Option<i32>,
    /// Dark theme instead of light.
    #[serde(default)]
    pub(crate) dark: bool,
}

/// `macro_run` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct MacroRunParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Macro name (see macro_list).
    pub(crate) name: String,
}

/// `macro_save` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct MacroSaveParams {
    /// Macro name; an existing macro with this name is replaced.
    pub(crate) name: String,
    /// Steps, one per line: `send: ATZ\r`, `send[250]: text` (pause first), `wait: 500 ms`,
    /// `expect: OK` or `expect[2000]: regex` (timeout in ms). `#` starts a comment.
    pub(crate) text: String,
    /// Optional keyboard shortcut in GTK syntax, e.g. `F9` or `<Control><Shift>1`.
    #[serde(default)]
    pub(crate) shortcut: Option<String>,
}

/// `preset_apply` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct PresetApplyParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Preset name (see preset_list), e.g. "Modbus RTU".
    pub(crate) name: String,
}

/// Profile selection.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ProfileParams {
    /// Profile name.
    pub(crate) name: String,
}

/// `profile_save` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ProfileSaveParams {
    /// Session whose current connection and terminal settings are saved; default: the first.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Profile name (created or replaced).
    pub(crate) name: String,
}
