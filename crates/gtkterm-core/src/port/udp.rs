//! UDP port. With no configured remote, the sender of the first datagram becomes the remote.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tracing::Instrument as _;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::net::{UdpSocket, lookup_host};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{OpenPort, PortError, PortKind, PortRx, PortWriter, RX_CHANNEL};

/// Largest datagram received in one piece.
const MAX_DATAGRAM: usize = 65_535;

/// Bind `bind:local_port`, optionally fixing the remote.
pub(crate) async fn open(
    bind: &str,
    local_port: u16,
    remote_host: Option<&str>,
    remote_port: u16,
    max_datagram: u16,
) -> Result<OpenPort, PortError> {
    let max_send = usize::from(match max_datagram {
        0 => crate::config::MAX_UDP_PAYLOAD,
        n => n.min(crate::config::MAX_UDP_PAYLOAD),
    });
    let remote = match remote_host {
        Some(host) => Some(
            lookup_host((host, remote_port))
                .await
                .map_err(|e| PortError::io(format!("cannot resolve {host}"), e))?
                .next()
                .ok_or_else(|| PortError::Config(format!("{host} has no address")))?,
        ),
        None => None,
    };
    // Bind an address family that can reach the remote.
    let bind = match (remote, bind) {
        (Some(SocketAddr::V6(_)), "0.0.0.0") => "::",
        _ => bind,
    };
    let socket = Arc::new(
        UdpSocket::bind((bind, local_port))
            .await
            .map_err(|e| PortError::io(format!("cannot bind UDP {bind}:{local_port}"), e))?,
    );
    let local = socket
        .local_addr()
        .map_err(|e| PortError::io("local address", e))?;
    let remote = Arc::new(Mutex::new(remote));
    let cancel = CancellationToken::new();
    let (tx, rx) = mpsc::channel(RX_CHANNEL);

    let (sock, rem, token) = (Arc::clone(&socket), Arc::clone(&remote), cancel.clone());
    let task = tokio::spawn(async move {
        let mut buf = vec![0u8; MAX_DATAGRAM];
        loop {
            let (n, from) = tokio::select! {
                () = token.cancelled() => return,
                r = sock.recv_from(&mut buf) => match r {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = tx.send(PortRx::Closed(Some(Arc::new(PortError::io("UDP receive", e))))).await;
                        return;
                    }
                },
            };
            let learned = {
                let mut r = rem.lock().expect("remote mutex poisoned");
                r.is_none().then(|| *r = Some(from)).is_some()
            };
            if learned
                && tx
                    .send(PortRx::Info(format!("remote set to {from}")))
                    .await
                    .is_err()
            {
                return;
            }
            if tx
                .send(PortRx::Data(Bytes::copy_from_slice(&buf[..n])))
                .await
                .is_err()
            {
                return;
            }
        }
    }.in_current_span());

    let remote_desc = remote
        .lock()
        .expect("fresh mutex")
        .map_or_else(|| "*".to_owned(), |r| r.to_string());
    Ok(OpenPort::new(
        PortKind::Udp,
        format!("UDP:{local}->{remote_desc}"),
        rx,
        Box::new(UdpWriter {
            socket,
            remote,
            max_send,
        }),
        None,
        cancel,
        Some(task),
    ))
}

#[derive(Debug)]
struct UdpWriter {
    socket: Arc<UdpSocket>,
    remote: Arc<Mutex<Option<SocketAddr>>>,
    /// Largest datagram payload.
    max_send: usize,
}

#[async_trait]
impl PortWriter for UdpWriter {
    async fn write_all(&mut self, data: &[u8]) -> Result<(), PortError> {
        let remote = (*self.remote.lock().expect("remote mutex poisoned")).ok_or(PortError::NoRemote)?;
        for chunk in data.chunks(self.max_send) {
            self.socket
                .send_to(chunk, remote)
                .await
                .map_err(|e| PortError::io(format!("UDP send to {remote}"), e))?;
        }
        Ok(())
    }

    fn is_stream(&self) -> bool {
        false
    }
}
