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
use tokio::sync::mpsc;
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

/// Most clients a TCP server serves at once; later connections are refused.
const MAX_CLIENTS: usize = 32;
/// Writes queued per client; a client that falls this far behind is dropped.
const CLIENT_QUEUE: usize = 256;

/// A connected client: its write queue and the token that ends both of its tasks.
#[derive(Debug)]
struct Client {
    peer: SocketAddr,
    queue: mpsc::Sender<Bytes>,
    cancel: CancellationToken,
}

type Clients = Arc<std::sync::Mutex<Vec<Client>>>;

fn lock_clients(c: &Clients) -> std::sync::MutexGuard<'_, Vec<Client>> {
    c.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

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
    let task = tokio::spawn(accept_loop(
        listener,
        Arc::clone(&clients),
        tx.clone(),
        cancel.clone(),
    ));
    Ok(OpenPort::new(
        PortKind::TcpServer,
        format!("TCP-Server:{}", local.port()),
        rx,
        Box::new(ServerWriter { clients, info: tx }),
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
                    // Out of file descriptors and the like: do not spin.
                    debug!(error = %e, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        if lock_clients(&clients).len() >= MAX_CLIENTS {
            drop(stream);
            let _ = tx
                .send(PortRx::Info(format!(
                    "client {peer} refused: {MAX_CLIENTS} clients connected"
                )))
                .await;
            continue;
        }
        let _ = stream.set_nodelay(true);
        let (read, write) = stream.into_split();
        let (queue, pending) = mpsc::channel(CLIENT_QUEUE);
        let client_cancel = cancel.child_token();
        lock_clients(&clients).push(Client {
            peer,
            queue,
            cancel: client_cancel.clone(),
        });
        if tx
            .send(PortRx::Info(format!("client {peer} connected")))
            .await
            .is_err()
        {
            return;
        }
        tokio::spawn(client_writer(write, pending, client_cancel.clone()));
        let (tx, clients) = (tx.clone(), Arc::clone(&clients));
        tokio::spawn(async move {
            let reason = read_stream(read, &tx, &client_cancel, Some(peer)).await;
            client_cancel.cancel();
            lock_clients(&clients).retain(|c| c.peer != peer);
            let msg = match reason {
                Some(e) => format!("client {peer} disconnected: {e}"),
                None => format!("client {peer} disconnected"),
            };
            let _ = tx.send(PortRx::Info(msg)).await;
        });
    }
}

/// Deliver one client's queue; a write error ends the client.
async fn client_writer(
    mut write: OwnedWriteHalf,
    mut pending: mpsc::Receiver<Bytes>,
    cancel: CancellationToken,
) {
    loop {
        let data = tokio::select! {
            () = cancel.cancelled() => return,
            d = pending.recv() => match d {
                Some(d) => d,
                None => return,
            },
        };
        if let Err(e) = write.write_all(&data).await {
            debug!(error = %e, "TCP client write failed");
            cancel.cancel();
            return;
        }
    }
}

#[derive(Debug)]
struct ServerWriter {
    clients: Clients,
    info: mpsc::Sender<PortRx>,
}

#[async_trait]
impl PortWriter for ServerWriter {
    /// Queue `data` for every client without waiting for any of them: a client that stops
    /// reading must not hold up the others (or the session).
    async fn write_all(&mut self, data: &[u8]) -> Result<(), PortError> {
        let data = Bytes::copy_from_slice(data);
        let mut dropped = Vec::new();
        lock_clients(&self.clients).retain(|c| {
            if c.queue.try_send(data.clone()).is_ok() {
                return true;
            }
            c.cancel.cancel();
            dropped.push(c.peer);
            false
        });
        for peer in dropped {
            let _ = self
                .info
                .send(PortRx::Info(format!("client {peer} dropped: not reading")))
                .await;
        }
        Ok(())
    }
}
