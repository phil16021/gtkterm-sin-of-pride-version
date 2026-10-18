//! Tool parameter and result types (their doc comments become the JSON schema descriptions).

use base64::Engine as _;
use gtkterm_core::config::{Connection, FlowControl, Parity, SerialConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Optional parameters (absent = default) as plain types instead of `["integer", "null"]` or
/// `anyOf [.., null]`: single-`type` schema dialects (e.g. Gemini function declarations)
/// reject those, and `null` means the same as leaving the parameter out.
pub(crate) fn plain_optionals(schema: &mut schemars::Schema) {
    fn strip(p: &mut Value) {
        let Some(obj) = p.as_object_mut() else { return };
        if obj.get("default") == Some(&Value::Null) {
            obj.remove("default");
        }
        if let Some(Value::Array(types)) = obj.get_mut("type") {
            types.retain(|t| t != "null");
            if types.len() == 1 {
                let only = types.remove(0);
                obj.insert("type".into(), only);
            }
        }
        if let Some(Value::Array(alts)) = obj.get_mut("anyOf") {
            alts.retain(|a| a.get("type").and_then(Value::as_str) != Some("null"));
            if alts.len() == 1 {
                let only = alts.remove(0);
                obj.remove("anyOf");
                if let Value::Object(fields) = only {
                    for (k, v) in fields {
                        obj.entry(k).or_insert(v);
                    }
                }
            }
        }
    }
    fn walk(v: &mut Value) {
        let Some(obj) = v.as_object_mut() else { return };
        let required: Vec<String> = obj
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        if let Some(Value::Object(props)) = obj.get_mut("properties") {
            for (name, p) in props.iter_mut() {
                if !required.contains(name) {
                    strip(p);
                }
            }
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(Value::Array(alts)) = obj.get_mut(key) {
                alts.iter_mut().for_each(walk);
            }
        }
    }
    if let Some(obj) = schema.as_object_mut() {
        let mut v = Value::Object(std::mem::take(obj));
        walk(&mut v);
        if let Value::Object(o) = v {
            *obj = o;
        }
    }
}

/// How bytes are represented in tool arguments and results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
pub(crate) struct SessionArg {
    /// Session id from `list_ports` / `get_status`; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
}

/// Connection to open.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
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
        /// Largest datagram sent, in bytes (default and maximum 65507); longer data is split.
        #[serde(default)]
        max_datagram: Option<u16>,
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
                max_datagram,
            } => Connection::Udp {
                bind: "0.0.0.0".into(),
                local_port,
                remote_host,
                remote_port,
                max_datagram: max_datagram.unwrap_or(0),
            },
        })
    }
}

/// `open_port` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
    /// Pause after every byte, 0-1000 ms (slow bootloaders); default: the profile's character delay.
    #[serde(default)]
    pub(crate) char_delay_ms: Option<u32>,
}

/// `read` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
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
    /// With a TCP server: which client sent which bytes (absent otherwise).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) peers: Vec<PeerSpanOut>,
}

/// Bytes of one TCP client within a `read` result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct PeerSpanOut {
    /// Stream offset (cursor) of the first byte.
    pub(crate) offset: u64,
    /// Number of bytes.
    pub(crate) bytes: usize,
    /// Client address, e.g. 10.0.0.2:51234.
    pub(crate) peer: String,
}

impl From<&gtkterm_core::chunks::PeerSpan> for PeerSpanOut {
    fn from(p: &gtkterm_core::chunks::PeerSpan) -> Self {
        Self {
            offset: p.offset,
            bytes: p.len,
            peer: p.peer.to_string(),
        }
    }
}

/// `expect` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct ExpectParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Regular expression (Rust regex syntax) matched against received bytes, or literal bytes
    /// when `pattern_encoding` is given.
    pub(crate) pattern: String,
    /// Treat `pattern` as literal bytes in this encoding instead of a regular expression, e.g.
    /// `hex` with "AA 55" waits for the bytes 0xAA 0x55.
    #[serde(default)]
    pub(crate) pattern_encoding: Option<Encoding>,
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
pub(crate) struct ChartParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Packet ID (chart) as shown by sunshine_list_series.
    pub(crate) id: String,
}

/// `sunshine_get_series` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
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
    /// Only samples from the last N seconds (counted back from the newest sample of the chart).
    #[serde(default)]
    pub(crate) last_seconds: Option<f64>,
    /// Downsample (LTTB) to at most this many points per series (default 200, at most 5000).
    #[serde(default)]
    pub(crate) max_points: Option<usize>,
}

/// `sunshine_set_chart_type` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
pub(crate) struct MacroRunParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Macro name (see macro_list).
    pub(crate) name: String,
}

/// `macro_save` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
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
#[schemars(transform = plain_optionals)]
pub(crate) struct PresetApplyParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Preset name (see preset_list), e.g. "Modbus RTU".
    pub(crate) name: String,
}

/// Profile selection.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct ProfileParams {
    /// Profile name.
    pub(crate) name: String,
}

/// `configure_port` arguments: the serial settings to change; omitted ones stay.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct ConfigurePortParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Baud rate (any rate, e.g. 250000).
    #[serde(default)]
    pub(crate) baudrate: Option<u32>,
    /// Data bits 5-8.
    #[serde(default)]
    pub(crate) bits: Option<u8>,
    /// none, odd, even, mark or space.
    #[serde(default)]
    pub(crate) parity: Option<String>,
    /// 1 or 2.
    #[serde(default)]
    pub(crate) stopbits: Option<u8>,
    /// none, xon, rts or rs485.
    #[serde(default)]
    pub(crate) flow_control: Option<String>,
}

impl ConfigurePortParams {
    /// Apply the given settings to `cfg`.
    pub(crate) fn apply(&self, cfg: &mut SerialConfig) -> Result<(), String> {
        if let Some(b) = self.baudrate {
            cfg.baudrate = b;
        }
        if let Some(b) = self.bits {
            cfg.bits = b;
        }
        if let Some(b) = self.stopbits {
            cfg.stopbits = b;
        }
        if let Some(p) = &self.parity {
            cfg.parity = Parity::parse(p).ok_or_else(|| format!("unknown parity {p:?}"))?;
        }
        if let Some(f) = &self.flow_control {
            cfg.flow_control = FlowControl::parse(f).ok_or_else(|| format!("unknown flow control {f:?}"))?;
        }
        Ok(())
    }
}

/// `profile_save` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct ProfileSaveParams {
    /// Session whose current connection and terminal settings are saved; default: the first.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Profile name (created or replaced).
    pub(crate) name: String,
}

/// `decoder_set` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct DecoderSetParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// off, auto, raw, at, nmea, modbus_rtu, modbus_ascii, modbus_tcp, can (candump text), slcan,
    /// ethercat or rs232 (modem line changes only).
    pub(crate) mode: String,
    /// Also record modem line changes (DTR/RTS/CTS/DSR/DCD/RI) as frames.
    #[serde(default)]
    pub(crate) signals: bool,
    /// Drop the frames decoded so far.
    #[serde(default)]
    pub(crate) clear: bool,
}

/// `decoder_get_frames` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct GetFramesParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// First sequence number to return (use `next_seq` of the previous call); default: the newest `max` frames.
    #[serde(default)]
    pub(crate) since: Option<u64>,
    /// Maximum number of frames (default 100, at most 1000).
    #[serde(default)]
    pub(crate) max: Option<usize>,
    /// Wait up to this many ms for a frame when none is available (default 0, at most 60000).
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    /// Include each frame's bytes as hex (default true).
    #[serde(default)]
    pub(crate) include_bytes: Option<bool>,
}

/// Modbus framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ModbusFraming {
    /// Binary RTU with CRC-16 (serial lines).
    Rtu,
    /// ASCII `:`…LRC CRLF (serial lines).
    Ascii,
    /// MBAP header (Modbus TCP; use with a TCP connection to port 502).
    Tcp,
}

/// A Modbus request.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum ModbusRequestSpec {
    /// 0x01: read coils.
    ReadCoils {
        /// First coil address (0-based).
        address: u16,
        /// Number of coils (1-2000).
        count: u16,
    },
    /// 0x02: read discrete inputs.
    ReadDiscreteInputs {
        /// First input address (0-based).
        address: u16,
        /// Number of inputs (1-2000).
        count: u16,
    },
    /// 0x03: read holding registers.
    ReadHoldingRegisters {
        /// First register address (0-based, so 40001 is 0).
        address: u16,
        /// Number of registers (1-125).
        count: u16,
    },
    /// 0x04: read input registers.
    ReadInputRegisters {
        /// First register address (0-based, so 30001 is 0).
        address: u16,
        /// Number of registers (1-125).
        count: u16,
    },
    /// 0x05: write a single coil.
    WriteSingleCoil {
        /// Coil address.
        address: u16,
        /// On or off.
        value: bool,
    },
    /// 0x06: write a single register.
    WriteSingleRegister {
        /// Register address.
        address: u16,
        /// Value.
        value: u16,
    },
    /// 0x0F: write multiple coils.
    WriteMultipleCoils {
        /// First coil address.
        address: u16,
        /// Values (1-1968).
        values: Vec<bool>,
    },
    /// 0x10: write multiple registers.
    WriteMultipleRegisters {
        /// First register address.
        address: u16,
        /// Values (1-123).
        values: Vec<u16>,
    },
}

impl From<ModbusRequestSpec> for gtkterm_protocols::decode::modbus::Request {
    fn from(s: ModbusRequestSpec) -> Self {
        use ModbusRequestSpec as S;
        match s {
            S::ReadCoils { address, count } => Self::ReadCoils { address, count },
            S::ReadDiscreteInputs { address, count } => Self::ReadDiscreteInputs { address, count },
            S::ReadHoldingRegisters { address, count } => Self::ReadHoldingRegisters { address, count },
            S::ReadInputRegisters { address, count } => Self::ReadInputRegisters { address, count },
            S::WriteSingleCoil { address, value } => Self::WriteSingleCoil { address, value },
            S::WriteSingleRegister { address, value } => Self::WriteSingleRegister { address, value },
            S::WriteMultipleCoils { address, values } => Self::WriteMultipleCoils { address, values },
            S::WriteMultipleRegisters { address, values } => Self::WriteMultipleRegisters { address, values },
        }
    }
}

/// `modbus_request` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct ModbusRequestParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Unit (slave) ID 1-247; 0 broadcasts (no response is awaited).
    pub(crate) unit: u8,
    /// The request.
    pub(crate) request: ModbusRequestSpec,
    /// Framing; default: the session's Modbus decoder, else rtu.
    #[serde(default)]
    pub(crate) framing: Option<ModbusFraming>,
    /// Response timeout in ms (default 1000, at most 60000).
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
}

/// File transfer protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransferProtocolArg {
    /// 128-byte blocks, CRC or checksum (one file).
    Xmodem,
    /// 1024-byte blocks (one file).
    Xmodem1k,
    /// Batch with names and sizes.
    Ymodem,
    /// YMODEM streaming, error-free links only.
    YmodemG,
    /// ZMODEM via lrzsz (sz/rz must be installed).
    Zmodem,
}

impl From<TransferProtocolArg> for gtkterm_core::transfer::TransferProtocol {
    fn from(p: TransferProtocolArg) -> Self {
        match p {
            TransferProtocolArg::Xmodem => Self::Xmodem,
            TransferProtocolArg::Xmodem1k => Self::Xmodem1k,
            TransferProtocolArg::Ymodem => Self::Ymodem,
            TransferProtocolArg::YmodemG => Self::YmodemG,
            TransferProtocolArg::Zmodem => Self::Zmodem,
        }
    }
}

/// `file_send` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct FileSendParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Protocol. The device must already be waiting to receive (e.g. after `rx`/`rb`/`rz` or a
    /// bootloader's "send file now" prompt).
    pub(crate) protocol: TransferProtocolArg,
    /// Files to send: absolute paths inside the transfer directory or a directory allowed with
    /// --mcp-allow-dir, or names relative to the transfer directory. XMODEM takes exactly one.
    pub(crate) paths: Vec<String>,
}

/// `file_receive` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct FileReceiveParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Protocol. Start the transfer on the device right after (or just before) this call.
    pub(crate) protocol: TransferProtocolArg,
    /// Subdirectory of the transfer directory to save into (created if missing).
    #[serde(default)]
    pub(crate) subdir: Option<String>,
    /// File name for XMODEM, which carries none (default: xmodem-<time>.bin).
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Replace existing files instead of saving as "name (1).ext".
    #[serde(default)]
    pub(crate) overwrite: bool,
}

/// `script_run` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct ScriptRunParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Script text (see the resource gtkterm://docs/script-language). Give this or `path`.
    #[serde(default)]
    pub(crate) script: Option<String>,
    /// Script file inside the transfer directory or an --mcp-allow-dir directory.
    #[serde(default)]
    pub(crate) path: Option<String>,
    /// Initial variables, usable as ${name}.
    #[serde(default)]
    pub(crate) vars: std::collections::BTreeMap<String, String>,
    /// Give up after this many ms (default 600000).
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
}

/// `script_check` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct ScriptCheckParams {
    /// Script text to validate.
    pub(crate) script: String,
}

/// `signal_history` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct SignalHistoryParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Look back this many seconds (default 60, at most 300).
    #[serde(default)]
    pub(crate) seconds: Option<u64>,
    /// Return CSV (Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals) instead of samples.
    #[serde(default)]
    pub(crate) csv: bool,
}

/// `port_auto_baud` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(transform = plain_optionals)]
pub(crate) struct AutoBaudParams {
    /// Session to use; default: the first session.
    #[serde(default)]
    pub(crate) session: Option<u32>,
    /// Rates to try (default: common rates from 2400 to 1000000).
    #[serde(default)]
    pub(crate) candidates: Option<Vec<u32>>,
    /// Listen time per rate in ms (default 400).
    #[serde(default)]
    pub(crate) listen_ms: Option<u64>,
}
