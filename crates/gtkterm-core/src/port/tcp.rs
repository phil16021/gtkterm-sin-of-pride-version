//! TCP client and server ports.
//!
//! The client connects asynchronously with a timeout (the C version blocked the UI thread).
//! The server accepts any number of clients: received data from all of them is merged and
//! writes are broadcast; a client disconnecting does not close the server (ROADMAP defect T1).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::{OpenPort, PortError, PortKind, PortRx, PortWriter, READ_SIZE, RX_CHANNEL};

/// Connect to `host:port` within `timeout`.
pub(crate) async fn connect(host: &str, port: u16, timeout: Duration) -> Result<OpenPort, PortError> {
    let endpoint = format!("{host}:{port}");
    let stream = tokio::time::timeout(timeout, TcpStream::connect((host, port)))
        .await
        .map_err(|_| PortError::Timeout(endpoint.clone()))?
        .map_err(|e| PortError::io(format!("cannot connect to {endpoint}"), e))?;
    let _ = stream.set_nodelay(true);
    let (read, write) = stream.into_split();
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel(RX_CHANNEL);
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        let reason = read_stream(read, &tx, &token, None).await;
        let _ = tx.send(PortRx::Closed(reason.map(Arc::new))).await;
    });
    Ok(OpenPort::new(
        PortKind::TcpClient,
        format!("TCP:{endpoint}"),
        rx,
        Box::new(ClientWriter(write)),
        None,
        cancel,
        Some(task),
    ))
}

/// Read until EOF/error/cancel. Returns the error, if any; `None` on EOF or cancel.
async fn read_stream(
    mut read: OwnedReadHalf,
    tx: &mpsc::Sender<PortRx>,
    cancel: &CancellationToken,
    peer: Option<SocketAddr>,
) -> Option<PortError> {
    let mut buf = vec![0u8; READ_SIZE];
    loop {
        let n = tokio::select! {
            () = cancel.cancelled() => return None,
            r = read.read(&mut buf) => match r {
                Ok(0) => return None,
                Ok(n) => n,
                Err(e) => return Some(PortError::io(peer.map_or_else(|| "TCP read".into(), |p| format!("TCP read from {p}")), e)),
            },
        };
        if tx
            .send(PortRx::Data(Bytes::copy_from_slice(&buf[..n])))
            .await
            .is_err()
        {
            return None;
        }
    }
}

#[derive(Debug)]
struct ClientWriter(OwnedWriteHalf);

#[async_trait]
impl PortWriter for ClientWriter {
    async fn write_all(&mut self, data: &[u8]) -> Result<(), PortError> {
        self.0
            .write_all(data)
            .await
            .map_err(|e| PortError::io("TCP write", e))
    }
}

type Clients = Arc<Mutex<Vec<(SocketAddr, OwnedWriteHalf)>>>;

/// Listen on `bind:port`.
pub(crate) async fn listen(bind: &str, port: u16) -> Result<OpenPort, PortError> {
    let listener = TcpListener::bind((bind, port))
        .await
        .map_err(|e| PortError::io(format!("cannot listen on {bind}:{port}"), e))?;
    let local = listener
        .local_addr()
        .map_err(|e| PortError::io("local address", e))?;
    let clients: Clients = Arc::default();
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel(RX_CHANNEL);
    let task = tokio::spawn(accept_loop(listener, Arc::clone(&clients), tx, cancel.clone()));
    Ok(OpenPort::new(
        PortKind::TcpServer,
        format!("TCP-Server:{}", local.port()),
        rx,
        Box::new(ServerWriter { clients }),
        None,
        cancel,
        Some(task),
    ))
}

async fn accept_loop(
    listener: TcpListener,
    clients: Clients,
    tx: mpsc::Sender<PortRx>,
    cancel: CancellationToken,
) {
    loop {
        let (stream, peer) = tokio::select! {
            () = cancel.cancelled() => return,
            r = listener.accept() => match r {
                Ok(c) => c,
                Err(e) => {
                    debug!(error = %e, "accept failed");
                    continue;
                }
            },
        };
        let _ = stream.set_nodelay(true);
        let (read, write) = stream.into_split();
        clients.lock().await.push((peer, write));
        if tx
            .send(PortRx::Info(format!("client {peer} connected")))
            .await
            .is_err()
        {
            return;
        }
        let (tx, clients, cancel) = (tx.clone(), Arc::clone(&clients), cancel.clone());
        tokio::spawn(async move {
            let reason = read_stream(read, &tx, &cancel, Some(peer)).await;
            clients.lock().await.retain(|(p, _)| *p != peer);
            let msg = match reason {
                Some(e) => format!("client {peer} disconnected: {e}"),
                None => format!("client {peer} disconnected"),
            };
            let _ = tx.send(PortRx::Info(msg)).await;
        });
    }
}

#[derive(Debug)]
struct ServerWriter {
    clients: Clients,
}

#[async_trait]
impl PortWriter for ServerWriter {
    async fn write_all(&mut self, data: &[u8]) -> Result<(), PortError> {
        let mut clients = self.clients.lock().await;
        let mut alive = Vec::with_capacity(clients.len());
        for (peer, mut w) in clients.drain(..) {
            match w.write_all(data).await {
                Ok(()) => alive.push((peer, w)),
                Err(e) => debug!(%peer, error = %e, "dropping TCP client after write error"),
            }
        }
        *clients = alive;
        Ok(())
    }
}
