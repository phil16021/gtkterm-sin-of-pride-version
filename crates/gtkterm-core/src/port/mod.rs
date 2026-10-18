//! Ports: serial devices, TCP client/server, UDP and an in-memory loopback.
//!
//! Every backend is opened into an [`OpenPort`]: received data arrives on an mpsc channel fed
//! by a backend task, writes go through a [`PortWriter`], and serial devices additionally
//! expose [`SerialControl`] for modem lines and break. Dropping the `OpenPort` cancels the
//! backend tasks and closes the device or sockets.

pub mod loopback;
pub mod serial;
pub mod tcp;
pub mod udp;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::config::Connection;

/// Capacity of the receive channel between a backend task and the session.
pub(crate) const RX_CHANNEL: usize = 256;
/// Size of a single read.
pub(crate) const READ_SIZE: usize = 8192;

/// Kind of an open port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortKind {
    /// Serial device.
    Serial,
    /// TCP client connection.
    TcpClient,
    /// TCP server.
    TcpServer,
    /// UDP socket.
    Udp,
    /// In-memory loopback (tests).
    Loopback,
}

/// Port errors.
#[derive(Debug, Error)]
pub enum PortError {
    /// An OS error with context.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// Cause.
        #[source]
        source: std::io::Error,
    },
    /// Another process holds the device lock.
    #[error("{0} is locked by another program")]
    Locked(String),
    /// Connect timeout.
    #[error("connection to {0} timed out")]
    Timeout(String),
    /// UDP write without a known remote.
    #[error("no remote address known yet; wait for a datagram or configure a remote host")]
    NoRemote,
    /// Operation not supported by this port.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// The port is closed.
    #[error("port is closed")]
    Closed,
    /// Invalid settings.
    #[error("invalid port settings: {0}")]
    Config(String),
}

impl PortError {
    /// The connection itself is broken (close the port), as opposed to one write that
    /// could not be delivered (no UDP remote yet, datagram too large, ICMP unreachable).
    pub fn is_fatal(&self) -> bool {
        match self {
            Self::NoRemote | Self::Unsupported(_) | Self::Config(_) => false,
            Self::Io { source, .. } => !matches!(
                source.raw_os_error(),
                Some(
                    nix::libc::EMSGSIZE
                        | nix::libc::ECONNREFUSED
                        | nix::libc::EHOSTUNREACH
                        | nix::libc::ENETUNREACH
                        | nix::libc::ENOBUFS
                )
            ),
            Self::Locked(_) | Self::Timeout(_) | Self::Closed => true,
        }
    }

    pub(crate) fn io(context: impl Into<String>, source: impl Into<std::io::Error>) -> Self {
        Self::Io {
            context: context.into(),
            source: source.into(),
        }
    }
}

/// Something received from a port.
#[derive(Debug, Clone)]
pub enum PortRx {
    /// Received bytes.
    Data(Bytes),
    /// Informational event, e.g. "client 10.0.0.2:5123 connected".
    Info(String),
    /// The port closed; `Some` if because of an error.
    Closed(Option<Arc<PortError>>),
}

/// Write half of a port.
#[async_trait]
pub trait PortWriter: Send + fmt::Debug {
    /// Write all bytes (partial writes are retried until done).
    async fn write_all(&mut self, data: &[u8]) -> Result<(), PortError>;

    /// True for byte streams (serial, TCP): large sends may be written in pieces. Datagram
    /// ports keep one send in one write, since piece boundaries would become datagrams.
    fn is_stream(&self) -> bool {
        true
    }
}

/// Modem control and status lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ModemLines {
    /// Data Terminal Ready (output).
    pub dtr: bool,
    /// Request To Send (output).
    pub rts: bool,
    /// Clear To Send (input).
    pub cts: bool,
    /// Data Set Ready (input).
    pub dsr: bool,
    /// Data Carrier Detect (input).
    pub dcd: bool,
    /// Ring Indicator (input).
    pub ri: bool,
}

/// Kernel line counters (`TIOCGICOUNT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LineCounters {
    /// Bytes received by the driver.
    pub rx: u64,
    /// Bytes transmitted by the driver.
    pub tx: u64,
    /// Framing errors.
    pub frame: u64,
    /// Overrun errors.
    pub overrun: u64,
    /// Parity errors.
    pub parity: u64,
    /// Breaks received.
    pub brk: u64,
    /// Buffer overruns.
    pub buf_overrun: u64,
    /// CTS transitions.
    pub cts: u64,
    /// DSR transitions.
    pub dsr: u64,
    /// RI transitions.
    pub ri: u64,
    /// DCD transitions.
    pub dcd: u64,
}

/// Serial-only controls. Methods are synchronous ioctls; blocking ones say so.
pub trait SerialControl: Send + Sync + fmt::Debug {
    /// Read modem lines.
    fn modem_lines(&self) -> Result<ModemLines, PortError>;
    /// Set or clear DTR.
    fn set_dtr(&self, on: bool) -> Result<(), PortError>;
    /// Set or clear RTS.
    fn set_rts(&self, on: bool) -> Result<(), PortError>;
    /// Start or stop sending a break condition.
    fn set_break(&self, on: bool) -> Result<(), PortError>;
    /// Kernel line counters.
    fn counters(&self) -> Result<LineCounters, PortError>;
    /// Discard data in the driver's input and output buffers.
    fn flush(&self) -> Result<(), PortError>;
    /// Block until a modem input line changes (`TIOCMIWAIT`). Call from a blocking thread.
    fn wait_modem_change(&self) -> Result<(), PortError>;
}

/// An open port.
pub struct OpenPort {
    /// Backend kind.
    pub kind: PortKind,
    /// Description for status bar and logs.
    pub description: String,
    /// Received data and events.
    pub rx: mpsc::Receiver<PortRx>,
    /// Write half.
    pub writer: Box<dyn PortWriter>,
    /// Serial controls, if any.
    pub control: Option<Arc<dyn SerialControl>>,
    task: Option<JoinHandle<()>>,
    cancel: Option<DropGuard>,
}

impl fmt::Debug for OpenPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenPort")
            .field("kind", &self.kind)
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

impl OpenPort {
    pub(crate) fn new(
        kind: PortKind,
        description: String,
        rx: mpsc::Receiver<PortRx>,
        writer: Box<dyn PortWriter>,
        control: Option<Arc<dyn SerialControl>>,
        cancel: CancellationToken,
        task: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            kind,
            description,
            rx,
            writer,
            control,
            task,
            cancel: Some(cancel.drop_guard()),
        }
    }

    /// Close the port and wait until the backend released the device.
    ///
    /// Dropping an `OpenPort` also closes it, but asynchronously: a serial device may stay
    /// locked for a moment. Use `close` before reopening the same device.
    pub async fn close(mut self) {
        drop(self.cancel.take());
        let task = self.task.take();
        drop(self);
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    /// Split into independently owned parts (used by the session actor).
    pub fn into_parts(mut self) -> PortParts {
        let closer = PortCloser {
            cancel: self.cancel.take(),
            task: self.task.take(),
        };
        let Self {
            kind,
            description,
            rx,
            writer,
            control,
            ..
        } = self;
        PortParts {
            kind,
            description,
            rx,
            writer,
            control,
            closer,
        }
    }

    /// Send a break of the given duration (serial only).
    pub async fn send_break(&self, duration: Duration) -> Result<(), PortError> {
        let control = self
            .control
            .clone()
            .ok_or_else(|| PortError::Unsupported("break on non-serial port".into()))?;
        control.set_break(true)?;
        tokio::time::sleep(duration).await;
        control.set_break(false)
    }
}

/// An [`OpenPort`] taken apart.
#[derive(Debug)]
pub struct PortParts {
    /// Backend kind.
    pub kind: PortKind,
    /// Description.
    pub description: String,
    /// Received data and events.
    pub rx: mpsc::Receiver<PortRx>,
    /// Write half.
    pub writer: Box<dyn PortWriter>,
    /// Serial controls, if any.
    pub control: Option<Arc<dyn SerialControl>>,
    /// Closes the backend.
    pub closer: PortCloser,
}

/// Cancels the backend of a port; dropping it does the same without waiting.
#[derive(Debug)]
pub struct PortCloser {
    cancel: Option<DropGuard>,
    task: Option<JoinHandle<()>>,
}

impl PortCloser {
    /// Cancel and wait for the backend task. Drop every other part first.
    pub async fn close(mut self) {
        drop(self.cancel.take());
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

/// Open the port described by `connection`.
pub async fn open(connection: &Connection) -> Result<OpenPort, PortError> {
    match connection {
        Connection::Serial(cfg) => serial::open(cfg),
        Connection::TcpClient {
            host,
            port,
            connect_timeout_ms,
        } => tcp::connect(host, *port, Duration::from_millis(*connect_timeout_ms)).await,
        Connection::TcpServer { bind, port } => tcp::listen(bind, *port).await,
        Connection::Udp {
            bind,
            local_port,
            remote_host,
            remote_port,
        } => udp::open(bind, *local_port, remote_host.as_deref(), *remote_port).await,
    }
}
