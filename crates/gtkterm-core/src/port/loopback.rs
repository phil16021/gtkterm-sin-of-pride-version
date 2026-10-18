//! In-memory port for tests: the [`DeviceEnd`] plays the device.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::{
    LineCounters, ModemLines, OpenPort, PortError, PortKind, PortRx, PortWriter, RX_CHANNEL, SerialControl,
};

/// Simulated modem lines: the host drives DTR/RTS, the device drives CTS/DSR/DCD/RI.
#[derive(Debug)]
struct FakeLines {
    state: watch::Sender<ModemLines>,
    counters: Mutex<LineCounters>,
}

impl SerialControl for FakeLines {
    fn modem_lines(&self) -> Result<ModemLines, PortError> {
        Ok(*self.state.borrow())
    }

    fn set_dtr(&self, on: bool) -> Result<(), PortError> {
        self.state.send_modify(|m| m.dtr = on);
        Ok(())
    }

    fn set_rts(&self, on: bool) -> Result<(), PortError> {
        self.state.send_modify(|m| m.rts = on);
        Ok(())
    }

    fn set_break(&self, _on: bool) -> Result<(), PortError> {
        Ok(())
    }

    fn counters(&self) -> Result<LineCounters, PortError> {
        Ok(*self
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    fn flush(&self) -> Result<(), PortError> {
        Ok(())
    }
}

/// The far end of a loopback port.
#[derive(Debug)]
pub struct DeviceEnd {
    to_host: mpsc::Sender<PortRx>,
    from_host: mpsc::UnboundedReceiver<Bytes>,
    lines: Option<Arc<FakeLines>>,
}

/// Device-side input lines of [`pair_with_lines`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct InputLines {
    /// Clear To Send.
    pub cts: bool,
    /// Data Set Ready.
    pub dsr: bool,
    /// Data Carrier Detect.
    pub dcd: bool,
    /// Ring Indicator.
    pub ri: bool,
}

impl DeviceEnd {
    fn fake(&self) -> &FakeLines {
        self.lines.as_deref().expect("created with pair_with_lines")
    }

    /// Set the device's output lines (the host's CTS/DSR/DCD/RI); counts transitions.
    pub fn set_inputs(&self, i: InputLines) {
        let f = self.fake();
        let old = *f.state.borrow();
        {
            let mut c = f
                .counters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            c.cts += u64::from(old.cts != i.cts);
            c.dsr += u64::from(old.dsr != i.dsr);
            c.dcd += u64::from(old.dcd != i.dcd);
            c.ri += u64::from(old.ri != i.ri);
        }
        f.state.send_modify(|m| {
            m.cts = i.cts;
            m.dsr = i.dsr;
            m.dcd = i.dcd;
            m.ri = i.ri;
        });
    }

    /// A pulse too short for polling: the level is unchanged but the counter says it toggled twice.
    pub fn pulse_ri(&self) {
        self.fake()
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ri += 2;
    }

    /// Current lines (the host's DTR/RTS included).
    pub fn lines(&self) -> ModemLines {
        *self.fake().state.borrow()
    }

    /// Watch the lines, e.g. to react to DTR resets.
    pub fn watch_lines(&self) -> watch::Receiver<ModemLines> {
        self.fake().state.subscribe()
    }

    /// Send bytes as if the device transmitted them.
    pub async fn send(&self, data: impl Into<Bytes>) -> Result<(), PortError> {
        self.to_host
            .send(PortRx::Data(data.into()))
            .await
            .map_err(|_| PortError::Closed)
    }

    /// Simulate the device going away.
    pub async fn disconnect(&self) {
        let _ = self.to_host.send(PortRx::Closed(None)).await;
    }

    /// Next chunk the host wrote, if any arrives.
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.from_host.recv().await
    }

    /// Everything the host has written so far, without waiting.
    pub fn drain(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Ok(b) = self.from_host.try_recv() {
            out.extend_from_slice(&b);
        }
        out
    }
}

#[derive(Debug)]
struct LoopbackWriter(mpsc::UnboundedSender<Bytes>);

#[async_trait]
impl PortWriter for LoopbackWriter {
    async fn write_all(&mut self, data: &[u8]) -> Result<(), PortError> {
        self.0
            .send(Bytes::copy_from_slice(data))
            .map_err(|_| PortError::Closed)
    }
}

/// Create a connected loopback port and its device end.
pub fn pair(description: &str) -> (OpenPort, DeviceEnd) {
    let (to_host, rx) = mpsc::channel(RX_CHANNEL);
    let (tx, from_host) = mpsc::unbounded_channel();
    let port = OpenPort::new(
        PortKind::Loopback,
        description.to_owned(),
        rx,
        Box::new(LoopbackWriter(tx)),
        None,
        CancellationToken::new(),
        None,
    );
    (
        port,
        DeviceEnd {
            to_host,
            from_host,
            lines: None,
        },
    )
}

/// Like [`pair`], with simulated modem lines (DTR/RTS set by the host are visible on the
/// [`DeviceEnd`], which drives CTS/DSR/DCD/RI).
pub fn pair_with_lines(description: &str) -> (OpenPort, DeviceEnd) {
    let (to_host, rx) = mpsc::channel(RX_CHANNEL);
    let (tx, from_host) = mpsc::unbounded_channel();
    let lines = Arc::new(FakeLines {
        state: watch::channel(ModemLines::default()).0,
        counters: Mutex::default(),
    });
    let control: Arc<dyn SerialControl> = lines.clone();
    let port = OpenPort::new(
        PortKind::Loopback,
        description.to_owned(),
        rx,
        Box::new(LoopbackWriter(tx)),
        Some(control),
        CancellationToken::new(),
        None,
    );
    (
        port,
        DeviceEnd {
            to_host,
            from_host,
            lines: Some(lines),
        },
    )
}
