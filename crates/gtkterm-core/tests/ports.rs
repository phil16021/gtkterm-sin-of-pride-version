//! Integration tests for the port backends on PTYs and loopback sockets.

use std::time::Duration;

use gtkterm_core::config::{Connection, FlowControl, Parity, SerialConfig};
use gtkterm_core::port::{self, OpenPort, PortError, PortRx};
use gtkterm_core::testing::PtyPair;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::timeout;

const T: Duration = Duration::from_secs(5);

fn serial(path: &str) -> Connection {
    Connection::Serial(SerialConfig {
        port: path.to_owned(),
        ..SerialConfig::default()
    })
}

/// Collect received data until `n` bytes arrived.
async fn recv_bytes(port: &mut OpenPort, n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while out.len() < n {
        match timeout(T, port.rx.recv())
            .await
            .expect("timed out waiting for data")
        {
            Some(PortRx::Data(b) | PortRx::DataFrom(b, _)) => out.extend_from_slice(&b),
            Some(PortRx::Info(_)) => {}
            other => panic!("unexpected {other:?} after {} bytes", out.len()),
        }
    }
    out
}

async fn next_info(port: &mut OpenPort) -> String {
    loop {
        match timeout(T, port.rx.recv())
            .await
            .expect("timed out waiting for info")
        {
            Some(PortRx::Info(s)) => return s,
            Some(PortRx::Data(_) | PortRx::DataFrom(..)) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
}

#[tokio::test]
async fn serial_rx_tx_on_pty() {
    let pty = PtyPair::new().unwrap();
    let mut port = port::open(&serial(&pty.path_str())).await.unwrap();
    pty.write(b"hello\r\n").await.unwrap();
    assert_eq!(recv_bytes(&mut port, 7).await, b"hello\r\n");
    port.writer.write_all(b"AT\r").await.unwrap();
    assert_eq!(pty.read_exact(3).await.unwrap(), b"AT\r");
}

#[tokio::test]
async fn serial_one_megabyte_each_way_is_exact() {
    // C defects B1/B2/P4: binary data without newlines and large writes.
    let pty = PtyPair::new().unwrap();
    let mut port = port::open(&serial(&pty.path_str())).await.unwrap();
    let data = pattern(1 << 20);

    let reader = async { pty.read_exact(data.len()).await.unwrap() };
    let ((), received) = tokio::join!(async { port.writer.write_all(&data).await.unwrap() }, reader);
    assert!(received == data, "host->device data differs");

    let writer = async { pty.write(&data).await.unwrap() };
    let (received, ()) = tokio::join!(recv_bytes(&mut port, data.len()), writer);
    assert!(received == data, "device->host data differs");
}

#[tokio::test]
async fn serial_lock_conflict() {
    let pty = PtyPair::new().unwrap();
    let _first = port::open(&serial(&pty.path_str())).await.unwrap();
    let err = port::open(&serial(&pty.path_str())).await.unwrap_err();
    assert!(matches!(err, PortError::Locked(_)), "{err}");

    let unlocked = Connection::Serial(SerialConfig {
        port: pty.path_str(),
        disable_port_lock: true,
        ..SerialConfig::default()
    });
    port::open(&unlocked).await.expect("lock disabled");
}

#[tokio::test]
async fn serial_termios_readback_custom_baud_and_format() {
    let pty = PtyPair::new().unwrap();
    let cfg = SerialConfig {
        port: pty.path_str(),
        baudrate: 250_000,
        bits: 7,
        stopbits: 2,
        parity: Parity::Even,
        flow_control: FlowControl::RtsCts,
        ..SerialConfig::default()
    };
    let _port = port::open(&Connection::Serial(cfg)).await.unwrap();
    let t = gtkterm_core::port::serial::read_termios(pty.slave_fd().unwrap()).unwrap();
    // The pty driver forces CS8 and clears PARENB, so data bits and parity cannot be read back
    // here; `configure_termios` unit tests cover them. Everything else must stick.
    assert_eq!(t.ospeed, 250_000);
    assert_ne!(t.cflag & libc::CSTOPB, 0);
    assert_ne!(t.cflag & libc::CRTSCTS, 0);
    assert_eq!(t.iflag & libc::IXON, 0);
    assert_eq!(t.oflag & libc::OPOST, 0);
    assert_eq!(t.lflag & (libc::ICANON | libc::ECHO), 0);
}

#[tokio::test]
async fn serial_reopen_many_times() {
    let pty = PtyPair::new().unwrap();
    for i in 0..50 {
        let mut port = port::open(&serial(&pty.path_str()))
            .await
            .unwrap_or_else(|e| panic!("open #{i}: {e}"));
        let msg = format!("#{i}\n");
        pty.write(msg.as_bytes()).await.unwrap();
        assert_eq!(recv_bytes(&mut port, msg.len()).await, msg.as_bytes());
        port.close().await;
    }
}

#[tokio::test]
async fn serial_hangup_reports_closed() {
    let mut pty = PtyPair::new().unwrap();
    let mut port = port::open(&serial(&pty.path_str())).await.unwrap();
    pty.release_slave();
    pty.hang_up();
    let ev = timeout(T, port.rx.recv()).await.unwrap();
    assert!(matches!(ev, Some(PortRx::Closed(_))), "{ev:?}");
}

#[tokio::test]
async fn serial_open_errors() {
    let err = port::open(&serial("/dev/does-not-exist")).await.unwrap_err();
    assert!(
        err.to_string().contains("cannot open /dev/does-not-exist"),
        "{err}"
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain");
    std::fs::write(&file, b"x").unwrap();
    let err = port::open(&serial(file.to_str().unwrap())).await.unwrap_err();
    assert!(err.to_string().contains("is not a serial device"), "{err}");
}

#[tokio::test]
async fn pty_has_no_modem_lines_but_reports_it_cleanly() {
    let pty = PtyPair::new().unwrap();
    let port = port::open(&serial(&pty.path_str())).await.unwrap();
    let ctl = port.control.clone().expect("serial port has controls");
    match ctl.modem_lines() {
        Ok(_) | Err(PortError::Unsupported(_)) => {}
        Err(e) => panic!("unexpected error {e}"),
    }
}

#[tokio::test]
async fn tcp_client_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4];
        s.read_exact(&mut buf).await.unwrap();
        s.write_all(&buf).await.unwrap();
        s.write_all(b"!").await.unwrap();
    });
    let conn = Connection::TcpClient {
        host: "localhost".into(),
        port: addr.port(),
        connect_timeout_ms: 2_000,
    };
    let mut port = port::open(&conn).await.unwrap();
    assert_eq!(port.description, format!("TCP:localhost:{}", addr.port()));
    port.writer.write_all(b"ping").await.unwrap();
    assert_eq!(recv_bytes(&mut port, 5).await, b"ping!");
    server.await.unwrap();
    let ev = timeout(T, port.rx.recv()).await.unwrap();
    assert!(matches!(ev, Some(PortRx::Closed(None))), "{ev:?}");
}

#[tokio::test]
async fn tcp_client_connect_timeout_or_unreachable() {
    let conn = Connection::TcpClient {
        host: "192.0.2.1".into(),
        port: 9,
        connect_timeout_ms: 300,
    };
    let started = std::time::Instant::now();
    let err = port::open(&conn).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(
        matches!(err, PortError::Timeout(_) | PortError::Io { .. }),
        "{err}"
    );
}

#[tokio::test]
async fn tcp_server_merges_clients_and_survives_disconnects() {
    let conn = Connection::TcpServer {
        bind: "127.0.0.1".into(),
        port: 0,
    };
    // Port 0 is rejected by validation in config, but the backend accepts it for tests.
    let mut port = port::open(&conn).await.unwrap();
    let local_port: u16 = port.description.rsplit(':').next().unwrap().parse().unwrap();

    let mut clients = Vec::new();
    for _ in 0..3 {
        clients.push(TcpStream::connect(("127.0.0.1", local_port)).await.unwrap());
        assert!(next_info(&mut port).await.contains("connected"));
    }
    for (i, c) in clients.iter_mut().enumerate() {
        c.write_all(format!("c{i};").as_bytes()).await.unwrap();
    }
    // Every client's data is tagged with its address.
    let mut from = std::collections::BTreeMap::new();
    while from.values().map(Vec::len).sum::<usize>() < 9 {
        match timeout(T, port.rx.recv()).await.expect("data") {
            Some(PortRx::DataFrom(b, peer)) => {
                from.entry(peer).or_insert_with(Vec::new).extend_from_slice(&b);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    for (i, c) in clients.iter().enumerate() {
        let me = c.local_addr().unwrap();
        assert_eq!(from[&me], format!("c{i};").as_bytes(), "data of {me}");
    }

    drop(clients.remove(1));
    assert!(next_info(&mut port).await.contains("disconnected"));

    port.writer.write_all(b"all").await.unwrap();
    for c in &mut clients {
        let mut buf = [0u8; 3];
        timeout(T, c.read_exact(&mut buf)).await.unwrap().unwrap();
        assert_eq!(&buf, b"all");
    }
}

/// A client that stops reading is dropped instead of blocking the writes to everyone else.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_server_drops_a_client_that_stops_reading() {
    const CHUNK: usize = 64 * 1024;
    const WRITES: usize = 400;
    let conn = Connection::TcpServer {
        bind: "127.0.0.1".into(),
        port: 0,
    };
    let mut port = port::open(&conn).await.unwrap();
    let local_port: u16 = port.description.rsplit(':').next().unwrap().parse().unwrap();
    let stalled = TcpStream::connect(("127.0.0.1", local_port)).await.unwrap();
    assert!(next_info(&mut port).await.contains("connected"));
    let mut readers = Vec::new();
    for _ in 0..2 {
        let mut c = TcpStream::connect(("127.0.0.1", local_port)).await.unwrap();
        assert!(next_info(&mut port).await.contains("connected"));
        readers.push(tokio::spawn(async move {
            let mut buf = vec![0u8; CHUNK];
            let mut total = 0;
            while total < CHUNK * WRITES {
                let n = timeout(T, c.read(&mut buf)).await.unwrap().unwrap();
                assert!(n > 0, "closed early");
                total += n;
            }
            total
        }));
    }
    let data = vec![0xA5u8; CHUNK];
    for i in 0..WRITES {
        timeout(Duration::from_secs(2), port.writer.write_all(&data))
            .await
            .unwrap_or_else(|_| panic!("write {i} blocked on the stalled client"))
            .unwrap();
        // Let the reading clients run (on a loaded machine yielding alone can starve them,
        // and a client whose queue fills up is dropped like the stalled one).
        if i % 16 == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
    for r in readers {
        assert_eq!(r.await.unwrap(), CHUNK * WRITES);
    }
    let mut dropped = false;
    while let Ok(Some(PortRx::Info(msg))) = timeout(Duration::from_millis(500), port.rx.recv()).await {
        dropped |= msg.contains("dropped: not reading");
    }
    assert!(dropped, "the stalled client was dropped");
    drop(stalled);
}

#[tokio::test]
async fn udp_learns_remote_and_resolves_hostnames() {
    // U1: hostnames must resolve (the C version passed NULL addresses on).
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer_port = peer.local_addr().unwrap().port();

    let fixed = Connection::Udp {
        bind: "127.0.0.1".into(),
        local_port: 0,
        remote_host: Some("localhost".into()),
        remote_port: peer_port,
        max_datagram: 0,
    };
    let mut port = port::open(&fixed).await.unwrap();
    port.writer.write_all(b"hi").await.unwrap();
    let mut buf = [0u8; 16];
    let (n, from) = timeout(T, peer.recv_from(&mut buf)).await.unwrap().unwrap();
    assert_eq!(&buf[..n], b"hi");
    peer.send_to(b"yo", from).await.unwrap();
    assert_eq!(recv_bytes(&mut port, 2).await, b"yo");

    let learn = Connection::Udp {
        bind: "127.0.0.1".into(),
        local_port: 0,
        remote_host: None,
        remote_port: 0,
        max_datagram: 0,
    };
    let mut port = port::open(&learn).await.unwrap();
    assert!(matches!(
        port.writer.write_all(b"x").await,
        Err(PortError::NoRemote)
    ));
    let addr: std::net::SocketAddr = port
        .description
        .trim_start_matches("UDP:")
        .split("->")
        .next()
        .unwrap()
        .parse()
        .unwrap();
    peer.send_to(b"first", addr).await.unwrap();
    assert!(next_info(&mut port).await.starts_with("remote set to"));
    assert_eq!(recv_bytes(&mut port, 5).await, b"first");
    port.writer.write_all(b"back").await.unwrap();
    let (n, _) = timeout(T, peer.recv_from(&mut buf)).await.unwrap().unwrap();
    assert_eq!(&buf[..n], b"back");
}

#[tokio::test]
async fn udp_ipv6_loopback() {
    let Ok(peer) = UdpSocket::bind("[::1]:0").await else {
        eprintln!("IPv6 unavailable, skipping");
        return;
    };
    let conn = Connection::Udp {
        bind: "::1".into(),
        local_port: 0,
        remote_host: Some("::1".into()),
        remote_port: peer.local_addr().unwrap().port(),
        max_datagram: 0,
    };
    let mut port = port::open(&conn).await.unwrap();
    port.writer.write_all(b"v6").await.unwrap();
    let mut buf = [0u8; 8];
    let (n, from) = timeout(T, peer.recv_from(&mut buf)).await.unwrap().unwrap();
    assert_eq!(&buf[..n], b"v6");
    peer.send_to(b"ok", from).await.unwrap();
    assert_eq!(recv_bytes(&mut port, 2).await, b"ok");
}

#[tokio::test]
async fn loopback_pair() {
    let (mut port, mut dev) = port::loopback::pair("loop");
    dev.send(&b"abc"[..]).await.unwrap();
    assert_eq!(recv_bytes(&mut port, 3).await, b"abc");
    port.writer.write_all(b"xyz").await.unwrap();
    assert_eq!(dev.recv().await.unwrap(), &b"xyz"[..]);
    dev.disconnect().await;
    assert!(matches!(port.rx.recv().await, Some(PortRx::Closed(None))));
}

/// Long UDP writes are split at the configured datagram size.
#[tokio::test]
async fn udp_splits_writes_at_max_datagram() {
    let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let conn = Connection::Udp {
        bind: "127.0.0.1".into(),
        local_port: 0,
        remote_host: Some("127.0.0.1".into()),
        remote_port: peer.local_addr().unwrap().port(),
        max_datagram: 100,
    };
    conn.validate().unwrap();
    let mut port = port::open(&conn).await.unwrap();
    port.writer.write_all(&pattern(250)).await.unwrap();
    let mut buf = [0u8; 1024];
    let mut sizes = Vec::new();
    for _ in 0..3 {
        let (n, _) = timeout(T, peer.recv_from(&mut buf)).await.unwrap().unwrap();
        sizes.push(n);
    }
    assert_eq!(sizes, [100, 100, 50]);
    let too_big = Connection::Udp {
        bind: "0.0.0.0".into(),
        local_port: 0,
        remote_host: None,
        remote_port: 0,
        max_datagram: 65_508,
    };
    assert!(too_big.validate().is_err());
}
