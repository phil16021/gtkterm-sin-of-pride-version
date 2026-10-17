//! Automation scripts: the GTKTerm script language, compatible with the C version's files.
//!
//! ```text
//! # comment (also indented)
//! log Resetting device...
//! set_dtr 0
//! delay 100            # ms; `100ms`, `1.5s` work too
//! set_dtr 1
//! send AT\r\n          # escapes \r \n \t \0 \e \\ \xHH; ${var} is expanded
//! expect \+CSQ: (\d+)  # regex on received data; captures in ${MATCH_0}, ${MATCH_1}, ...
//! timeout 2000         # default timeout of the following expects (initially 5000 ms)
//! expect[500] OK       # one-off timeout
//! if ${MATCH_1} == 99 goto no_signal
//! if_match ERROR goto failed     # wait up to the timeout; jump if it matched
//! set retries 3
//! goto done
//! label no_signal      # or `no_signal:`
//! fail no signal
//! done:
//! break 250
//! exec ./flash.sh ${MATCH_1}     # opt-in; no shell; ${EXEC_OUTPUT}, ${EXEC_STATUS}
//! send_file ymodem fw.bin        # file transfer; receive_file xmodem downloads/
//! include common.script
//! exit
//! ```
//!
//! Differences from the C version: `exec` runs a program directly (arguments are split before
//! `${...}` is expanded, so received data cannot inject arguments; ROADMAP A1) and only when
//! allowed; the C `CONDITIONAL` stub became `if`/`if_match`/`goto`; regexes use Rust syntax.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use regex::bytes::Regex;
use serde::Serialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::session::{Session, SessionError};
use crate::transfer::{TransferProtocol, TransferRequest, TransferState};

/// Default `expect` timeout.
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;
/// Deepest `include` nesting.
const MAX_INCLUDE_DEPTH: usize = 8;
/// Largest script file.
const MAX_SCRIPT_BYTES: u64 = 1 << 20;

/// Read a script file: regular files only (not `/dev/zero` or a FIFO), at most 1 MiB.
fn read_script(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() > MAX_SCRIPT_BYTES {
        return Err(format!("{} is larger than 1 MiB", path.display()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Debug, Clone, PartialEq)]
enum Stmt {
    Send(String),
    SendHex(String),
    Expect {
        pattern: String,
        timeout: Option<u64>,
    },
    IfMatch {
        pattern: String,
        timeout: Option<u64>,
        label: String,
    },
    If {
        lhs: String,
        equal: bool,
        rhs: String,
        label: String,
    },
    Goto(String),
    Label,
    Delay(u64),
    Timeout(u64),
    SetDtr(bool),
    SetRts(bool),
    Break(u64),
    Log(String),
    Exec(Vec<String>),
    Set(String, String),
    SendFile {
        protocol: TransferProtocol,
        paths: Vec<String>,
    },
    ReceiveFile {
        protocol: TransferProtocol,
        dir: String,
    },
    Exit,
    Fail(String),
}

#[derive(Debug, Clone)]
struct Line {
    stmt: Stmt,
    file: Arc<str>,
    lineno: usize,
    base: Option<Arc<Path>>,
}

/// A parsed script.
#[derive(Debug, Clone)]
pub struct Script {
    name: String,
    lines: Vec<Line>,
    labels: HashMap<String, usize>,
}

/// Where a problem is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    /// File (or `<script>` for text).
    pub file: String,
    /// 1-based line.
    pub line: usize,
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// Script errors.
#[derive(Debug, Error)]
pub enum ScriptError {
    /// The script text is invalid.
    #[error("{at}: {message}")]
    Parse {
        /// Where.
        at: Location,
        /// What.
        message: String,
    },
    /// A statement failed while running (`fail` included).
    #[error("{at}: {message}")]
    Failed {
        /// Where.
        at: Location,
        /// What.
        message: String,
        /// What happened up to the failure.
        report: Box<ScriptReport>,
    },
    /// Stopped by the user.
    #[error("cancelled")]
    Cancelled,
    /// Another macro or script runs in the session.
    #[error("{0} is already running in this session")]
    Busy(String),
}

/// Result of a run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScriptReport {
    /// Script name.
    pub name: String,
    /// Statements executed.
    pub steps: u64,
    /// Duration.
    pub duration_ms: u64,
    /// `log` output in order.
    pub log: Vec<String>,
    /// Variables at the end.
    pub vars: BTreeMap<String, String>,
    /// Data received during the run (last 64 KiB, lossy UTF-8).
    pub received: String,
}

/// Progress of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptEvent {
    /// About to execute this line.
    Line(Location),
    /// A `log` statement.
    Log(String),
}

/// Policy for a run.
#[derive(Debug, Clone, Default)]
pub struct ScriptOptions {
    /// Allow `exec`.
    pub allow_exec: bool,
    /// Directories `include`, `send_file` and `receive_file` may use (`None`: anywhere).
    pub allowed_dirs: Option<Vec<PathBuf>>,
    /// Initial variables.
    pub vars: BTreeMap<String, String>,
}

fn parse_err(file: &str, line: usize, message: impl Into<String>) -> ScriptError {
    ScriptError::Parse {
        at: Location {
            file: file.to_owned(),
            line,
        },
        message: message.into(),
    }
}

/// `500`, `500ms`, `1.5s`, `2 s`.
fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (num, scale) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000.0)
    } else {
        (s.as_str(), 1.0)
    };
    let v: f64 = num.trim().parse().ok()?;
    (v.is_finite() && v >= 0.0 && v * scale <= 86_400_000.0).then(|| (v * scale).round() as u64)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "high" | "yes" => Some(true),
        "0" | "off" | "false" | "low" | "no" => Some(false),
        _ => None,
    }
}

/// Split a command line into arguments: whitespace separates, '…' and "…" quote, `\` escapes.
fn split_args(s: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_arg = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_arg = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err("unterminated '".into()),
                    }
                }
            }
            '"' => {
                in_arg = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => cur.push(chars.next().ok_or("unterminated \"")?),
                        Some(c) => cur.push(c),
                        None => return Err("unterminated \"".into()),
                    }
                }
            }
            '\\' => {
                in_arg = true;
                cur.push(chars.next().ok_or("trailing backslash")?);
            }
            c if c.is_whitespace() => {
                if in_arg {
                    args.push(std::mem::take(&mut cur));
                    in_arg = false;
                }
            }
            c => {
                in_arg = true;
                cur.push(c);
            }
        }
    }
    if in_arg {
        args.push(cur);
    }
    Ok(args)
}

/// `cmd[123] rest` → (`cmd`, Some(123), `rest`).
fn bracket(cmd: &str) -> Result<(&str, Option<u64>), String> {
    match cmd.split_once('[') {
        None => Ok((cmd, None)),
        Some((name, rest)) => {
            let ms = rest
                .strip_suffix(']')
                .and_then(parse_duration)
                .ok_or_else(|| format!("bad timeout in {cmd:?}"))?;
            Ok((name, Some(ms)))
        }
    }
}

fn check_regex(pattern: &str) -> Result<(), String> {
    if pattern.contains("${") {
        return Ok(()); // compiled after expansion
    }
    Regex::new(pattern)
        .map(|_| ())
        .map_err(|e| format!("invalid regular expression: {e}"))
}

/// Resolve `p` against `base` and check it against `allowed`.
fn resolve_path(p: &str, base: Option<&Path>, allowed: Option<&[PathBuf]>) -> Result<PathBuf, String> {
    let path = Path::new(p);
    let path = match base {
        Some(b) if path.is_relative() => b.join(path),
        _ => path.to_owned(),
    };
    let Some(allowed) = allowed else {
        return Ok(path);
    };
    // Directories to receive into may not exist yet: check the nearest existing ancestor.
    let mut probe = path.clone();
    let mut rest = Vec::new();
    let real = loop {
        if let Ok(r) = probe.canonicalize() {
            break r;
        }
        let Some(name) = probe.file_name().map(ToOwned::to_owned) else {
            return Err(format!("{}: no such file or directory", path.display()));
        };
        if name == ".." {
            return Err(format!("{} must not use `..`", path.display()));
        }
        rest.push(name);
        if !probe.pop() {
            return Err(format!("{}: no such file or directory", path.display()));
        }
    };
    let ok = allowed
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| real.starts_with(d));
    if !ok {
        return Err(format!("{} is outside the allowed directories", real.display()));
    }
    let mut full = real;
    for r in rest.into_iter().rev() {
        full.push(r);
    }
    Ok(full)
}

impl Script {
    /// Parse script text. `base` resolves `include`, `send_file` and `receive_file` paths.
    pub fn parse(name: &str, text: &str, base: Option<&Path>) -> Result<Self, ScriptError> {
        Self::parse_with(name, text, base, None)
    }

    /// Parse a script file.
    pub fn load(path: &Path, allowed: Option<&[PathBuf]>) -> Result<Self, ScriptError> {
        let shown = path.display().to_string();
        let path = resolve_path(&shown, None, allowed).map_err(|e| parse_err(&shown, 0, e))?;
        let text = read_script(&path).map_err(|e| parse_err(&shown, 0, e))?;
        let name = path
            .file_name()
            .map_or(shown.clone(), |n| n.to_string_lossy().into_owned());
        Self::parse_with(&name, &text, path.parent(), allowed)
    }

    /// Parse with a path policy for `include`.
    pub fn parse_with(
        name: &str,
        text: &str,
        base: Option<&Path>,
        allowed: Option<&[PathBuf]>,
    ) -> Result<Self, ScriptError> {
        let mut script = Self {
            name: name.to_owned(),
            lines: Vec::new(),
            labels: HashMap::new(),
        };
        let mut gotos = Vec::new();
        parse_into(&mut script, &mut gotos, name, text, base, allowed, 0)?;
        for (label, file, line) in gotos {
            if !script.labels.contains_key(&label) {
                return Err(parse_err(&file, line, format!("unknown label {label:?}")));
            }
        }
        Ok(script)
    }

    /// Script name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Number of statements (after includes).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// `true` if the script has no statements.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Uses `exec`.
    pub fn uses_exec(&self) -> bool {
        self.lines.iter().any(|l| matches!(l.stmt, Stmt::Exec(_)))
    }
}

#[allow(clippy::too_many_lines)]
fn parse_into(
    script: &mut Script,
    gotos: &mut Vec<(String, String, usize)>,
    file: &str,
    text: &str,
    base: Option<&Path>,
    allowed: Option<&[PathBuf]>,
    depth: usize,
) -> Result<(), ScriptError> {
    let file_arc: Arc<str> = Arc::from(file);
    let base_arc: Option<Arc<Path>> = base.map(Arc::from);
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |m: String| parse_err(file, n, m);
        let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let need = |what: &str| -> Result<(), ScriptError> {
            if rest.is_empty() {
                Err(err(format!("{cmd} needs {what}")))
            } else {
                Ok(())
            }
        };
        let mut label_target = |s: &str| -> Result<String, ScriptError> {
            let l = s.trim();
            if l.is_empty() || l.contains(char::is_whitespace) {
                return Err(err(format!("bad label {l:?}")));
            }
            gotos.push((l.to_owned(), file.to_owned(), n));
            Ok(l.to_owned())
        };
        let (name, bracket_ms) = bracket(cmd).map_err(err)?;
        if bracket_ms.is_some() && !matches!(name, "expect" | "if_match") {
            return Err(err(format!("{name} takes no [timeout]")));
        }
        let stmt = match name.to_ascii_lowercase().as_str() {
            // `send` keeps the text after the first space verbatim (trailing spaces included).
            "send" => Stmt::Send(
                raw.trim_start()
                    .split_once(char::is_whitespace)
                    .map_or("", |(_, r)| r)
                    .to_owned(),
            ),
            "send_hex" => {
                need("hex bytes")?;
                if !rest.contains("${") {
                    gtkterm_protocols::hex::parse_hex(rest).map_err(|e| err(e.to_string()))?;
                }
                Stmt::SendHex(rest.to_owned())
            }
            "expect" => {
                need("a regular expression")?;
                check_regex(rest).map_err(err)?;
                Stmt::Expect {
                    pattern: rest.to_owned(),
                    timeout: bracket_ms,
                }
            }
            "if_match" => {
                let (pattern, label) = rest
                    .rsplit_once(" goto ")
                    .ok_or_else(|| err("if_match needs `<regex> goto <label>`".into()))?;
                check_regex(pattern.trim()).map_err(err)?;
                Stmt::IfMatch {
                    pattern: pattern.trim().to_owned(),
                    timeout: bracket_ms,
                    label: label_target(label)?,
                }
            }
            "if" => {
                let (cond, label) = rest
                    .rsplit_once(" goto ")
                    .ok_or_else(|| err("if needs `<a> == <b> goto <label>`".into()))?;
                let (lhs, rhs, equal) = if let Some((a, b)) = cond.split_once("!=") {
                    (a, b, false)
                } else if let Some((a, b)) = cond.split_once("==") {
                    (a, b, true)
                } else {
                    return Err(err("if needs == or !=".into()));
                };
                Stmt::If {
                    lhs: lhs.trim().to_owned(),
                    equal,
                    rhs: rhs.trim().to_owned(),
                    label: label_target(label)?,
                }
            }
            "goto" => Stmt::Goto(label_target(rest)?),
            "label" => {
                need("a name")?;
                define_label(script, file, n, rest)?;
                Stmt::Label
            }
            "delay" | "wait" => {
                need("a duration")?;
                Stmt::Delay(parse_duration(rest).ok_or_else(|| err(format!("bad duration {rest:?}")))?)
            }
            "timeout" => {
                need("a duration")?;
                Stmt::Timeout(parse_duration(rest).ok_or_else(|| err(format!("bad duration {rest:?}")))?)
            }
            "set_dtr" | "set_rts" => {
                let v =
                    parse_bool(rest).ok_or_else(|| err(format!("{name} takes 0/1, on/off or true/false")))?;
                if name == "set_dtr" {
                    Stmt::SetDtr(v)
                } else {
                    Stmt::SetRts(v)
                }
            }
            "break" => Stmt::Break(if rest.is_empty() {
                250
            } else {
                parse_duration(rest).ok_or_else(|| err(format!("bad duration {rest:?}")))?
            }),
            "log" => Stmt::Log(rest.to_owned()),
            "exec" => {
                need("a command")?;
                Stmt::Exec(split_args(rest).map_err(err)?)
            }
            "set" => {
                let (var, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                if var.is_empty() || !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err(err(format!("bad variable name {var:?}")));
                }
                Stmt::Set(var.to_owned(), value.trim().to_owned())
            }
            "send_file" | "receive_file" => {
                let mut args = split_args(rest).map_err(err)?.into_iter();
                let protocol = args
                    .next()
                    .and_then(|p| TransferProtocol::parse(&p))
                    .ok_or_else(|| {
                        err(format!(
                            "{name} needs a protocol (xmodem, xmodem_1k, ymodem, ymodem_g, zmodem)"
                        ))
                    })?;
                let rest: Vec<String> = args.collect();
                if name == "send_file" {
                    if rest.is_empty() {
                        return Err(err("send_file needs a file".into()));
                    }
                    Stmt::SendFile {
                        protocol,
                        paths: rest,
                    }
                } else {
                    Stmt::ReceiveFile {
                        protocol,
                        dir: rest.into_iter().next().unwrap_or_else(|| ".".into()),
                    }
                }
            }
            "include" => {
                need("a file")?;
                if depth >= MAX_INCLUDE_DEPTH {
                    return Err(err("includes nested too deeply".into()));
                }
                let path = resolve_path(rest, base, allowed).map_err(err)?;
                let text = read_script(&path).map_err(err)?;
                let shown = path.display().to_string();
                parse_into(script, gotos, &shown, &text, path.parent(), allowed, depth + 1)?;
                continue;
            }
            "exit" => Stmt::Exit,
            "fail" => Stmt::Fail(if rest.is_empty() {
                "fail".into()
            } else {
                rest.to_owned()
            }),
            _ if rest.is_empty() && name.ends_with(':') && name.len() > 1 => {
                define_label(script, file, n, &name[..name.len() - 1])?;
                Stmt::Label
            }
            other => return Err(err(format!("unknown command {other:?}"))),
        };
        script.lines.push(Line {
            stmt,
            file: Arc::clone(&file_arc),
            lineno: n,
            base: base_arc.clone(),
        });
    }
    Ok(())
}

fn define_label(script: &mut Script, file: &str, n: usize, name: &str) -> Result<(), ScriptError> {
    if name.contains(char::is_whitespace) {
        return Err(parse_err(file, n, format!("bad label {name:?}")));
    }
    if script
        .labels
        .insert(name.to_owned(), script.lines.len())
        .is_some()
    {
        return Err(parse_err(file, n, format!("label {name:?} defined twice")));
    }
    Ok(())
}

/// `${name}` → value; unknown variables stay as written (like the C version).
fn expand(s: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let name = &after[..j];
        if let Some(v) = vars.get(name) {
            out.push_str(v);
        } else {
            out.push_str("${");
            out.push_str(name);
            out.push('}');
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

/// The C version's `send` escapes: `\n \r \t \\ \xHH`, plus `\0` and `\e`; any other escaped
/// character stands for itself.
fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
            match b[i] {
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'0' => out.push(0),
                b'e' => out.push(0x1B),
                b'x' if b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                    && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit) =>
                {
                    let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("00");
                    out.push(u8::from_str_radix(hex, 16).unwrap_or(0));
                    i += 2;
                }
                c => out.push(c),
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    out
}

struct Runner<'a> {
    session: &'a Session,
    script: &'a Script,
    opts: &'a ScriptOptions,
    cancel: &'a CancellationToken,
    vars: BTreeMap<String, String>,
    report: ScriptReport,
    timeout_ms: u64,
    cursor: u64,
    start_cursor: u64,
    started: Instant,
}

enum Flow {
    Next,
    Jump(usize),
    Exit,
}

impl Runner<'_> {
    fn label(&self, l: &str) -> usize {
        self.script.labels[l]
    }

    async fn sleep(&self, ms: u64) -> Result<(), String> {
        tokio::select! {
            () = self.cancel.cancelled() => Err(String::new()),
            () = tokio::time::sleep(Duration::from_millis(ms)) => Ok(()),
        }
    }

    fn set_matches(&mut self, matched: &str, captures: &[Option<String>]) {
        self.vars.retain(|k, _| !k.starts_with("MATCH_"));
        self.vars.insert("MATCH_0".into(), matched.to_owned());
        for (i, c) in captures.iter().enumerate() {
            self.vars
                .insert(format!("MATCH_{}", i + 1), c.clone().unwrap_or_default());
        }
    }

    /// `Ok(true)` matched, `Ok(false)` timed out.
    async fn wait_for(&mut self, pattern: &str, timeout: Option<u64>) -> Result<bool, String> {
        let pattern = expand(pattern, &self.vars);
        let re = Regex::new(&pattern).map_err(|e| format!("invalid regular expression: {e}"))?;
        let t = Duration::from_millis(timeout.unwrap_or(self.timeout_ms));
        let r = tokio::select! {
            () = self.cancel.cancelled() => return Err(String::new()),
            r = self.session.expect(&re, self.cursor, t) => r,
        };
        match r {
            Ok(m) => {
                self.cursor = m.cursor;
                self.set_matches(&m.matched, &m.captures);
                Ok(true)
            }
            Err(SessionError::Timeout(_)) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn send(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        // Replies to this send count from here, but not before the previous match.
        self.cursor = self.cursor.max(self.session.rx_cursor());
        self.session.send(bytes).await.map_err(|e| e.to_string())
    }

    fn path(&self, l: &Line, p: &str) -> Result<PathBuf, String> {
        resolve_path(
            &expand(p, &self.vars),
            l.base.as_deref(),
            self.opts.allowed_dirs.as_deref(),
        )
    }

    async fn exec(&mut self, argv: &[String]) -> Result<(), String> {
        if !self.opts.allow_exec {
            return Err(
                "exec is disabled (enable it in Preferences → Scripts, or --allow-exec / --mcp-allow-exec)"
                    .into(),
            );
        }
        let argv: Vec<String> = argv.iter().map(|a| expand(a, &self.vars)).collect();
        let (prog, arguments) = argv.split_first().ok_or("exec needs a command")?;
        let mut cmd = tokio::process::Command::new(prog);
        cmd.args(arguments)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let child = cmd.spawn().map_err(|e| format!("cannot run {prog}: {e}"))?;
        let out = tokio::select! {
            () = self.cancel.cancelled() => return Err(String::new()),
            r = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()) => r
                .map_err(|_| format!("{prog} did not finish within 60 s"))?
                .map_err(|e| e.to_string())?,
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        self.vars.insert(
            "EXEC_OUTPUT".into(),
            stdout.trim_end_matches(['\r', '\n']).to_owned(),
        );
        self.vars.insert(
            "EXEC_STATUS".into(),
            out.status
                .code()
                .map_or_else(|| "signal".into(), |c| c.to_string()),
        );
        Ok(())
    }

    async fn transfer(&mut self, req: TransferRequest) -> Result<(), String> {
        let st = tokio::select! {
            () = self.cancel.cancelled() => {
                self.session.cancel_transfer().await;
                return Err(String::new());
            }
            r = self.session.transfer(req, |_| {}) => r.map_err(|e| e.to_string())?,
        };
        if st.state != TransferState::Finished {
            return Err(st.summary());
        }
        self.vars
            .insert("TRANSFER_BYTES".into(), st.total_bytes.to_string());
        if let Some(f) = st.files.last() {
            self.vars
                .insert("TRANSFER_FILE".into(), f.path.display().to_string());
        }
        self.cursor = self.session.rx_cursor();
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn step(&mut self, l: &Line) -> Result<Flow, String> {
        match &l.stmt {
            Stmt::Send(text) => {
                let bytes = unescape(&expand(text, &self.vars));
                self.send(bytes).await?;
            }
            Stmt::SendHex(hex) => {
                let bytes =
                    gtkterm_protocols::hex::parse_hex(&expand(hex, &self.vars)).map_err(|e| e.to_string())?;
                self.send(bytes).await?;
            }
            Stmt::Expect { pattern, timeout } => {
                if !self.wait_for(pattern, *timeout).await? {
                    let got = self.session.read_rx(self.cursor, 512);
                    return Err(format!(
                        "timed out after {} ms waiting for {:?}; received: {:?}",
                        timeout.unwrap_or(self.timeout_ms),
                        expand(pattern, &self.vars),
                        String::from_utf8_lossy(&got.data)
                    ));
                }
            }
            Stmt::IfMatch {
                pattern,
                timeout,
                label,
            } => {
                if self.wait_for(pattern, *timeout).await? {
                    return Ok(Flow::Jump(self.label(label)));
                }
            }
            Stmt::If {
                lhs,
                equal,
                rhs,
                label,
            } => {
                let same = expand(lhs, &self.vars) == expand(rhs, &self.vars);
                if same == *equal {
                    return Ok(Flow::Jump(self.label(label)));
                }
            }
            Stmt::Goto(label) => return Ok(Flow::Jump(self.label(label))),
            Stmt::Label => {}
            Stmt::Delay(ms) => self.sleep(*ms).await?,
            Stmt::Timeout(ms) => self.timeout_ms = *ms,
            Stmt::SetDtr(on) => self.session.set_dtr(*on).await.map_err(|e| e.to_string())?,
            Stmt::SetRts(on) => self.session.set_rts(*on).await.map_err(|e| e.to_string())?,
            Stmt::Break(ms) => self
                .session
                .send_break(Duration::from_millis(*ms))
                .await
                .map_err(|e| e.to_string())?,
            Stmt::Log(text) => {
                let msg = expand(text, &self.vars);
                self.report.log.push(msg);
            }
            Stmt::Exec(argv) => self.exec(argv).await?,
            Stmt::Set(var, value) => {
                let v = expand(value, &self.vars);
                self.vars.insert(var.clone(), v);
            }
            Stmt::SendFile { protocol, paths } => {
                let files = paths
                    .iter()
                    .map(|p| self.path(l, p))
                    .collect::<Result<Vec<_>, _>>()?;
                self.transfer(TransferRequest::Send {
                    protocol: *protocol,
                    files,
                })
                .await?;
            }
            Stmt::ReceiveFile { protocol, dir } => {
                let dir = self.path(l, dir)?;
                self.transfer(TransferRequest::Receive {
                    protocol: *protocol,
                    dir,
                    name: None,
                    overwrite: false,
                })
                .await?;
            }
            Stmt::Exit => return Ok(Flow::Exit),
            Stmt::Fail(msg) => return Err(expand(msg, &self.vars)),
        }
        Ok(Flow::Next)
    }

    fn finish(&mut self) {
        self.report.duration_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.report.vars = self.vars.clone();
        self.report.received =
            String::from_utf8_lossy(&self.session.read_rx(self.start_cursor, 64 * 1024).data).into_owned();
    }
}

/// Run `script` in `session` until it ends, fails or `cancel` fires. `on_event` sees every line
/// before it runs and every `log` message.
pub async fn run_script(
    session: &Session,
    script: &Script,
    opts: &ScriptOptions,
    cancel: &CancellationToken,
    mut on_event: impl FnMut(ScriptEvent),
) -> Result<ScriptReport, ScriptError> {
    if script.uses_exec() && !opts.allow_exec {
        let l = script
            .lines
            .iter()
            .find(|l| matches!(l.stmt, Stmt::Exec(_)))
            .expect("uses exec");
        return Err(ScriptError::Failed {
            at: Location {
                file: l.file.to_string(),
                line: l.lineno,
            },
            message:
                "exec is disabled (enable it in Preferences → Scripts, or --allow-exec / --mcp-allow-exec)"
                    .into(),
            report: Box::default(),
        });
    }
    let start_cursor = session.rx_cursor();
    let mut r = Runner {
        session,
        script,
        opts,
        cancel,
        vars: opts.vars.clone(),
        report: ScriptReport {
            name: script.name.clone(),
            ..ScriptReport::default()
        },
        timeout_ms: DEFAULT_TIMEOUT_MS,
        cursor: start_cursor,
        start_cursor,
        started: Instant::now(),
    };
    let mut pc = 0;
    while let Some(l) = script.lines.get(pc) {
        if cancel.is_cancelled() {
            return Err(ScriptError::Cancelled);
        }
        on_event(ScriptEvent::Line(Location {
            file: l.file.to_string(),
            line: l.lineno,
        }));
        r.report.steps += 1;
        let logged = r.report.log.len();
        let flow = r.step(l).await;
        for msg in &r.report.log[logged..] {
            on_event(ScriptEvent::Log(msg.clone()));
        }
        match flow {
            Ok(Flow::Next) => pc += 1,
            Ok(Flow::Jump(to)) => {
                pc = to;
                // A loop without waits must not starve the runtime.
                tokio::task::yield_now().await;
            }
            Ok(Flow::Exit) => break,
            Err(_) if cancel.is_cancelled() => return Err(ScriptError::Cancelled),
            Err(message) => {
                r.finish();
                return Err(ScriptError::Failed {
                    at: Location {
                        file: l.file.to_string(),
                        line: l.lineno,
                    },
                    message,
                    report: Box::new(r.report),
                });
            }
        }
    }
    r.finish();
    Ok(r.report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Script, ScriptError> {
        Script::parse("t", text, None)
    }

    #[test]
    fn shipped_examples_parse() {
        for f in ["example_modem.script", "example_embedded.script"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../data/examples")
                .join(f);
            let s = Script::load(&path, None).unwrap_or_else(|e| panic!("{f}: {e}"));
            assert!(s.len() > 10, "{f}");
        }
    }

    #[test]
    fn parse_errors_have_line_numbers() {
        let e = parse("send a\n\nfrobnicate\n").unwrap_err().to_string();
        assert_eq!(e, "t:3: unknown command \"frobnicate\"");
        assert!(
            parse("expect (unclosed")
                .unwrap_err()
                .to_string()
                .contains("t:1: invalid regular")
        );
        assert!(
            parse("goto nowhere")
                .unwrap_err()
                .to_string()
                .contains("unknown label")
        );
        assert!(
            parse("a:\nlabel a")
                .unwrap_err()
                .to_string()
                .contains("defined twice")
        );
        assert!(parse("set_dtr maybe").is_err());
        assert!(parse("delay soon").is_err());
        assert!(parse("send_hex ZZ").is_err());
        assert!(parse("delay[5] 1").is_err());
        assert!(parse("if a = b goto x\nx:").is_err());
        assert!(parse("  # indented comment\n\tdelay 1.5s\nexpect[2s] x\n").is_ok());
    }

    #[test]
    fn helpers() {
        assert_eq!(parse_duration("1.5s"), Some(1500));
        assert_eq!(parse_duration("250 ms"), Some(250));
        assert_eq!(parse_duration("-1"), None);
        assert_eq!(unescape(r"AT\r\n\x41\+\\"), b"AT\r\nA+\\");
        assert_eq!(unescape(r"\xZ1"), b"xZ1", "bad hex escape keeps the text");
        let vars = BTreeMap::from([("a".to_owned(), "1".to_owned())]);
        assert_eq!(expand("x${a}y${b}${", &vars), "x1y${b}${");
        assert_eq!(
            split_args(r#"./flash.sh "two words" it\'s '${MATCH_1}'"#).unwrap(),
            ["./flash.sh", "two words", "it's", "${MATCH_1}"]
        );
        assert!(split_args("\"open").is_err());
    }

    #[test]
    fn includes_resolve_relative_and_respect_the_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("lib")).unwrap();
        std::fs::write(
            dir.path().join("lib/common.script"),
            "common:\nlog from include\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.script"),
            "include lib/common.script\ngoto common\n",
        )
        .unwrap();
        let s = Script::load(&dir.path().join("main.script"), None).unwrap();
        assert_eq!(s.len(), 3);
        let other = tempfile::tempdir().unwrap();
        let allowed = [other.path().to_owned()];
        let e = Script::load(&dir.path().join("main.script"), Some(&allowed)).unwrap_err();
        assert!(e.to_string().contains("outside the allowed"), "{e}");
        let e = Script::parse("t", "include /dev/zero\n", None).unwrap_err();
        assert!(e.to_string().contains("not a regular file"), "{e}");
        std::fs::write(dir.path().join("loop.script"), "include loop.script\n").unwrap();
        assert!(
            Script::load(&dir.path().join("loop.script"), None)
                .unwrap_err()
                .to_string()
                .contains("nested")
        );
    }
}
