//! Macros: named command sequences (send / wait / expect) with optional keyboard shortcuts.
//!
//! Stored in `config_dir()/macros.json` using the C version's schema
//! (`{"macros":[{"name","shortcut","commands":[{"type":0|1|2,"data","delay"}]}]}`) but with
//! standard JSON escapes; `data` holds escaped text (`ATZ\r`, see [`gtkterm_protocols::escape`]).
//! The C version wrote `g_strescape` octal escapes, so its files were invalid JSON for non-ASCII
//! text (ROADMAP defect M1); [`import_c_macros`] reads them anyway.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gtkterm_protocols::escape;
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::session::{Session, SessionError};

/// Default timeout of an `expect` step.
pub const DEFAULT_EXPECT_TIMEOUT_MS: u64 = 5_000;

/// One step of a macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroCommand {
    /// Send escaped text after waiting `delay_ms`.
    Send {
        /// Escaped text, e.g. `ATZ\r`.
        data: String,
        /// Pause before sending.
        delay_ms: u64,
    },
    /// Pause.
    Wait {
        /// Duration.
        ms: u64,
    },
    /// Wait until received data matches a regular expression.
    Expect {
        /// Regex (Rust syntax) matched against received bytes.
        pattern: String,
        /// Pause before starting to wait.
        delay_ms: u64,
        /// Give up after this long.
        timeout_ms: u64,
    },
}

/// A named macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macro {
    /// Unique name.
    pub name: String,
    /// GTK accelerator string such as `F9` or `<Control><Shift>1`, if bound.
    pub shortcut: Option<String>,
    /// Steps in order.
    pub commands: Vec<MacroCommand>,
}

/// Macro errors.
#[derive(Debug, Error)]
pub enum MacroError {
    /// File I/O.
    #[error("{path}: {source}")]
    Io {
        /// File.
        path: PathBuf,
        /// Cause.
        source: std::io::Error,
    },
    /// Malformed JSON.
    #[error("{path}: {source}")]
    Json {
        /// File.
        path: PathBuf,
        /// Cause.
        source: serde_json::Error,
    },
    /// Editor text error.
    #[error("line {line}: {message}")]
    Syntax {
        /// 1-based line.
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// Invalid macro definition.
    #[error("{0}")]
    Invalid(String),
    /// No such macro.
    #[error("no macro named {0:?}")]
    NotFound(String),
    /// Playback was cancelled.
    #[error("macro cancelled")]
    Cancelled,
    /// A step failed.
    #[error("step {step} failed: {message}")]
    Step {
        /// 1-based step number.
        step: usize,
        /// Reason.
        message: String,
        /// What happened up to the failure.
        report: Box<MacroReport>,
    },
    /// Another macro is already running in this session.
    #[error("macro {0:?} is already running in this session")]
    Busy(String),
}

// ----- on-disk format -----

#[derive(Debug, Serialize, Deserialize)]
struct DiskFile {
    macros: Vec<DiskMacro>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DiskMacro {
    name: String,
    #[serde(default)]
    shortcut: String,
    #[serde(default)]
    commands: Vec<DiskCommand>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DiskCommand {
    #[serde(rename = "type")]
    kind: u8,
    #[serde(default)]
    data: String,
    #[serde(default)]
    delay: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
}

impl From<&Macro> for DiskMacro {
    fn from(m: &Macro) -> Self {
        Self {
            name: m.name.clone(),
            shortcut: m.shortcut.clone().unwrap_or_default(),
            commands: m
                .commands
                .iter()
                .map(|c| match c {
                    MacroCommand::Send { data, delay_ms } => DiskCommand {
                        kind: 0,
                        data: data.clone(),
                        delay: *delay_ms,
                        timeout: None,
                    },
                    MacroCommand::Wait { ms } => DiskCommand {
                        kind: 1,
                        data: String::new(),
                        delay: *ms,
                        timeout: None,
                    },
                    MacroCommand::Expect {
                        pattern,
                        delay_ms,
                        timeout_ms,
                    } => DiskCommand {
                        kind: 2,
                        data: pattern.clone(),
                        delay: *delay_ms,
                        timeout: Some(*timeout_ms),
                    },
                })
                .collect(),
        }
    }
}

fn from_disk(d: DiskMacro, decode_data: impl Fn(&str) -> String) -> Option<Macro> {
    let name = d.name.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let commands = d
        .commands
        .into_iter()
        .filter_map(|c| match c.kind {
            0 => Some(MacroCommand::Send {
                data: decode_data(&c.data),
                delay_ms: c.delay,
            }),
            1 => Some(MacroCommand::Wait { ms: c.delay }),
            2 => Some(MacroCommand::Expect {
                pattern: c.data,
                delay_ms: if c.timeout.is_some() { c.delay } else { 0 },
                timeout_ms: c.timeout.unwrap_or(DEFAULT_EXPECT_TIMEOUT_MS),
            }),
            _ => None,
        })
        .collect();
    let shortcut = Some(d.shortcut.trim().to_owned()).filter(|s| !s.is_empty());
    Some(Macro {
        name,
        shortcut,
        commands,
    })
}

/// All macros, in display order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MacroStore {
    macros: Vec<Macro>,
}

/// Path of `macros.json`.
pub fn macros_path() -> PathBuf {
    crate::config::config_dir().join("macros.json")
}

impl MacroStore {
    /// Parse our JSON format.
    pub fn from_json(text: &str, path: &Path) -> Result<Self, MacroError> {
        let file: DiskFile = serde_json::from_str(text).map_err(|source| MacroError::Json {
            path: path.to_owned(),
            source,
        })?;
        let mut store = Self::default();
        for d in file.macros {
            if let Some(m) = from_disk(d, str::to_owned) {
                store.insert_unique(m);
            }
        }
        Ok(store)
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        let file = DiskFile {
            macros: self.macros.iter().map(DiskMacro::from).collect(),
        };
        serde_json::to_string_pretty(&file).unwrap_or_else(|_| "{\"macros\":[]}".into()) + "\n"
    }

    /// Load from `path`; a missing file is an empty store.
    pub fn load(path: &Path) -> Result<Self, MacroError> {
        match std::fs::read_to_string(path) {
            Ok(t) => Self::from_json(&t, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(MacroError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Write atomically.
    pub fn save(&self, path: &Path) -> Result<(), MacroError> {
        let io = |source| MacroError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json()).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    /// Macros in order.
    pub fn list(&self) -> &[Macro] {
        &self.macros
    }

    /// Macro by name.
    pub fn get(&self, name: &str) -> Option<&Macro> {
        self.macros.iter().find(|m| m.name == name)
    }

    /// Add with a unique name ("x", "x (2)", ...); returns the name used.
    pub fn insert_unique(&mut self, mut m: Macro) -> String {
        let base = m.name.clone();
        let mut n = 2;
        while self.get(&m.name).is_some() {
            m.name = format!("{base} ({n})");
            n += 1;
        }
        let name = m.name.clone();
        self.macros.push(m);
        name
    }

    /// Replace the macro called `old_name` (or add it) with `m`; renames keep the position.
    pub fn upsert(&mut self, old_name: Option<&str>, m: Macro) -> Result<(), MacroError> {
        validate(&m)?;
        let pos = old_name.and_then(|o| self.macros.iter().position(|x| x.name == o));
        if self
            .macros
            .iter()
            .enumerate()
            .any(|(i, x)| x.name == m.name && Some(i) != pos)
        {
            return Err(MacroError::Invalid(format!(
                "a macro named {:?} already exists",
                m.name
            )));
        }
        if let Some(s) = &m.shortcut
            && let Some(other) = self
                .macros
                .iter()
                .enumerate()
                .find(|(i, x)| x.shortcut.as_ref() == Some(s) && Some(*i) != pos)
        {
            return Err(MacroError::Invalid(format!(
                "shortcut {s} is already used by {:?}",
                other.1.name
            )));
        }
        match pos {
            Some(i) => self.macros[i] = m,
            None => self.macros.push(m),
        }
        Ok(())
    }

    /// Remove by name.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.macros.len();
        self.macros.retain(|m| m.name != name);
        self.macros.len() != before
    }

    /// Unused name for a new macro ("New macro", "New macro (2)", ...).
    pub fn fresh_name(&self, base: &str) -> String {
        let mut name = base.to_owned();
        let mut n = 2;
        while self.get(&name).is_some() {
            name = format!("{base} ({n})");
            n += 1;
        }
        name
    }
}

/// Check names, escapes and regexes.
pub fn validate(m: &Macro) -> Result<(), MacroError> {
    if m.name.trim().is_empty() {
        return Err(MacroError::Invalid("macro name is empty".into()));
    }
    for (i, c) in m.commands.iter().enumerate() {
        match c {
            MacroCommand::Send { data, .. } => {
                escape::decode(data).map_err(|e| MacroError::Invalid(format!("step {}: {e}", i + 1)))?;
            }
            MacroCommand::Expect { pattern, .. } => {
                Regex::new(pattern)
                    .map_err(|e| MacroError::Invalid(format!("step {}: invalid pattern: {e}", i + 1)))?;
            }
            MacroCommand::Wait { .. } => {}
        }
    }
    Ok(())
}

// ----- editor text -----

fn parse_bracket(head: &str, line: usize) -> Result<(&str, Option<u64>), MacroError> {
    match head.split_once('[') {
        None => Ok((head.trim(), None)),
        Some((kw, rest)) => {
            let num = rest.strip_suffix(']').ok_or(MacroError::Syntax {
                line,
                message: "missing ']'".into(),
            })?;
            let v = num.trim().parse().map_err(|_| MacroError::Syntax {
                line,
                message: format!("{num:?} is not a number of milliseconds"),
            })?;
            Ok((kw.trim(), Some(v)))
        }
    }
}

fn parse_duration_ms(s: &str, line: usize) -> Result<u64, MacroError> {
    let s = s.trim();
    let err = || MacroError::Syntax {
        line,
        message: format!("{s:?} is not a duration (e.g. 500 ms or 1.5 s)"),
    };
    let (num, scale) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000.0)
    } else {
        (s, 1.0)
    };
    let v: f64 = num.trim().parse().map_err(|_| err())?;
    if !(v.is_finite() && v >= 0.0) {
        return Err(err());
    }
    Ok((v * scale).round() as u64)
}

/// Parse editor text into commands.
///
/// ```text
/// # comment
/// send: ATZ\r            send[250]: AT+CSQ\r   (pause 250 ms first)
/// wait: 500 ms           wait: 1.5 s
/// expect: OK\r\n         expect[2000]: \+CSQ: (\d+)   (timeout in ms, default 5000)
/// ```
pub fn parse_text(text: &str) -> Result<Vec<MacroCommand>, MacroError> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let trimmed = raw.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((head, value)) = trimmed.split_once(':') else {
            return Err(MacroError::Syntax {
                line,
                message: "expected 'send:', 'wait:' or 'expect:'".into(),
            });
        };
        // One space after the colon is syntax; further spaces belong to the data.
        let value = value.strip_prefix(' ').unwrap_or(value);
        let (kw, bracket) = parse_bracket(head, line)?;
        let cmd = match kw.to_ascii_lowercase().as_str() {
            "send" => {
                escape::decode(value).map_err(|e| MacroError::Syntax {
                    line,
                    message: e.to_string(),
                })?;
                MacroCommand::Send {
                    data: value.to_owned(),
                    delay_ms: bracket.unwrap_or(0),
                }
            }
            "wait" => {
                if bracket.is_some() {
                    return Err(MacroError::Syntax {
                        line,
                        message: "wait takes its duration after the colon".into(),
                    });
                }
                MacroCommand::Wait {
                    ms: parse_duration_ms(value, line)?,
                }
            }
            "expect" => {
                let pattern = value.trim_end().to_owned();
                if pattern.is_empty() {
                    return Err(MacroError::Syntax {
                        line,
                        message: "expect needs a pattern".into(),
                    });
                }
                Regex::new(&pattern).map_err(|e| MacroError::Syntax {
                    line,
                    message: format!("invalid pattern: {e}"),
                })?;
                MacroCommand::Expect {
                    pattern,
                    delay_ms: 0,
                    timeout_ms: bracket.unwrap_or(DEFAULT_EXPECT_TIMEOUT_MS),
                }
            }
            other => {
                return Err(MacroError::Syntax {
                    line,
                    message: format!("unknown command {other:?}"),
                });
            }
        };
        out.push(cmd);
    }
    Ok(out)
}

/// Format commands as editor text (inverse of [`parse_text`]).
pub fn format_text(commands: &[MacroCommand]) -> String {
    let mut s = String::new();
    for c in commands {
        match c {
            MacroCommand::Send { data, delay_ms: 0 } => {
                let _ = writeln!(s, "send: {data}");
            }
            MacroCommand::Send { data, delay_ms } => {
                let _ = writeln!(s, "send[{delay_ms}]: {data}");
            }
            MacroCommand::Wait { ms } => {
                let _ = writeln!(s, "wait: {ms} ms");
            }
            MacroCommand::Expect {
                pattern,
                delay_ms,
                timeout_ms,
            } => {
                if *delay_ms > 0 {
                    let _ = writeln!(s, "wait: {delay_ms} ms");
                }
                if *timeout_ms == DEFAULT_EXPECT_TIMEOUT_MS {
                    let _ = writeln!(s, "expect: {pattern}");
                } else {
                    let _ = writeln!(s, "expect[{timeout_ms}]: {pattern}");
                }
            }
        }
    }
    s
}

// ----- importers -----

/// GLib `g_strcompress`: `\b \f \n \r \t \v \" \\`, 1–3 digit octal; other `\c` become `c`.
fn g_strcompress(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        if b != b'\\' || i >= bytes.len() {
            out.push(b);
            continue;
        }
        let e = bytes[i];
        i += 1;
        match e {
            b'0'..=b'7' => {
                let mut v = u32::from(e - b'0');
                let mut n = 1;
                while n < 3 && i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                    v = v * 8 + u32::from(bytes[i] - b'0');
                    i += 1;
                    n += 1;
                }
                out.push((v & 0xFF) as u8);
            }
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            other => out.push(other),
        }
    }
    out
}

/// Make the C writer's output parseable: octal escapes like `\303` are invalid JSON, so turn
/// every backslash inside string literals into `\\`, keeping `\"` and `\\` intact.
fn c_json_to_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if !in_string {
            if c == '"' {
                in_string = true;
            }
            out.push(c);
            continue;
        }
        match c {
            '"' => {
                in_string = false;
                out.push(c);
            }
            '\\' => match chars.next() {
                Some('"') => out.push_str("\\\""),
                Some('\\') => out.push_str("\\\\\\\\"),
                Some(other) => {
                    out.push_str("\\\\");
                    out.push(other);
                }
                None => out.push_str("\\\\"),
            },
            c => out.push(c),
        }
    }
    out
}

/// Import a macros file written by the C version (`g_strescape`d data).
pub fn import_c_macros(text: &str, path: &Path) -> Result<MacroStore, MacroError> {
    let fixed = c_json_to_json(text);
    let file: DiskFile = serde_json::from_str(&fixed).map_err(|source| MacroError::Json {
        path: path.to_owned(),
        source,
    })?;
    let mut store = MacroStore::default();
    for mut d in file.macros {
        d.name = String::from_utf8_lossy(&g_strcompress(&d.name)).into_owned();
        d.shortcut = String::from_utf8_lossy(&g_strcompress(&d.shortcut)).into_owned();
        for c in &mut d.commands {
            if c.kind == 2 {
                c.data = String::from_utf8_lossy(&g_strcompress(&c.data)).into_owned();
            }
        }
        if let Some(m) = from_disk(d, |data| escape::encode(&g_strcompress(data))) {
            store.insert_unique(m);
        }
    }
    Ok(store)
}

/// Import the `macros = shortcut;action;...` key of upstream GTKTerm key files (first section
/// that has one). `\;` is a literal semicolon; actions keep their backslash escapes.
pub fn import_keyfile_macros(text: &str) -> MacroStore {
    let mut store = MacroStore::default();
    let Some(value) = text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "macros").then(|| v.trim().to_owned())
    }) else {
        return store;
    };
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&';') => {
                cur.push(';');
                chars.next();
            }
            ';' => fields.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        fields.push(cur);
    }
    for pair in fields.chunks(2) {
        let [shortcut, action] = pair else { continue };
        let shortcut = shortcut.trim();
        if shortcut.is_empty() || action.is_empty() {
            continue;
        }
        // Keep valid escapes; protect stray backslashes so the action still decodes.
        let data = if escape::decode(action).is_ok() {
            action.clone()
        } else {
            action.replace('\\', "\\\\")
        };
        store.insert_unique(Macro {
            name: format!("{shortcut}: {}", action.chars().take(24).collect::<String>()),
            shortcut: Some(shortcut.to_owned()),
            commands: vec![MacroCommand::Send { data, delay_ms: 0 }],
        });
    }
    store
}

/// Load the macro store; on first run import the C fork's `~/.config/gtkterm/macros.json` or the
/// upstream key file's macros. Returns a message describing an import.
pub fn load_or_import(
    path: &Path,
    c_file: Option<&Path>,
    keyfile: Option<&Path>,
) -> Result<(MacroStore, Option<String>), MacroError> {
    if path.exists() {
        return Ok((MacroStore::load(path)?, None));
    }
    if let Some(c) = c_file.filter(|p| p.exists()) {
        let text = std::fs::read_to_string(c).map_err(|source| MacroError::Io {
            path: c.to_owned(),
            source,
        })?;
        let store = import_c_macros(&text, c)?;
        store.save(path)?;
        return Ok((
            store.clone(),
            Some(format!(
                "Imported {} macro(s) from {}",
                store.list().len(),
                c.display()
            )),
        ));
    }
    if let Some(k) = keyfile.filter(|p| p.exists()) {
        let text = std::fs::read(k).map_err(|source| MacroError::Io {
            path: k.to_owned(),
            source,
        })?;
        let store = import_keyfile_macros(&String::from_utf8_lossy(&text));
        if !store.list().is_empty() {
            store.save(path)?;
            return Ok((
                store.clone(),
                Some(format!(
                    "Imported {} macro(s) from {}",
                    store.list().len(),
                    k.display()
                )),
            ));
        }
    }
    Ok((MacroStore::default(), None))
}

/// Location of the C fork's macros file.
pub fn c_macros_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.config_dir().join("gtkterm").join("macros.json"))
}

// ----- playback -----

/// Outcome of one step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepReport {
    /// 1-based step number.
    pub step: usize,
    /// What the step did.
    pub detail: String,
}

/// Outcome of a macro run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct MacroReport {
    /// Macro name.
    pub name: String,
    /// Completed steps.
    pub steps: Vec<StepReport>,
    /// Text received while the macro ran (lossy UTF-8, at most 64 KiB).
    pub received: String,
    /// Run time in ms.
    pub duration_ms: u64,
}

async fn sleep_or_cancel(ms: u64, cancel: &CancellationToken) -> Result<(), MacroError> {
    if ms == 0 {
        return Ok(());
    }
    tokio::select! {
        () = cancel.cancelled() => Err(MacroError::Cancelled),
        () = tokio::time::sleep(Duration::from_millis(ms)) => Ok(()),
    }
}

/// Run a macro on a session. `expect` steps look at data received since the preceding `send`.
pub async fn run_macro(
    session: &Session,
    m: &Macro,
    cancel: &CancellationToken,
) -> Result<MacroReport, MacroError> {
    validate(m)?;
    let started = Instant::now();
    let start_cursor = session.rx_cursor();
    let mut cursor = start_cursor;
    let mut report = MacroReport {
        name: m.name.clone(),
        ..MacroReport::default()
    };
    let finish = |report: &mut MacroReport| {
        report.received =
            String::from_utf8_lossy(&session.read_rx(start_cursor, 64 * 1024).data).into_owned();
        report.duration_ms = started.elapsed().as_millis() as u64;
    };
    for (i, c) in m.commands.iter().enumerate() {
        let step = i + 1;
        let fail = |report: &mut MacroReport, message: String| {
            finish(report);
            MacroError::Step {
                step,
                message,
                report: Box::new(report.clone()),
            }
        };
        match c {
            MacroCommand::Send { data, delay_ms } => {
                sleep_or_cancel(*delay_ms, cancel).await?;
                let bytes = escape::decode(data).map_err(|e| fail(&mut report, e.to_string()))?;
                cursor = session.rx_cursor();
                let n = bytes.len();
                session
                    .send(bytes)
                    .await
                    .map_err(|e| fail(&mut report, e.to_string()))?;
                report.steps.push(StepReport {
                    step,
                    detail: format!("sent {n} bytes: {data}"),
                });
            }
            MacroCommand::Wait { ms } => {
                sleep_or_cancel(*ms, cancel).await?;
                report.steps.push(StepReport {
                    step,
                    detail: format!("waited {ms} ms"),
                });
            }
            MacroCommand::Expect {
                pattern,
                delay_ms,
                timeout_ms,
            } => {
                sleep_or_cancel(*delay_ms, cancel).await?;
                let re = Regex::new(pattern).map_err(|e| fail(&mut report, e.to_string()))?;
                let timeout = Duration::from_millis(*timeout_ms);
                let result = tokio::select! {
                    () = cancel.cancelled() => return Err(MacroError::Cancelled),
                    r = session.expect(&re, cursor, timeout) => r,
                };
                match result {
                    Ok(found) => {
                        cursor = found.cursor;
                        report.steps.push(StepReport {
                            step,
                            detail: format!("matched {:?}", found.matched),
                        });
                    }
                    Err(SessionError::Timeout(_)) => {
                        return Err(fail(
                            &mut report,
                            format!("no match for {pattern:?} within {timeout_ms} ms"),
                        ));
                    }
                    Err(e) => return Err(fail(&mut report, e.to_string())),
                }
            }
        }
    }
    finish(&mut report);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Macro {
        Macro {
            name: "Modem äöü".into(),
            shortcut: Some("F9".into()),
            commands: vec![
                MacroCommand::Send {
                    data: "ATZ\\r".into(),
                    delay_ms: 0,
                },
                MacroCommand::Expect {
                    pattern: "OK\\r\\n".into(),
                    delay_ms: 0,
                    timeout_ms: 2000,
                },
                MacroCommand::Wait { ms: 100 },
                MacroCommand::Send {
                    data: "x\\x00\\x1B ü".into(),
                    delay_ms: 250,
                },
            ],
        }
    }

    #[test]
    fn json_roundtrip_with_non_ascii_and_controls() {
        // C defect M1: non-ASCII and control bytes produced invalid JSON.
        let mut s = MacroStore::default();
        s.insert_unique(sample());
        let json = s.to_json();
        serde_json::from_str::<serde_json::Value>(&json).unwrap();
        assert_eq!(MacroStore::from_json(&json, Path::new("x")).unwrap(), s);
        assert!(json.contains("\"type\": 2"));
    }

    #[test]
    fn imports_c_fork_file_with_octal_escapes() {
        // Exactly what the C writer produced: g_strescape turns "ä" into \303\244 and CR into \r.
        let c = "{\n  \"macros\": [\n    {\n      \"name\": \"Gr\\303\\274\\303\\237e\",\n      \"shortcut\": \"F2\",\n      \"commands\": [\n        {\"type\": 0, \"data\": \"AT\\r\", \"delay\": 0},\n        {\"type\": 1, \"data\": \"\", \"delay\": 500},\n        {\"type\": 0, \"data\": \"h\\303\\244\\\\x\", \"delay\": 100}\n      ]\n    }\n  ]\n}\n";
        let s = import_c_macros(c, Path::new("c.json")).unwrap();
        let m = &s.list()[0];
        assert_eq!(m.name, "Grüße");
        assert_eq!(m.shortcut.as_deref(), Some("F2"));
        assert_eq!(
            m.commands[0],
            MacroCommand::Send {
                data: "AT\\r".into(),
                delay_ms: 0
            }
        );
        assert_eq!(m.commands[1], MacroCommand::Wait { ms: 500 });
        assert_eq!(
            escape::decode(match &m.commands[2] {
                MacroCommand::Send { data, .. } => data,
                _ => panic!(),
            })
            .unwrap(),
            "hä\\x".as_bytes()
        );
    }

    #[test]
    fn imports_upstream_keyfile_macros() {
        let kf = "[default]\nport = /dev/ttyS0\nmacros = F1;ATZ\\r;<Shift>F2;a\\;b;F3;\n";
        let s = import_keyfile_macros(kf);
        assert_eq!(s.list().len(), 2);
        assert_eq!(s.list()[0].shortcut.as_deref(), Some("F1"));
        assert_eq!(
            s.list()[0].commands,
            vec![MacroCommand::Send {
                data: "ATZ\\r".into(),
                delay_ms: 0
            }]
        );
        assert_eq!(
            s.list()[1].commands,
            vec![MacroCommand::Send {
                data: "a;b".into(),
                delay_ms: 0
            }]
        );
        assert!(import_keyfile_macros("[default]\nport=x\n").list().is_empty());
    }

    #[test]
    fn editor_text_roundtrip_and_errors() {
        let m = sample();
        let text = format_text(&m.commands);
        assert_eq!(
            text,
            "send: ATZ\\r\nexpect[2000]: OK\\r\\n\nwait: 100 ms\nsend[250]: x\\x00\\x1B ü\n"
        );
        assert_eq!(parse_text(&text).unwrap(), m.commands);
        let parsed = parse_text("# setup\n\nsend:  two spaces\nwait: 1.5 s\nWAIT: 20\nexpect: OK\n").unwrap();
        assert_eq!(
            parsed[0],
            MacroCommand::Send {
                data: " two spaces".into(),
                delay_ms: 0
            }
        );
        assert_eq!(parsed[1], MacroCommand::Wait { ms: 1500 });
        assert_eq!(parsed[2], MacroCommand::Wait { ms: 20 });
        assert_eq!(
            parsed[3],
            MacroCommand::Expect {
                pattern: "OK".into(),
                delay_ms: 0,
                timeout_ms: DEFAULT_EXPECT_TIMEOUT_MS
            }
        );
        for (text, line) in [
            ("send: ok\nbogus\n", 2),
            ("wait: soon", 1),
            ("expect: (", 1),
            ("send: \\q", 1),
            ("send[x]: a", 1),
            ("expect:", 1),
        ] {
            match parse_text(text) {
                Err(MacroError::Syntax { line: l, .. }) => assert_eq!(l, line, "{text:?}"),
                other => panic!("{text:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn store_names_shortcuts_and_files() {
        let mut s = MacroStore::default();
        assert_eq!(s.insert_unique(sample()), "Modem äöü");
        assert_eq!(s.insert_unique(sample()), "Modem äöü (2)");
        let mut renamed = sample();
        renamed.name = "Renamed".into();
        renamed.shortcut = Some("F10".into());
        s.upsert(Some("Modem äöü (2)"), renamed).unwrap();
        assert_eq!(s.list()[1].name, "Renamed");
        let mut dup = sample();
        dup.name = "Other".into();
        assert!(matches!(s.upsert(None, dup), Err(MacroError::Invalid(m)) if m.contains("F9")));
        let mut bad = sample();
        bad.name = "Bad".into();
        bad.shortcut = None;
        bad.commands = vec![MacroCommand::Expect {
            pattern: "(".into(),
            delay_ms: 0,
            timeout_ms: 1,
        }];
        assert!(s.upsert(None, bad).is_err());
        assert_eq!(s.fresh_name("Renamed"), "Renamed (2)");
        assert!(s.remove("Renamed"));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m/macros.json");
        s.save(&path).unwrap();
        assert_eq!(MacroStore::load(&path).unwrap(), s);
        assert_eq!(
            MacroStore::load(&dir.path().join("none.json")).unwrap(),
            MacroStore::default()
        );
    }

    #[test]
    fn first_run_import_prefers_c_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("macros.json");
        let kf = dir.path().join(".gtktermrc");
        std::fs::write(&kf, "[default]\nmacros = F1;hi;\n").unwrap();
        let (s, msg) = load_or_import(&target, Some(&dir.path().join("missing.json")), Some(&kf)).unwrap();
        assert_eq!(s.list().len(), 1);
        assert!(msg.unwrap().contains("Imported 1 macro"));
        let (again, msg) = load_or_import(&target, None, Some(&kf)).unwrap();
        assert_eq!((again, msg), (s, None));
    }
}
