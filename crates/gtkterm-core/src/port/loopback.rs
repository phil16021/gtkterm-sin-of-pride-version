//! In-memory port for tests: the [`DeviceEnd`] plays the device.

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{OpenPort, PortError, PortKind, PortRx, PortWriter, RX_CHANNEL};

/// The far end of a loopback port.
#[derive(Debug)]
pub struct DeviceEnd {
    to_host: mpsc::Sender<PortRx>,
    from_host: mpsc::UnboundedReceiver<Bytes>,
}

impl DeviceEnd {
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
    (port, DeviceEnd { to_host, from_host })
}
