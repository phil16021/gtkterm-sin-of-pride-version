//! Command line interface.
//!
//! Long options and short letters follow the C GTKTerm (`--port/-p`, `--baudrate/-s`,
//! `--term_echo/-e on|off`, `--show_config/-V`, ...). `-h` is help, so auto-LF moved to `-j`.
//! Kebab-case aliases (`--flow-control`) are accepted as well.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{ArgAction, Parser};
use gtkterm_core::config::{
    Config, ConfigError, Connection, DEFAULT_PROFILE, Profile, apply_legacy_key, config_path,
    legacy_keyfile_path, load_or_import,
};
use gtkterm_protocols::logfmt::LogFormat;

fn on_off(s: &str) -> Result<String, String> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok("true".into()),
        "off" | "false" | "no" | "0" => Ok("false".into()),
        _ => Err(format!("{s:?} is not on/off")),
    }
}

fn log_format(s: &str) -> Result<LogFormat, String> {
    LogFormat::parse(s).ok_or_else(|| format!("unknown log format {s:?} (raw, text, csv, json, xml)"))
}

/// Parse `host:port`.
fn host_port(s: &str) -> Result<(String, u16), String> {
    let (host, port) = s
        .rsplit_once(':')
        .ok_or_else(|| format!("{s:?} is not host:port"))?;
    let port = port.parse().map_err(|_| format!("bad port in {s:?}"))?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err(format!("missing host in {s:?}"));
    }
    Ok((host.to_owned(), port))
}

/// Parse `[bind:]port` for the MCP HTTP endpoint (defaults to 127.0.0.1).
fn mcp_addr(s: &str) -> Result<SocketAddr, String> {
    if let Ok(port) = s.parse::<u16>() {
        return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
    }
    s.parse().map_err(|_| format!("{s:?} is not PORT or ADDR:PORT"))
}

/// Parse a UDP spec `[local_port]->host:port` or `local_port`.
fn udp_spec(s: &str) -> Result<Connection, String> {
    let (local, remote) = match s.split_once("->") {
        Some((l, r)) => (l, Some(r)),
        None => (s, None),
    };
    let local_port = if local.is_empty() {
        0
    } else {
        local.parse().map_err(|_| format!("bad local port {local:?}"))?
    };
    let (remote_host, remote_port) = match remote {
        Some(r) => {
            let (h, p) = host_port(r)?;
            (Some(h), p)
        }
        None => (None, 0),
    };
    Ok(Connection::Udp {
        bind: "0.0.0.0".into(),
        local_port,
        remote_host,
        remote_port,
        max_datagram: 0,
    })
}

/// GTKTerm Sunshine — serial/TCP/UDP terminal with live plotting and an MCP server.
#[derive(Debug, Parser)]
#[command(
    name = "gtkterm-sunshine",
    version,
    disable_version_flag = true,
    max_term_width = 100
)]
pub struct Cli {
    /// Print version information.
    #[arg(long, action = ArgAction::Version)]
    version: Option<bool>,

    // ----- serial port (legacy GTKTerm options) -----
    /// Serial device, e.g. /dev/ttyUSB0.
    #[arg(short = 'p', long, help_heading = "Port")]
    pub port: Option<String>,
    /// Baud rate (any value; non-standard rates use termios2).
    #[arg(short = 's', long, visible_alias = "speed", help_heading = "Port")]
    pub baudrate: Option<u32>,
    /// Data bits (5-8).
    #[arg(short = 'b', long, help_heading = "Port")]
    pub bits: Option<u8>,
    /// Stop bits (1 or 2).
    #[arg(short = 't', long, help_heading = "Port")]
    pub stopbits: Option<u8>,
    /// Parity: none, odd, even, mark, space.
    #[arg(short = 'a', long, help_heading = "Port")]
    pub parity: Option<String>,
    /// Flow control: none, xon, rts, rs485.
    #[arg(
        short = 'w',
        long = "flow_control",
        visible_alias = "flow-control",
        help_heading = "Port"
    )]
    pub flow_control: Option<String>,
    /// RS485: ms between asserting RTS and sending.
    #[arg(
        short = 'x',
        long = "rs485_rts_time_before_tx",
        visible_alias = "rs485-rts-time-before-tx",
        help_heading = "Port"
    )]
    pub rs485_before: Option<u32>,
    /// RS485: ms between the last byte and releasing RTS.
    #[arg(
        short = 'y',
        long = "rs485_rts_time_after_tx",
        visible_alias = "rs485-rts-time-after-tx",
        help_heading = "Port"
    )]
    pub rs485_after: Option<u32>,
    /// Do not lock the device (on/off).
    #[arg(short = 'l', long = "disable_port_lock", visible_alias = "disable-port-lock", value_parser = on_off, help_heading = "Port")]
    pub disable_port_lock: Option<String>,

    // ----- network -----
    /// Connect to a TCP server: HOST:PORT.
    #[arg(long, value_parser = host_port, conflicts_with_all = ["tcp_listen", "udp"], help_heading = "Network")]
    pub tcp: Option<(String, u16)>,
    /// Listen for TCP clients on PORT.
    #[arg(long = "tcp-listen", conflicts_with = "udp", help_heading = "Network")]
    pub tcp_listen: Option<u16>,
    /// UDP socket: LOCAL_PORT, or [LOCAL_PORT]->HOST:PORT.
    #[arg(long, value_parser = udp_spec, help_heading = "Network")]
    pub udp: Option<Connection>,

    // ----- terminal -----
    /// End-of-line delay in ms (0-500).
    #[arg(
        short = 'd',
        long = "term_wait_delay",
        visible_alias = "term-wait-delay",
        help_heading = "Terminal"
    )]
    pub wait_delay: Option<u32>,
    /// Wait for this byte (decimal) after each line.
    #[arg(
        short = 'r',
        long = "term_wait_char",
        visible_alias = "term-wait-char",
        help_heading = "Terminal"
    )]
    pub wait_char: Option<i32>,
    /// Local echo (on/off).
    #[arg(short = 'e', long = "term_echo", visible_alias = "term-echo", value_parser = on_off, help_heading = "Terminal")]
    pub echo: Option<String>,
    /// Default file for "send raw file".
    #[arg(
        short = 'f',
        long = "term_raw_filename",
        visible_alias = "term-raw-filename",
        help_heading = "Terminal"
    )]
    pub raw_filename: Option<String>,
    /// Terminal rows.
    #[arg(
        short = 'o',
        long = "term_rows",
        visible_alias = "term-rows",
        help_heading = "Terminal"
    )]
    pub rows: Option<u16>,
    /// Terminal columns.
    #[arg(
        short = 'c',
        long = "term_columns",
        visible_alias = "term-columns",
        help_heading = "Terminal"
    )]
    pub columns: Option<u16>,
    /// Insert CR before a bare LF (on/off).
    #[arg(short = 'g', long = "term_auto_cr", visible_alias = "term-auto-cr", value_parser = on_off, help_heading = "Terminal")]
    pub auto_cr: Option<String>,
    /// Insert LF after a bare CR (on/off). Was -h in GTKTerm 1.x.
    #[arg(short = 'j', long = "term_auto_lf", visible_alias = "term-auto-lf", value_parser = on_off, help_heading = "Terminal")]
    pub auto_lf: Option<String>,

    // ----- configuration -----
    /// Use this profile (must exist unless options create it).
    #[arg(short = 'U', long = "use_config", visible_aliases = ["use-config", "profile"], help_heading = "Configuration")]
    pub use_config: Option<String>,
    /// Print a profile and exit.
    #[arg(
        short = 'V',
        long = "show_config",
        visible_alias = "show-config",
        help_heading = "Configuration"
    )]
    pub show_config: Option<String>,
    /// Save the profile including the options given on this command line, then exit.
    #[arg(
        short = 'S',
        long = "save_config",
        visible_alias = "save-config",
        help_heading = "Configuration"
    )]
    pub save_config: bool,
    /// List profiles and exit.
    #[arg(
        short = 'L',
        long = "list_config",
        visible_alias = "list-config",
        help_heading = "Configuration"
    )]
    pub list_config: bool,
    /// Remove a profile and exit.
    #[arg(
        short = 'R',
        long = "remove_config",
        visible_alias = "remove-config",
        help_heading = "Configuration"
    )]
    pub remove_config: Option<String>,
    /// Configuration file (default: ~/.config/gtkterm-sunshine/config.toml).
    #[arg(long = "config", help_heading = "Configuration")]
    pub config_file: Option<PathBuf>,

    // ----- modes -----
    /// Run without a window: received data goes to stdout, stdin is sent.
    #[arg(long, help_heading = "Modes")]
    pub headless: bool,
    /// With --headless: print received data as a hex dump.
    #[arg(long, requires = "headless", help_heading = "Modes")]
    pub hexdump: bool,
    /// Run the MCP server on stdin/stdout (for `claude mcp add`), without a window.
    #[arg(long = "mcp-stdio", conflicts_with = "headless", help_heading = "MCP")]
    pub mcp_stdio: bool,
    /// Serve MCP over Streamable HTTP on [ADDR:]PORT (default address 127.0.0.1).
    #[arg(long = "mcp-http", value_parser = mcp_addr, help_heading = "MCP")]
    pub mcp_http: Option<SocketAddr>,
    /// Only expose read-only MCP tools.
    #[arg(long = "mcp-read-only", help_heading = "MCP")]
    pub mcp_read_only: bool,
    /// Allow `exec` in scripts run through MCP.
    #[arg(long = "mcp-allow-exec", help_heading = "MCP")]
    pub mcp_allow_exec: bool,
    /// Run this automation script once the port is open (with --headless, exit with its result).
    #[arg(long = "script", value_name = "FILE", help_heading = "Automation")]
    pub script: Option<PathBuf>,
    /// Allow `exec` in scripts started from the command line or the window.
    #[arg(long = "allow-exec", help_heading = "Automation")]
    pub allow_exec: bool,
    /// Let MCP `file_send` read files from DIR as well as the transfer directory (repeatable).
    #[arg(long = "mcp-allow-dir", value_name = "DIR", help_heading = "MCP")]
    pub mcp_allow_dir: Vec<PathBuf>,
    /// Write tool calls an agent may make per second (bursts of twice that); 0: no limit.
    #[arg(long = "mcp-rate", value_name = "N", help_heading = "MCP")]
    pub mcp_rate: Option<u32>,
    /// Bearer token for --mcp-http (default: random, printed at start).
    #[arg(long = "mcp-token", env = "GTKTERM_MCP_TOKEN", help_heading = "MCP")]
    pub mcp_token: Option<String>,

    // ----- logging -----
    /// Log the session to this file.
    #[arg(long = "log-file", help_heading = "Logging")]
    pub log_file: Option<PathBuf>,
    /// Log format: raw, text, csv, json, xml (default: from the file extension, else raw).
    #[arg(long = "log-format", value_parser = log_format, help_heading = "Logging")]
    pub log_format: Option<LogFormat>,
    /// More diagnostics on stderr (-v info, -vv debug, -vvv trace). RUST_LOG overrides.
    #[arg(short = 'v', long, action = ArgAction::Count, help_heading = "Logging")]
    pub verbose: u8,
    /// Append diagnostics as JSON lines (with session and port spans) to this file; the
    /// level is debug for GTKTerm, GTKTERM_TRACE overrides.
    #[arg(long = "trace-file", value_name = "FILE", help_heading = "Logging")]
    pub trace_file: Option<PathBuf>,

    /// Write the man page into DIR and exit (packaging).
    #[arg(long = "generate-man", hide = true, value_name = "DIR")]
    pub generate_man: Option<PathBuf>,
    /// Print the MCP tools, prompts and resources with their JSON schemas and exit.
    #[arg(long = "mcp-dump-schema", help_heading = "MCP")]
    pub mcp_dump_schema: bool,
    /// Print shell completions (bash, zsh, fish, elvish, powershell) and exit (packaging).
    #[arg(long = "generate-completions", hide = true, value_name = "SHELL")]
    pub generate_completions: Option<clap_complete::Shell>,
}

/// Man page sections clap cannot generate.
const MAN_EXTRA: &str = r#".SH "KEYBOARD"
Plain Ctrl+letter keys go to the device. Application shortcuts use Ctrl+Shift or function keys:
F5/F6 open/close the port, F7/F8 toggle DTR/RTS, Ctrl+Shift+B sends a break, Ctrl+Shift+H opens
hex send, Ctrl+Shift+J the script editor, Ctrl+Shift+M the signal monitor and Ctrl+? lists all of them.
.SH SIGNALS
.TP
.B SIGUSR1
Open (or reopen) the configured port.
.TP
.B SIGUSR2
Close the port.
.TP
.BR SIGINT ", " SIGTERM
Stop logging, close the port and exit (\fB\-\-headless\fR).
.SH FILES
.TP
.I $XDG_CONFIG_HOME/gtkterm\-sunshine/config.toml
Profiles (\fB[profiles.<name>]\fR) and settings. On first start \fI~/.config/.gtktermrc\fR of
the C GTKTerm is imported.
.TP
.I $XDG_CONFIG_HOME/gtkterm\-sunshine/macros.json
Macros; \fI~/.config/gtkterm/macros.json\fR is imported on first start.
.TP
.I $XDG_CONFIG_HOME/gtkterm\-sunshine/state.toml
Window state, recent choices and the embedded MCP server settings (including its token; mode 0600).
.TP
.I $XDG_DATA_HOME/gtkterm\-sunshine/logs/
Default directory of session logs.
.TP
.I $XDG_DATA_HOME/gtkterm\-sunshine/transfers/
Default directory of received files and of files agents may send.
.TP
.I /usr/share/gtkterm\-sunshine/examples/
Example scripts (under the installation prefix).
.SH ENVIRONMENT
.TP
.B RUST_LOG
Diagnostics filter, e.g. \fBgtkterm_core=debug\fR; overrides \fB\-v\fR.
.TP
.B GTKTERM_MCP_TOKEN
Bearer token of \fB\-\-mcp\-http\fR.
.TP
.B GTKTERM_SUNSHINE_CONFIG_DIR
Use another configuration directory.
.SH "EXIT STATUS"
0 on success, 1 on runtime errors (including a failed \fB\-\-script\fR in \fB\-\-headless\fR
mode), 2 on invalid options.
.SH EXAMPLES
.TP
.B gtkterm\-sunshine \-p /dev/ttyUSB0 \-s 115200
Open the GUI on a USB serial adapter.
.TP
.B gtkterm\-sunshine \-\-tcp 192.168.1.50:23
Connect to a TCP endpoint (\fB\-\-tcp\-listen 5000\fR accepts clients instead).
.TP
.B gtkterm\-sunshine \-\-headless \-p /dev/ttyACM0 \-\-log\-file run.csv
Print received data on stdout, send stdin, log as CSV.
.TP
.B gtkterm\-sunshine \-\-headless \-p /dev/ttyUSB0 \-\-script probe.script
Run a script and exit with its result.
.TP
.B claude mcp add gtkterm \-\- gtkterm\-sunshine \-\-mcp\-stdio
Register the MCP server with Claude Code.
.SH "SEE ALSO"
.BR gtkterm (1),
.BR socat (1),
.BR stty (1),
.BR sz (1),
.BR rz (1)
"#;

/// Handle the packaging flags; returns true if one was given.
pub fn packaging(cli: &Cli) -> std::io::Result<bool> {
    use clap::CommandFactory;
    if let Some(dir) = &cli.generate_man {
        std::fs::create_dir_all(dir)?;
        let man = clap_mangen::Man::new(Cli::command());
        let mut buf = Vec::new();
        man.render(&mut buf)?;
        buf.extend_from_slice(MAN_EXTRA.as_bytes());
        std::fs::write(dir.join("gtkterm-sunshine.1"), buf)?;
        return Ok(true);
    }
    if cli.mcp_dump_schema {
        let manager =
            gtkterm_core::manager::SessionManager::new(gtkterm_core::config::Config::default(), None);
        let server = gtkterm_mcp::GtkTermMcp::new(manager, gtkterm_mcp::ServerOptions::default());
        let json = serde_json::to_string_pretty(&server.schema()).map_err(std::io::Error::other)?;
        println!("{json}");
        return Ok(true);
    }
    if let Some(shell) = cli.generate_completions {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "gtkterm-sunshine",
            &mut std::io::stdout(),
        );
        return Ok(true);
    }
    Ok(false)
}

/// A configuration operation that ends the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOp {
    /// `--show_config NAME`
    Show(String),
    /// `--list_config`
    List,
    /// `--save_config`
    Save,
    /// `--remove_config NAME`
    Remove(String),
}

/// Everything resolved from the command line and the config file.
#[derive(Debug)]
pub struct Resolved {
    /// Loaded (and possibly imported) configuration.
    pub config: Config,
    /// Where it lives.
    pub config_path: PathBuf,
    /// Message about a legacy import, if one happened.
    pub import_message: Option<String>,
    /// Selected profile name.
    pub profile_name: String,
    /// Selected profile with command line overrides applied.
    pub profile: Profile,
    /// True if the command line chose a connection explicitly (port, --tcp, ...).
    pub explicit_connection: bool,
    /// Requested config operation.
    pub op: Option<ConfigOp>,
    /// Macro store and where it is saved.
    pub macros: (gtkterm_core::macros::MacroStore, PathBuf),
}

impl Cli {
    fn overrides(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        let mut push = |k: &'static str, val: Option<String>| {
            if let Some(val) = val {
                v.push((k, val));
            }
        };
        push("port", self.port.clone());
        push("baudrate", self.baudrate.map(|x| x.to_string()));
        push("bits", self.bits.map(|x| x.to_string()));
        push("stopbits", self.stopbits.map(|x| x.to_string()));
        push("parity", self.parity.clone());
        push("flow_control", self.flow_control.clone());
        push(
            "rs485_rts_time_before_tx",
            self.rs485_before.map(|x| x.to_string()),
        );
        push("rs485_rts_time_after_tx", self.rs485_after.map(|x| x.to_string()));
        push("disable_port_lock", self.disable_port_lock.clone());
        push("term_wait_delay", self.wait_delay.map(|x| x.to_string()));
        push("term_wait_char", self.wait_char.map(|x| x.to_string()));
        push("term_echo", self.echo.clone());
        push("term_raw_filename", self.raw_filename.clone());
        push("term_rows", self.rows.map(|x| x.to_string()));
        push("term_columns", self.columns.map(|x| x.to_string()));
        push("term_auto_cr", self.auto_cr.clone());
        push("term_auto_lf", self.auto_lf.clone());
        v
    }

    fn config_op(&self) -> Option<ConfigOp> {
        if let Some(n) = &self.show_config {
            Some(ConfigOp::Show(n.clone()))
        } else if self.list_config {
            Some(ConfigOp::List)
        } else if let Some(n) = &self.remove_config {
            Some(ConfigOp::Remove(n.clone()))
        } else if self.save_config {
            Some(ConfigOp::Save)
        } else {
            None
        }
    }

    /// Load the configuration and apply the command line to the selected profile.
    pub fn resolve(&self) -> Result<Resolved, String> {
        let config_path = self.config_file.clone().unwrap_or_else(config_path);
        let legacy = if self.config_file.is_some() {
            None
        } else {
            legacy_keyfile_path()
        };
        let (config, mut import_message) =
            load_or_import(&config_path, legacy.as_deref()).map_err(|e| e.to_string())?;
        let (macros_path, c_macros) = match &self.config_file {
            Some(f) => (f.with_file_name("macros.json"), None),
            None => (
                gtkterm_core::macros::macros_path(),
                gtkterm_core::macros::c_macros_path(),
            ),
        };
        let (macro_store, macro_message) =
            gtkterm_core::macros::load_or_import(&macros_path, c_macros.as_deref(), legacy.as_deref())
                .map_err(|e| e.to_string())?;
        if let Some(m) = macro_message {
            import_message = Some(import_message.map_or(m.clone(), |prev| format!("{prev}\n{m}")));
        }
        let profile_name = self
            .use_config
            .clone()
            .unwrap_or_else(|| DEFAULT_PROFILE.to_owned());
        gtkterm_core::config::check_profile_name(&profile_name).map_err(|e| e.to_string())?;
        let overrides = self.overrides();
        let mut profile = match config.profile(&profile_name) {
            Ok(p) => p.clone(),
            Err(ConfigError::NoSuchProfile(_)) if !overrides.is_empty() || self.save_config => {
                Profile::default()
            }
            Err(e) => return Err(e.to_string()),
        };
        for (key, value) in &overrides {
            apply_legacy_key(&mut profile, key, value).map_err(|e| format!("--{key}: {e}"))?;
        }
        let mut explicit_connection = self.port.is_some();
        if let Some((host, port)) = &self.tcp {
            profile.connection = Connection::TcpClient {
                host: host.clone(),
                port: *port,
                connect_timeout_ms: 5_000,
            };
            explicit_connection = true;
        }
        if let Some(port) = self.tcp_listen {
            profile.connection = Connection::TcpServer {
                bind: "0.0.0.0".into(),
                port,
            };
            explicit_connection = true;
        }
        if let Some(udp) = &self.udp {
            profile.connection = udp.clone();
            explicit_connection = true;
        }
        profile.validate().map_err(|e| e.to_string())?;
        Ok(Resolved {
            config,
            config_path,
            import_message,
            profile_name,
            profile,
            explicit_connection,
            op: self.config_op(),
            macros: (macro_store, macros_path),
        })
    }

    /// The log format to use for `--log-file`.
    pub fn effective_log_format(&self) -> LogFormat {
        self.log_format.unwrap_or_else(|| {
            self.log_file
                .as_ref()
                .and_then(|p| p.extension())
                .and_then(|e| LogFormat::parse(&e.to_string_lossy()))
                .unwrap_or(LogFormat::Raw)
        })
    }
}

/// Run a configuration operation; returns the text to print.
pub fn run_config_op(mut r: Resolved) -> Result<String, String> {
    let Some(op) = r.op.clone() else {
        return Ok(String::new());
    };
    match op {
        ConfigOp::Show(name) => {
            let profile = if name == r.profile_name {
                r.profile.clone()
            } else {
                r.config.profile(&name).map_err(|e| e.to_string())?.clone()
            };
            Ok(profile.describe(&name))
        }
        ConfigOp::List => Ok(r.config.list_text()),
        ConfigOp::Remove(name) => {
            r.config.remove_profile(&name).map_err(|e| e.to_string())?;
            r.config.save(&r.config_path).map_err(|e| e.to_string())?;
            Ok(if name == DEFAULT_PROFILE {
                format!("Profile [{name}] reset to defaults\n")
            } else {
                format!("Profile [{name}] removed\n")
            })
        }
        ConfigOp::Save => {
            *r.config.profile_mut(&r.profile_name).map_err(|e| e.to_string())? = r.profile.clone();
            r.config.save(&r.config_path).map_err(|e| e.to_string())?;
            Ok(format!(
                "Profile [{}] saved to {}\n",
                r.profile_name,
                r.config_path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("gtkterm-sunshine").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn legacy_short_options() {
        let cli = parse(&[
            "-p",
            "/dev/ttyACM0",
            "-s",
            "9600",
            "-b",
            "7",
            "-t",
            "2",
            "-a",
            "even",
            "-w",
            "RTS",
            "-e",
            "on",
            "-g",
            "off",
            "-j",
            "on",
        ]);
        let ov = cli.overrides();
        assert!(ov.contains(&("port", "/dev/ttyACM0".into())));
        assert!(ov.contains(&("term_echo", "true".into())));
        assert!(ov.contains(&("term_auto_lf", "true".into())));
        assert!(ov.contains(&("flow_control", "RTS".into())));
    }

    #[test]
    fn kebab_aliases_and_network_specs() {
        let cli = parse(&[
            "--flow-control",
            "xon",
            "--term-echo",
            "off",
            "--tcp",
            "[::1]:5000",
        ]);
        assert_eq!(cli.flow_control.as_deref(), Some("xon"));
        assert_eq!(cli.tcp, Some(("::1".into(), 5000)));
        let cli = parse(&["--udp", "9000->localhost:9001"]);
        assert_eq!(
            cli.udp,
            Some(Connection::Udp {
                bind: "0.0.0.0".into(),
                local_port: 9000,
                remote_host: Some("localhost".into()),
                remote_port: 9001,
                max_datagram: 0,
            })
        );
        assert!(
            Cli::try_parse_from(["x", "--tcp", "h:1", "--udp", "1"]).is_err(),
            "connection options conflict"
        );
        assert_eq!(
            parse(&["--mcp-http", "8765"]).mcp_http,
            Some("127.0.0.1:8765".parse().unwrap())
        );
        assert!(Cli::try_parse_from(["x", "--term_echo", "maybe"]).is_err());
    }

    #[test]
    fn config_ops_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        let cfg_s = cfg.to_str().unwrap();

        let r = parse(&[
            "--config",
            cfg_s,
            "-U",
            "lab",
            "-p",
            "/dev/ttyUSB3",
            "-s",
            "57600",
            "-S",
        ])
        .resolve()
        .unwrap();
        assert!(run_config_op(r).unwrap().starts_with("Profile [lab] saved"));

        let r = parse(&["--config", cfg_s, "-L"]).resolve().unwrap();
        assert_eq!(
            run_config_op(r).unwrap(),
            "Configurations found:\n\n[default]\n[lab]\n"
        );

        let r = parse(&["--config", cfg_s, "-V", "lab"]).resolve().unwrap();
        let shown = run_config_op(r).unwrap();
        assert!(
            shown.contains("port                     : /dev/ttyUSB3"),
            "{shown}"
        );
        assert!(shown.contains("baudrate                 : 57600"));

        let r = parse(&["--config", cfg_s, "-U", "lab"]).resolve().unwrap();
        assert!(!r.explicit_connection);
        assert_eq!(r.profile.connection.serial().unwrap().baudrate, 57_600);

        let r = parse(&["--config", cfg_s, "-R", "lab"]).resolve().unwrap();
        run_config_op(r).unwrap();
        assert!(
            parse(&["--config", cfg_s, "-U", "lab"]).resolve().is_err(),
            "removed profile"
        );
    }

    #[test]
    fn invalid_values_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("c.toml");
        let err = parse(&["--config", cfg.to_str().unwrap(), "-b", "9"])
            .resolve()
            .unwrap_err();
        assert!(err.contains("bits"), "{err}");
        let err = parse(&["--config", cfg.to_str().unwrap(), "-a", "weird"])
            .resolve()
            .unwrap_err();
        assert!(err.contains("parity"), "{err}");
    }

    #[test]
    fn log_format_from_extension() {
        assert_eq!(
            parse(&["--log-file", "x.csv"]).effective_log_format(),
            LogFormat::Csv
        );
        assert_eq!(
            parse(&["--log-file", "x.bin", "--log-format", "json"]).effective_log_format(),
            LogFormat::Json
        );
        assert_eq!(parse(&["--log-file", "x"]).effective_log_format(), LogFormat::Raw);
    }

    #[test]
    fn clap_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
