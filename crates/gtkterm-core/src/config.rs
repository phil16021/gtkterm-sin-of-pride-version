//! Configuration: connection and terminal settings grouped into named profiles.
//!
//! Stored as TOML in `$XDG_CONFIG_HOME/gtkterm-sunshine/config.toml`:
//!
//! ```toml
//! [profiles.default.connection]
//! type = "serial"
//! port = "/dev/ttyUSB0"
//! baudrate = 115200
//!
//! [profiles.default.terminal]
//! echo = false
//! ```
//!
//! Old GTKTerm key files (`~/.config/.gtktermrc`, both the upstream 1.x keys such as `speed` /
//! `flow` / `crlfauto` and the C fork's `baudrate` / `flow_control` / `term_*` keys) are
//! imported by [`import_legacy_keyfile`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use gtkterm_protocols::transform::TimestampFormat;
use gtkterm_protocols::view::ViewMode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Name of the profile used when none is selected.
pub const DEFAULT_PROFILE: &str = "default";
/// Profile names must be shorter than this (same limit as the C version).
pub const MAX_PROFILE_NAME: usize = 32;
/// Baud rates offered in the UI. Any other rate is possible through termios2/BOTHER.
pub const STANDARD_BAUD_RATES: &[u32] = &[
    300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115_200, 230_400, 460_800, 500_000, 576_000,
    921_600, 1_000_000, 2_000_000,
];

/// Configuration errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// I/O error reading or writing the file.
    #[error("{path}: {source}")]
    Io {
        /// File involved.
        path: PathBuf,
        /// Cause.
        source: std::io::Error,
    },
    /// TOML syntax or schema error.
    #[error("{path}: {source}")]
    Parse {
        /// File involved.
        path: PathBuf,
        /// Cause.
        source: toml::de::Error,
    },
    /// Serialization failure.
    #[error("cannot serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// A value outside its allowed range.
    #[error("{field}: {message}")]
    Invalid {
        /// Field name, using the legacy key names.
        field: &'static str,
        /// What is wrong.
        message: String,
    },
    /// Unknown profile.
    #[error("no profile named [{0}]")]
    NoSuchProfile(String),
    /// Bad profile name.
    #[error("invalid profile name {0:?} (1-{max} characters, no '[', ']' or '.')", max = MAX_PROFILE_NAME - 1)]
    BadProfileName(String),
}

fn invalid(field: &'static str, message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        field,
        message: message.into(),
    }
}

/// Parity setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    /// No parity bit.
    #[default]
    None,
    /// Odd parity.
    Odd,
    /// Even parity.
    Even,
    /// Parity bit always 1.
    Mark,
    /// Parity bit always 0.
    Space,
}

impl Parity {
    /// One-letter code used in the status line (`8N1`).
    pub fn letter(self) -> char {
        match self {
            Self::None => 'N',
            Self::Odd => 'O',
            Self::Even => 'E',
            Self::Mark => 'M',
            Self::Space => 'S',
        }
    }

    /// Parse the CLI/legacy spelling (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "n" => Some(Self::None),
            "odd" | "o" => Some(Self::Odd),
            "even" | "e" => Some(Self::Even),
            "mark" | "m" => Some(Self::Mark),
            "space" | "s" => Some(Self::Space),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Odd => "odd",
            Self::Even => "even",
            Self::Mark => "mark",
            Self::Space => "space",
        }
    }
}

/// Flow control setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowControl {
    /// No flow control.
    #[default]
    None,
    /// Software XON/XOFF.
    XonXoff,
    /// Hardware RTS/CTS.
    RtsCts,
    /// RS485 half duplex: RTS asserted while transmitting.
    Rs485,
}

impl FlowControl {
    /// Parse the CLI/legacy spelling (`Xon`, `RTS`, `RS485`, `none`, ...).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "xon" | "xonxoff" | "xon_xoff" | "xon/xoff" => Some(Self::XonXoff),
            "rts" | "rtscts" | "rts_cts" | "rts/cts" => Some(Self::RtsCts),
            "rs485" => Some(Self::Rs485),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::XonXoff => "xon",
            Self::RtsCts => "rts",
            Self::Rs485 => "rs485",
        }
    }
}

/// Serial port settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SerialConfig {
    /// Device path, e.g. `/dev/ttyUSB0`.
    pub port: String,
    /// Baud rate; any positive value (non-standard rates use termios2).
    pub baudrate: u32,
    /// Data bits 5–8.
    pub bits: u8,
    /// Stop bits 1 or 2.
    pub stopbits: u8,
    /// Parity.
    pub parity: Parity,
    /// Flow control.
    pub flow_control: FlowControl,
    /// RS485: delay between asserting RTS and sending, in ms.
    pub rs485_rts_time_before_tx: u32,
    /// RS485: delay between the last byte and releasing RTS, in ms.
    pub rs485_rts_time_after_tx: u32,
    /// Do not take an exclusive lock on the device.
    pub disable_port_lock: bool,
    /// Length of a break condition, in ms.
    pub break_ms: u32,
    /// RS485: RTS is low while transmitting (some transceivers need the inverted level).
    pub rs485_rts_active_low: bool,
    /// RS485: keep receiving while transmitting (hear our own echo).
    pub rs485_rx_during_tx: bool,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            port: "/dev/ttyS0".into(),
            baudrate: 115_200,
            bits: 8,
            stopbits: 1,
            parity: Parity::None,
            flow_control: FlowControl::None,
            rs485_rts_time_before_tx: 30,
            rs485_rts_time_after_tx: 30,
            disable_port_lock: false,
            break_ms: 250,
            rs485_rts_active_low: false,
            rs485_rx_during_tx: false,
        }
    }
}

impl SerialConfig {
    /// Short description, e.g. `/dev/ttyUSB0 115200-8-N-1`.
    pub fn summary(&self) -> String {
        format!(
            "{} {}-{}-{}-{}",
            self.port,
            self.baudrate,
            self.bits,
            self.parity.letter(),
            self.stopbits
        )
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.port.trim().is_empty() {
            return Err(invalid("port", "must not be empty"));
        }
        if !(50..=12_000_000).contains(&self.baudrate) {
            return Err(invalid(
                "baudrate",
                format!("{} is out of range 50-12000000", self.baudrate),
            ));
        }
        if !(5..=8).contains(&self.bits) {
            return Err(invalid("bits", format!("{} must be 5-8", self.bits)));
        }
        if !matches!(self.stopbits, 1 | 2) {
            return Err(invalid("stopbits", format!("{} must be 1 or 2", self.stopbits)));
        }
        if self.rs485_rts_time_before_tx > 10_000 || self.rs485_rts_time_after_tx > 10_000 {
            return Err(invalid("rs485_rts_time", "must be at most 10000 ms"));
        }
        Ok(())
    }
}

/// Where to connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Connection {
    /// Local serial device.
    Serial(SerialConfig),
    /// Outgoing TCP connection.
    TcpClient {
        /// Host name or address.
        host: String,
        /// TCP port.
        port: u16,
        /// Connect timeout in ms.
        #[serde(default = "default_connect_timeout")]
        connect_timeout_ms: u64,
    },
    /// Listening TCP server; all clients are merged.
    TcpServer {
        /// Bind address, e.g. `0.0.0.0` or `::`.
        #[serde(default = "default_bind")]
        bind: String,
        /// TCP port.
        port: u16,
    },
    /// UDP socket.
    Udp {
        /// Local bind address.
        #[serde(default = "default_bind")]
        bind: String,
        /// Local port (0 = any).
        #[serde(default)]
        local_port: u16,
        /// Remote host; if `None`, the first sender becomes the remote.
        #[serde(default)]
        remote_host: Option<String>,
        /// Remote port.
        #[serde(default)]
        remote_port: u16,
    },
}

fn default_connect_timeout() -> u64 {
    5_000
}

fn default_bind() -> String {
    "0.0.0.0".into()
}

impl Default for Connection {
    fn default() -> Self {
        Self::Serial(SerialConfig::default())
    }
}

impl Connection {
    /// Endpoint description shown in title and status bar; also the C endpoint strings
    /// (`TCP:host:port`, `TCP-Server:port`, `UDP:local:lport->remote:rport`).
    pub fn summary(&self) -> String {
        match self {
            Self::Serial(s) => s.summary(),
            Self::TcpClient { host, port, .. } => format!("TCP:{host}:{port}"),
            Self::TcpServer { port, .. } => format!("TCP-Server:{port}"),
            Self::Udp {
                bind,
                local_port,
                remote_host,
                remote_port,
            } => format!(
                "UDP:{bind}:{local_port}->{}:{remote_port}",
                remote_host.as_deref().unwrap_or("*")
            ),
        }
    }

    /// Serial settings, if this is a serial connection.
    pub fn serial(&self) -> Option<&SerialConfig> {
        match self {
            Self::Serial(s) => Some(s),
            _ => None,
        }
    }

    /// Mutable serial settings; switches to serial (with defaults) if needed.
    pub fn serial_mut(&mut self) -> &mut SerialConfig {
        if !matches!(self, Self::Serial(_)) {
            *self = Self::Serial(SerialConfig::default());
        }
        match self {
            Self::Serial(s) => s,
            _ => unreachable!("just set to serial"),
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        match self {
            Self::Serial(s) => s.validate(),
            Self::TcpClient { host, port, .. } => {
                if host.trim().is_empty() {
                    return Err(invalid("host", "must not be empty"));
                }
                if *port == 0 {
                    return Err(invalid("port", "TCP port must not be 0"));
                }
                Ok(())
            }
            Self::TcpServer { port, .. } if *port == 0 => Err(invalid("port", "TCP port must not be 0")),
            Self::TcpServer { .. } => Ok(()),
            Self::Udp {
                remote_host,
                remote_port,
                ..
            } => {
                if remote_host.is_some() && *remote_port == 0 {
                    return Err(invalid("remote_port", "required when remote_host is set"));
                }
                Ok(())
            }
        }
    }
}

/// RGBA colour with components in 0.0–1.0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    /// Red.
    pub red: f64,
    /// Green.
    pub green: f64,
    /// Blue.
    pub blue: f64,
    /// Alpha.
    pub alpha: f64,
}

impl Rgba {
    /// Construct from components.
    pub const fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// Terminal and display settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Show transmitted bytes locally.
    pub echo: bool,
    /// Insert LF after a bare CR.
    pub auto_lf: bool,
    /// Insert CR before a bare LF.
    pub auto_cr: bool,
    /// Prefix lines with timestamps.
    pub timestamps: bool,
    /// Timestamp format.
    pub timestamp_format: TimestampFormat,
    /// Pango font description.
    pub font: String,
    /// Block cursor instead of I-beam.
    pub block_cursor: bool,
    /// Show the cursor.
    pub show_cursor: bool,
    /// Rows of the initial window.
    pub rows: u16,
    /// Columns of the initial window.
    pub columns: u16,
    /// Scrollback lines.
    pub scrollback: u32,
    /// Show the byte index in dump views.
    pub show_index: bool,
    /// View mode.
    pub view_mode: ViewMode,
    /// Bytes per line in hex view.
    pub hex_chars: u8,
    /// Flash instead of beeping.
    pub visual_bell: bool,
    /// Text colour.
    pub foreground: Rgba,
    /// Background colour.
    pub background: Rgba,
    /// Delay after each transmitted line in ms (0 = none).
    pub wait_delay_ms: u32,
    /// Wait for this byte from the device after each transmitted line.
    pub wait_char: Option<u8>,
    /// Default file for "send raw file".
    pub raw_filename: Option<String>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            echo: false,
            auto_lf: false,
            auto_cr: false,
            timestamps: false,
            timestamp_format: TimestampFormat::LocalTime,
            font: "Monospace 12".into(),
            block_cursor: true,
            show_cursor: true,
            rows: 25,
            columns: 80,
            scrollback: 10_000,
            show_index: false,
            view_mode: ViewMode::Ascii,
            hex_chars: 16,
            visual_bell: false,
            foreground: Rgba::new(0.66, 0.66, 0.66, 1.0),
            background: Rgba::new(0.0, 0.0, 0.0, 1.0),
            wait_delay_ms: 0,
            wait_char: None,
            raw_filename: None,
        }
    }
}

impl TerminalConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.wait_delay_ms > 500 {
            return Err(invalid(
                "term_wait_delay",
                format!("{} ms exceeds 500 ms", self.wait_delay_ms),
            ));
        }
        if !(1..=128).contains(&self.hex_chars) {
            return Err(invalid(
                "term_hex_chars",
                format!("{} must be 1-128", self.hex_chars),
            ));
        }
        if self.rows == 0 || self.columns == 0 {
            return Err(invalid("term_rows/term_columns", "must be positive"));
        }
        for (field, c) in [
            ("term_foreground", self.foreground),
            ("term_background", self.background),
        ] {
            if [c.red, c.green, c.blue, c.alpha]
                .iter()
                .any(|v| !(0.0..=1.0).contains(v))
            {
                return Err(invalid(field, "colour components must be within 0.0-1.0"));
            }
        }
        Ok(())
    }
}

/// A named set of settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Connection settings.
    pub connection: Connection,
    /// Terminal settings.
    pub terminal: TerminalConfig,
    /// Protocol decoder.
    #[serde(skip_serializing_if = "crate::decoding::DecoderConfig::is_default")]
    pub decoder: crate::decoding::DecoderConfig,
}

impl Profile {
    /// Check all values.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.connection.validate()?;
        self.terminal.validate()
    }

    /// Human readable dump in the layout of the C `--show_config` output.
    pub fn describe(&self, name: &str) -> String {
        fn onoff(b: bool) -> String {
            if b { "true" } else { "false" }.to_owned()
        }
        let mut sections: Vec<(&str, Vec<(&str, String)>)> = Vec::new();
        match &self.connection {
            Connection::Serial(c) => sections.push((
                "Serial port settings:",
                vec![
                    ("port", c.port.clone()),
                    ("baudrate", c.baudrate.to_string()),
                    ("bits", c.bits.to_string()),
                    ("stopbits", c.stopbits.to_string()),
                    ("parity", c.parity.name().to_owned()),
                    ("flow_control", c.flow_control.name().to_owned()),
                    ("rs485_rts_time_before_tx", c.rs485_rts_time_before_tx.to_string()),
                    ("rs485_rts_time_after_tx", c.rs485_rts_time_after_tx.to_string()),
                    ("disable_port_lock", onoff(c.disable_port_lock)),
                ],
            )),
            other => sections.push(("Connection settings:", vec![("endpoint", other.summary())])),
        }
        let t = &self.terminal;
        sections.push((
            "Terminal settings:",
            vec![
                ("term_font", t.font.clone()),
                ("term_raw_filename", t.raw_filename.clone().unwrap_or_default()),
                ("term_echo", onoff(t.echo)),
                ("term_auto_lf", onoff(t.auto_lf)),
                ("term_auto_cr", onoff(t.auto_cr)),
                ("term_wait_delay", t.wait_delay_ms.to_string()),
                ("term_wait_char", t.wait_char.map_or(-1, i32::from).to_string()),
                ("term_show_timestamp", onoff(t.timestamps)),
                ("term_block_cursor", onoff(t.block_cursor)),
                ("term_show_cursor", onoff(t.show_cursor)),
                ("term_rows", t.rows.to_string()),
                ("term_columns", t.columns.to_string()),
                ("term_scrollback", t.scrollback.to_string()),
                ("term_index", onoff(t.show_index)),
                ("term_view_mode", t.view_mode.label().to_owned()),
                ("term_hex_chars", t.hex_chars.to_string()),
                ("term_visual_bell", onoff(t.visual_bell)),
            ],
        ));
        let mut out = format!("Configuration [{name}]\n");
        for (title, items) in sections {
            let _ = write!(out, "\n{title}\n");
            for (key, value) in items {
                let _ = writeln!(out, "{key:<24} : {value}");
            }
        }
        out
    }
}

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Profiles by name; `default` always exists.
    pub profiles: BTreeMap<String, Profile>,
    /// User serial presets (built-ins are in [`crate::presets::builtin_presets`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub presets: BTreeMap<String, crate::presets::SerialPreset>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profiles: BTreeMap::from([(DEFAULT_PROFILE.to_owned(), Profile::default())]),
            presets: BTreeMap::new(),
        }
    }
}

/// Validate a profile name.
pub fn check_profile_name(name: &str) -> Result<(), ConfigError> {
    let ok = !name.is_empty()
        && name.chars().count() < MAX_PROFILE_NAME
        && !name.contains(['[', ']', '.', '\n', '\r'])
        && name.trim() == name;
    if ok {
        Ok(())
    } else {
        Err(ConfigError::BadProfileName(name.to_owned()))
    }
}

impl Config {
    /// Parse TOML text; ensures the default profile exists.
    pub fn from_toml(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let mut cfg: Self = toml::from_str(text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        cfg.profiles.entry(DEFAULT_PROFILE.to_owned()).or_default();
        Ok(cfg)
    }

    /// Serialize to TOML.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Load from `path`. A missing file yields the defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Atomically write to `path` (write to a temp file, then rename), creating directories.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let io = |source| ConfigError::Io {
            path: path.to_owned(),
            source,
        };
        crate::fsutil::write_atomic(path, self.to_toml()?.as_bytes(), None).map_err(io)
    }

    /// Profile by name.
    pub fn profile(&self, name: &str) -> Result<&Profile, ConfigError> {
        self.profiles
            .get(name)
            .ok_or_else(|| ConfigError::NoSuchProfile(name.to_owned()))
    }

    /// Profile by name, created from defaults if it does not exist yet.
    pub fn profile_mut(&mut self, name: &str) -> Result<&mut Profile, ConfigError> {
        check_profile_name(name)?;
        Ok(self.profiles.entry(name.to_owned()).or_default())
    }

    /// Remove a profile. Removing `default` resets it instead, as the C version did.
    pub fn remove_profile(&mut self, name: &str) -> Result<(), ConfigError> {
        if name == DEFAULT_PROFILE {
            self.profiles
                .insert(DEFAULT_PROFILE.to_owned(), Profile::default());
            return Ok(());
        }
        self.profiles
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| ConfigError::NoSuchProfile(name.to_owned()))
    }

    /// `--list_config` output.
    pub fn list_text(&self) -> String {
        let mut s = String::from("Configurations found:\n\n");
        for name in self.profiles.keys() {
            let _ = writeln!(s, "[{name}]");
        }
        s
    }
}

/// Directory holding the configuration. `GTKTERM_SUNSHINE_CONFIG_DIR` overrides it (tests).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GTKTERM_SUNSHINE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    directories::BaseDirs::new()
        .map_or_else(|| PathBuf::from(".config"), |d| d.config_dir().to_owned())
        .join("gtkterm-sunshine")
}

/// Path of `config.toml`.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Path of an existing legacy GTKTerm configuration: `~/.config/.gtktermrc` (C GTKTerm 2.x)
/// or `~/.gtktermrc` (upstream 1.x).
pub fn legacy_keyfile_path() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    [
        dirs.config_dir().join(".gtktermrc"),
        dirs.home_dir().join(".gtktermrc"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// Load the configuration; on first run, import the legacy key file if present.
/// Returns the config and an optional message describing an import.
pub fn load_or_import(path: &Path, legacy: Option<&Path>) -> Result<(Config, Option<String>), ConfigError> {
    if path.exists() {
        return Ok((Config::load(path)?, None));
    }
    let Some(legacy) = legacy.filter(|p| p.exists()) else {
        return Ok((Config::default(), None));
    };
    let text = std::fs::read(legacy).map_err(|source| ConfigError::Io {
        path: legacy.to_owned(),
        source,
    })?;
    let (config, warnings) = import_legacy_keyfile(&String::from_utf8_lossy(&text));
    config.save(path)?;
    let mut msg = format!(
        "Imported {} profile(s) from {} into {}",
        config.profiles.len(),
        legacy.display(),
        path.display()
    );
    for w in warnings {
        let _ = write!(msg, "\n  warning: {w}");
    }
    Ok((config, Some(msg)))
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Some(true),
        "false" | "off" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// Import an old GTKTerm key file. Unknown keys and bad values produce warnings, not errors.
pub fn import_legacy_keyfile(text: &str) -> (Config, Vec<String>) {
    let mut config = Config {
        profiles: BTreeMap::new(),
        presets: BTreeMap::new(),
    };
    let mut warnings = Vec::new();
    let mut current: Option<String> = None;

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim().to_owned();
            if check_profile_name(&name).is_err() {
                warnings.push(format!(
                    "line {}: skipping section with invalid name [{name}]",
                    lineno + 1
                ));
                current = None;
                continue;
            }
            config.profiles.entry(name.clone()).or_default();
            current = Some(name);
            continue;
        }
        let Some(name) = current.as_ref() else { continue };
        let Some((key, value)) = line.split_once('=') else {
            warnings.push(format!("line {}: not a key = value pair", lineno + 1));
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        let profile = config.profiles.get_mut(name).expect("section inserted above");
        if let Err(msg) = apply_legacy_key(profile, key, value) {
            warnings.push(format!("[{name}] {key}: {msg}"));
        }
    }
    for (name, profile) in &mut config.profiles {
        if let Err(e) = profile.validate() {
            warnings.push(format!("[{name}] {e}; using defaults for that profile"));
            *profile = Profile::default();
        }
    }
    config.profiles.entry(DEFAULT_PROFILE.to_owned()).or_default();
    (config, warnings)
}

/// Apply one legacy key (also used for `--key value` command line overrides).
pub fn apply_legacy_key(profile: &mut Profile, key: &str, value: &str) -> Result<(), String> {
    fn num<T: std::str::FromStr>(v: &str) -> Result<T, String> {
        v.trim()
            .parse()
            .map_err(|_| format!("{v:?} is not a valid number"))
    }
    fn boolean(v: &str) -> Result<bool, String> {
        parse_bool(v).ok_or_else(|| format!("{v:?} is not on/off/true/false"))
    }
    fn color(c: &mut Rgba, part: &str, v: &str) -> Result<(), String> {
        let x: f64 = num(v)?;
        match part {
            "red" => c.red = x,
            "green" => c.green = x,
            "blue" => c.blue = x,
            "alpha" => c.alpha = x,
            _ => return Err("unknown colour component".into()),
        }
        Ok(())
    }

    let key = key.trim().to_ascii_lowercase().replace('-', "_");
    if let Some(part) = key.strip_prefix("term_foreground_") {
        return color(&mut profile.terminal.foreground, part, value);
    }
    if let Some(part) = key.strip_prefix("term_background_") {
        return color(&mut profile.terminal.background, part, value);
    }
    let t = &mut profile.terminal;
    match key.as_str() {
        "port" => value.clone_into(&mut profile.connection.serial_mut().port),
        "baudrate" | "speed" => profile.connection.serial_mut().baudrate = num(value)?,
        "bits" => profile.connection.serial_mut().bits = num(value)?,
        "stopbits" => profile.connection.serial_mut().stopbits = num(value)?,
        "parity" => {
            profile.connection.serial_mut().parity =
                Parity::parse(value).ok_or_else(|| format!("unknown parity {value:?}"))?;
        }
        "flow_control" | "flow" => {
            profile.connection.serial_mut().flow_control =
                FlowControl::parse(value).ok_or_else(|| format!("unknown flow control {value:?}"))?;
        }
        "rs485_rts_time_before_tx" => profile.connection.serial_mut().rs485_rts_time_before_tx = num(value)?,
        "rs485_rts_time_after_tx" => profile.connection.serial_mut().rs485_rts_time_after_tx = num(value)?,
        "disable_port_lock" => profile.connection.serial_mut().disable_port_lock = boolean(value)?,
        "term_echo" | "echo" => t.echo = boolean(value)?,
        "term_auto_lf" => t.auto_lf = boolean(value)?,
        "term_auto_cr" => t.auto_cr = boolean(value)?,
        "crlfauto" => {
            let b = boolean(value)?;
            t.auto_lf = b;
            t.auto_cr = b;
        }
        "term_show_timestamp" | "timestamp" => t.timestamps = boolean(value)?,
        "term_font" | "font" => value.clone_into(&mut t.font),
        "term_block_cursor" => t.block_cursor = boolean(value)?,
        "term_show_cursor" => t.show_cursor = boolean(value)?,
        "term_rows" => t.rows = num(value)?,
        "term_columns" => t.columns = num(value)?,
        "term_scrollback" => t.scrollback = num(value)?,
        "term_index" => t.show_index = boolean(value)?,
        "term_view_mode" => {
            t.view_mode = if value.eq_ignore_ascii_case("ascii") {
                ViewMode::Ascii
            } else {
                ViewMode::Hex
            };
        }
        "term_hex_chars" => {
            let n: u8 = num(value)?;
            t.hex_chars = if (8..=64).contains(&n) { n } else { 32 };
        }
        "term_visual_bell" => t.visual_bell = boolean(value).unwrap_or(false),
        "term_wait_delay" | "wait_delay" => t.wait_delay_ms = num(value)?,
        "term_wait_char" | "wait_char" => {
            let n: i32 = num(value)?;
            t.wait_char = u8::try_from(n).ok();
        }
        "term_raw_filename" => t.raw_filename = (!value.is_empty()).then(|| value.to_owned()),
        "macros" => {} // legacy macro lists are imported by the macro store
        _ => return Err("unknown key, ignored".into()),
    }
    // Old files carry the swapped C defaults (rows=80, columns=25); repair that obvious case.
    if key == "term_columns" && t.rows == 80 && t.columns == 25 {
        t.rows = 25;
        t.columns = 80;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upstream GTKTerm 1.x file found on the developer machine.
    const UPSTREAM_1X: &str = "[default]\nport\t= /dev/ttyS0\nspeed\t= 115200\nbits\t= 8\nstopbits\t= 1\n\
        parity\t= none\nflow\t= none\nwait_delay\t= 0\nwait_char\t= -1\nrs485_rts_time_before_tx\t= 30\n\
        rs485_rts_time_after_tx\t= 30\necho\t= False\ncrlfauto\t= False\ntimestamp\t= False\n\
        font\t= \"Monospace 12\"\nterm_block_cursor\t= True\nterm_rows\t= 80\nterm_columns\t= 25\n\
        term_scrollback\t= 10000\nterm_visual_bell\t= True\nterm_foreground_red\t= 0.660000\n\
        term_foreground_blue\t= 0.660000\nterm_foreground_green\t= 0.660000\nterm_foreground_alpha\t= 1.000000\n\
        term_background_red\t= 0.000000\nterm_background_blue\t= 0.000000\nterm_background_green\t= 0.000000\n\
        term_background_alpha\t= 1.000000\n";

    /// Keys written by the C fork (GKeyFile).
    const C_FORK: &str = "[default]\nport=/dev/ttyUSB1\nbaudrate=921600\nbits=7\nstopbits=2\nparity=even\n\
        flow_control=rs485\nrs485_rts_time_before_tx=5\nrs485_rts_time_after_tx=7\ndisable_port_lock=true\n\
        term_echo=true\nterm_auto_lf=true\nterm_auto_cr=false\nterm_show_timestamp=true\nterm_font=Hack 10\n\
        term_view_mode=HEX\nterm_hex_chars=99\nterm_wait_char=13\nmacros=F1;hello;\n\n[lab]\nport=/dev/ttyACM0\n";

    #[test]
    fn imports_upstream_1x_keyfile() {
        let (cfg, warnings) = import_legacy_keyfile(UPSTREAM_1X);
        assert!(warnings.is_empty(), "{warnings:?}");
        let p = cfg.profile(DEFAULT_PROFILE).unwrap();
        assert_eq!(p.connection, Connection::Serial(SerialConfig::default()));
        let t = &p.terminal;
        assert_eq!(
            (t.rows, t.columns),
            (25, 80),
            "swapped legacy defaults are repaired"
        );
        assert!(t.visual_bell && t.block_cursor && !t.echo && !t.auto_cr);
        assert_eq!(t.font, "Monospace 12");
        assert_eq!(t.wait_char, None);
        assert_eq!(t.foreground, Rgba::new(0.66, 0.66, 0.66, 1.0));
    }

    #[test]
    fn imports_c_fork_keyfile() {
        let (cfg, warnings) = import_legacy_keyfile(C_FORK);
        assert!(warnings.is_empty(), "{warnings:?}");
        let p = cfg.profile(DEFAULT_PROFILE).unwrap();
        let s = p.connection.serial().unwrap();
        assert_eq!(
            (s.baudrate, s.bits, s.stopbits, s.parity),
            (921_600, 7, 2, Parity::Even)
        );
        assert_eq!(s.flow_control, FlowControl::Rs485);
        assert!(s.disable_port_lock);
        assert!(p.terminal.echo && p.terminal.auto_lf && p.terminal.timestamps);
        assert_eq!(p.terminal.view_mode, ViewMode::Hex);
        assert_eq!(
            p.terminal.hex_chars, 32,
            "out-of-range hex width falls back like the C version"
        );
        assert_eq!(p.terminal.wait_char, Some(13));
        assert_eq!(
            cfg.profile("lab").unwrap().connection.serial().unwrap().port,
            "/dev/ttyACM0"
        );
    }

    #[test]
    fn import_warns_but_keeps_going() {
        let (cfg, warnings) = import_legacy_keyfile("[a]\nbits=12\nfoo=1\nbaudrate=abc\n[b.c]\nport=x\n");
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert_eq!(
            cfg.profile("a").unwrap(),
            &Profile::default(),
            "invalid profile reset to defaults"
        );
        assert!(cfg.profile("default").is_ok());
    }

    #[test]
    fn toml_roundtrip_all_connection_types() {
        let mut cfg = Config::default();
        cfg.profiles.insert(
            "tcp".into(),
            Profile {
                connection: Connection::TcpClient {
                    host: "::1".into(),
                    port: 5000,
                    connect_timeout_ms: 100,
                },
                ..Default::default()
            },
        );
        cfg.profiles.insert(
            "srv".into(),
            Profile {
                connection: Connection::TcpServer {
                    bind: "::".into(),
                    port: 7,
                },
                ..Default::default()
            },
        );
        cfg.profiles.insert(
            "udp".into(),
            Profile {
                connection: Connection::Udp {
                    bind: "0.0.0.0".into(),
                    local_port: 9000,
                    remote_host: Some("localhost".into()),
                    remote_port: 9001,
                },
                terminal: TerminalConfig {
                    view_mode: ViewMode::DecimalMixed,
                    wait_char: Some(b'>'),
                    ..Default::default()
                },
                decoder: crate::decoding::DecoderConfig {
                    mode: crate::decoding::DecoderMode::Protocol(
                        gtkterm_protocols::decode::Protocol::ModbusRtu,
                    ),
                    signals: true,
                },
            },
        );
        let text = cfg.to_toml().unwrap();
        assert!(text.contains("mode = \"modbus_rtu\""), "{text}");
        assert_eq!(Config::from_toml(&text, Path::new("x")).unwrap(), cfg);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let cfg = Config::from_toml(
            "[profiles.x.connection]\ntype = \"serial\"\nport = \"/dev/ttyUSB0\"\n",
            Path::new("x"),
        )
        .unwrap();
        let p = cfg.profile("x").unwrap();
        assert_eq!(p.connection.serial().unwrap().baudrate, 115_200);
        assert_eq!(p.terminal, TerminalConfig::default());
        assert!(cfg.profile(DEFAULT_PROFILE).is_ok());
    }

    #[test]
    fn save_load_and_first_run_import() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let legacy = dir.path().join(".gtktermrc");
        std::fs::write(&legacy, UPSTREAM_1X).unwrap();
        let (cfg, msg) = load_or_import(&path, Some(&legacy)).unwrap();
        assert!(msg.unwrap().starts_with("Imported 1 profile(s)"));
        assert!(path.exists());
        let (again, msg) = load_or_import(&path, Some(&legacy)).unwrap();
        assert_eq!((again, msg), (cfg, None));
    }

    #[test]
    fn remove_default_resets_it() {
        let mut cfg = Config::default();
        cfg.profile_mut(DEFAULT_PROFILE).unwrap().terminal.echo = true;
        cfg.remove_profile(DEFAULT_PROFILE).unwrap();
        assert!(!cfg.profile(DEFAULT_PROFILE).unwrap().terminal.echo);
        assert!(matches!(
            cfg.remove_profile("nope"),
            Err(ConfigError::NoSuchProfile(_))
        ));
    }

    #[test]
    fn validation() {
        let mut p = Profile::default();
        p.connection.serial_mut().bits = 9;
        assert!(matches!(
            p.validate(),
            Err(ConfigError::Invalid { field: "bits", .. })
        ));
        let mut p = Profile::default();
        p.terminal.wait_delay_ms = 501;
        assert!(p.validate().is_err());
        assert!(check_profile_name("a.b").is_err());
        assert!(check_profile_name(&"x".repeat(32)).is_err());
        assert!(check_profile_name("lab bench").is_ok());
    }

    #[test]
    fn describe_layout() {
        let text = Profile::default().describe("default");
        assert!(text.starts_with(
            "Configuration [default]\n\nSerial port settings:\nport                     : /dev/ttyS0\n"
        ));
        assert!(text.contains("\nbaudrate                 : 115200\n"));
        assert!(text.contains("\nTerminal settings:\nterm_font                : Monospace 12\n"));
    }

    #[test]
    fn summaries() {
        assert_eq!(Connection::default().summary(), "/dev/ttyS0 115200-8-N-1");
        let udp = Connection::Udp {
            bind: "0.0.0.0".into(),
            local_port: 1,
            remote_host: None,
            remote_port: 0,
        };
        assert_eq!(udp.summary(), "UDP:0.0.0.0:1->*:0");
    }
}
