//! What agents did and may do: the JSONL audit log of tool calls and the approval hook that
//! lets the GUI ask the user before an agent changes something.

use std::future::Future;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use serde::Serialize;
use serde_json::{Map, Value};

/// Audit files larger than this are moved to `<name>.1` before the next write.
const AUDIT_ROTATE: u64 = 8 << 20;
/// Arguments longer than this (as JSON) are cut in the audit log.
const AUDIT_MAX_ARGS: usize = 4096;

/// A tool call waiting for the user's decision.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    /// Tool name, e.g. `send`.
    pub tool: String,
    /// The arguments as compact JSON (cut to a readable length).
    pub arguments: String,
    /// The client's name from its initialize request, when known.
    pub client: Option<String>,
}

/// The user's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// Refuse this call.
    Deny,
    /// Run this call; ask again next time.
    Once,
    /// Run this and every later call until the server stops.
    Always,
}

type ApproveFn = dyn Fn(ApprovalRequest) -> Pin<Box<dyn Future<Output = Approval> + Send>> + Send + Sync;

/// Asks whether an agent may run a tool from [`WRITE_TOOLS`](crate::WRITE_TOOLS).
#[derive(Clone)]
pub struct Approver(Arc<ApproveFn>);

impl Approver {
    /// Wrap a callback; it is awaited before every write tool until it answers [`Approval::Always`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(ApprovalRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Approval> + Send + 'static,
    {
        Self(Arc::new(move |r| Box::pin(f(r))))
    }

    pub(crate) async fn ask(&self, request: ApprovalRequest) -> Approval {
        (self.0)(request).await
    }
}

impl std::fmt::Debug for Approver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Approver")
    }
}

/// Compact JSON of tool arguments, cut to `max` characters.
pub(crate) fn compact_arguments(args: Option<&Map<String, Value>>, max: usize) -> String {
    let text = args.map_or_else(|| "{}".to_owned(), |a| Value::Object(a.clone()).to_string());
    if text.chars().count() <= max {
        text
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}…")
    }
}

/// One line of the audit log.
#[derive(Debug, Serialize)]
pub(crate) struct AuditEntry<'a> {
    pub time: Timestamp,
    pub tool: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<&'a str>,
    pub arguments: Value,
    /// `ok`, `error` or `denied`.
    pub outcome: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub ms: u64,
}

/// Arguments as stored in the audit log: the JSON object, or a cut string when it is long.
pub(crate) fn audit_arguments(args: Option<&Map<String, Value>>) -> Value {
    let text = compact_arguments(args, usize::MAX);
    if text.len() <= AUDIT_MAX_ARGS {
        args.map_or(Value::Object(Map::new()), |a| Value::Object(a.clone()))
    } else {
        Value::String(compact_arguments(args, AUDIT_MAX_ARGS))
    }
}

/// Append-only JSONL file, readable only by the user.
#[derive(Debug)]
pub(crate) struct AuditLog {
    path: PathBuf,
    /// Serialises writes (and the size check before each one).
    writing: Mutex<()>,
}

impl AuditLog {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            writing: Mutex::new(()),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Append `entry`; failures are logged, never returned (auditing must not break tools).
    pub(crate) fn write(&self, entry: &AuditEntry<'_>) {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _writing = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // Checked at every write: a long-running window must not grow the file forever.
            if std::fs::metadata(&self.path).is_ok_and(|m| m.len() > AUDIT_ROTATE) {
                let mut old = self.path.clone().into_os_string();
                old.push(".1");
                std::fs::rename(&self.path, old)?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&self.path)?;
            let mut line = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
            line.push(b'\n');
            f.write_all(&line)
        })();
        if let Err(e) = result {
            tracing::warn!(path = %self.path.display(), error = %e, "cannot write the MCP audit log");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_cut() {
        let mut m = Map::new();
        m.insert("data".into(), Value::String("x".repeat(10_000)));
        assert!(compact_arguments(Some(&m), 20).ends_with('…'));
        assert!(
            matches!(audit_arguments(Some(&m)), Value::String(s) if s.chars().count() == AUDIT_MAX_ARGS + 1)
        );
        m.insert("data".into(), Value::String("AT".into()));
        assert_eq!(audit_arguments(Some(&m))["data"], "AT");
        assert_eq!(compact_arguments(None, 10), "{}");
    }
}
