//! Session actor tests on loopback ports and PTYs.

use std::time::{Duration, Instant};

use gtkterm_core::chunks::Direction;
use gtkterm_core::config::{Connection, SerialConfig, TerminalConfig};
use gtkterm_core::port::loopback;
use gtkterm_core::session::{PortState, Session, SessionError, SessionEvent};
use gtkterm_core::testing::PtyPair;
use gtkterm_protocols::sunshine::ChartKind;
use regex::bytes::Regex;
use tokio::sync::broadcast;
use tokio::time::timeout;

const T: Duration = Duration::from_secs(5);

async fn wait_for(
    events: &mut broadcast::Receiver<SessionEvent>,
    pred: impl Fn(&SessionEvent) -> bool,
) -> SessionEvent {
    timeout(T, async {
        loop {
            match events.recv().await {
                Ok(ev) if pred(&ev) => return ev,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("event channel closed: {e}"),
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

fn session() -> Session {
    Session::spawn(1, "test", TerminalConfig::default())
}

#[tokio::test]
async fn rx_and_tx_are_recorded() {
    let s = session();
    let mut ev = s.subscribe();
    let (port, mut dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    assert_eq!(s.status().state, PortState::Open);

    dev.send(&b"hello"[..]).await.unwrap();
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Data(c) if c.dir == Direction::Rx),
    )
    .await;
    s.send(&b"AT\r"[..]).await.unwrap();
    assert_eq!(dev.recv().await.unwrap(), &b"AT\r"[..]);
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Data(c) if c.dir == Direction::Tx),
    )
    .await;

    let st = s.status();
    assert_eq!((st.rx_bytes, st.tx_bytes, st.tx_queue), (5, 3, 0));
    assert_eq!(s.read_rx(0, 100).data, b"hello");
    let dirs: Vec<_> = s.with_chunks(|c| c.iter().map(|c| c.dir).collect());
    assert_eq!(dirs, [Direction::Rx, Direction::Tx]);
}

#[tokio::test]
async fn sunshine_keeps_updating_after_twenty_thousand_chunks() {
    // C defect S1.
    let s = session();
    let (port, dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    for i in 0..20_000 {
        dev.send(format!("{{P0|v|{i}}}\n").into_bytes()).await.unwrap();
    }
    dev.send(&b"{B:P1|a|1}\n"[..]).await.unwrap();
    let mut ev = s.subscribe();
    // Wait until the last packet was processed.
    timeout(T, async {
        while s.with_sunshine(|sun| sun.chart("P1").is_none()) {
            let _ = ev.recv().await;
        }
    })
    .await
    .unwrap();
    s.with_sunshine(|sun| {
        let c = sun.chart("P0").unwrap();
        assert_eq!(c.packets, 20_000);
        assert_eq!(c.series[0].last().unwrap().values, [19_999.0]);
        assert_eq!(sun.chart("P1").unwrap().kind, ChartKind::Bar);
    });
}

#[tokio::test]
async fn expect_matches_and_times_out() {
    let s = session();
    let (port, mut dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let cursor = s.rx_cursor();
    let device = tokio::spawn(async move {
        let cmd = dev.recv().await.unwrap();
        assert_eq!(cmd, &b"ATI\r"[..]);
        dev.send(&b"Model: X"[..]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        dev.send(&b"42\r\nOK\r\n"[..]).await.unwrap();
        dev
    });
    s.send(&b"ATI\r"[..]).await.unwrap();
    let re = Regex::new(r"Model: (\w+)\r\nOK").unwrap();
    let m = s.expect(&re, cursor, T).await.unwrap();
    assert_eq!(m.captures, vec![Some("X42".to_owned())]);
    assert_eq!(m.before, "");
    assert_eq!(m.cursor, s.rx_cursor() - 2);
    let _dev = device.await.unwrap();

    let never = Regex::new("ERROR").unwrap();
    let err = s
        .expect(&never, m.cursor, Duration::from_millis(50))
        .await
        .unwrap_err();
    assert!(matches!(err, SessionError::Timeout(_)));
}

#[tokio::test]
async fn line_pacing_with_delay() {
    let s = Session::spawn(
        1,
        "t",
        TerminalConfig {
            wait_delay_ms: 60,
            ..TerminalConfig::default()
        },
    );
    let (port, mut dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let start = Instant::now();
    s.send(&b"one\ntwo\nthree"[..]).await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(115),
        "two line delays expected, got {:?}",
        start.elapsed()
    );
    let mut got = Vec::new();
    while let Ok(Some(b)) = timeout(Duration::from_millis(20), dev.recv()).await {
        got.push(b);
    }
    assert_eq!(got, [&b"one\n"[..], b"two\n", b"three"]);
}

#[tokio::test]
async fn line_pacing_waits_for_char() {
    let s = Session::spawn(
        1,
        "t",
        TerminalConfig {
            wait_char: Some(b'>'),
            ..TerminalConfig::default()
        },
    );
    let (port, mut dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let sender = {
        let s = s.clone();
        tokio::spawn(async move { s.send(&b"a\nb\n"[..]).await })
    };
    assert_eq!(dev.recv().await.unwrap(), &b"a\n"[..]);
    // The second line must not be sent before the prompt arrives.
    assert!(timeout(Duration::from_millis(100), dev.recv()).await.is_err());
    dev.send(&b"> "[..]).await.unwrap();
    assert_eq!(timeout(T, dev.recv()).await.unwrap().unwrap(), &b"b\n"[..]);
    dev.send(&b">"[..]).await.unwrap();
    timeout(T, sender).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn disconnect_and_send_while_closed() {
    let s = session();
    let mut ev = s.subscribe();
    let (port, dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    dev.disconnect().await;
    wait_for(&mut ev, |e| matches!(e, SessionEvent::State(PortState::Closed))).await;
    assert!(matches!(s.send(&b"x"[..]).await, Err(SessionError::NotOpen)));
    assert!(matches!(s.set_dtr(true).await, Err(SessionError::NotOpen)));
}

#[tokio::test]
async fn open_errors_set_error_state() {
    let s = session();
    let conn = Connection::Serial(SerialConfig {
        port: "/dev/nope".into(),
        ..SerialConfig::default()
    });
    assert!(s.open(conn).await.is_err());
    assert!(matches!(s.status().state, PortState::Error(m) if m.contains("/dev/nope")));
}

#[tokio::test]
async fn serial_session_on_pty_reopens() {
    let pty = PtyPair::new().unwrap();
    let s = session();
    let conn = Connection::Serial(SerialConfig {
        port: pty.path_str(),
        ..SerialConfig::default()
    });
    for round in 0..3 {
        s.open(conn.clone()).await.unwrap();
        let mut ev = s.subscribe();
        pty.write(format!("{{P0|r|{round}}}\n").as_bytes()).await.unwrap();
        wait_for(&mut ev, |e| matches!(e, SessionEvent::Sunshine(_))).await;
        s.send(&b"ping"[..]).await.unwrap();
        assert_eq!(pty.read_exact(4).await.unwrap(), b"ping");
        s.close().await.unwrap();
        assert_eq!(s.status().state, PortState::Closed);
    }
    assert_eq!(s.with_sunshine(|sun| sun.chart("P0").unwrap().packets), 3);
}

#[tokio::test]
async fn non_serial_ports_reject_control_lines() {
    let s = session();
    let (port, _dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let err = s.send_break(Duration::from_millis(10)).await.unwrap_err();
    assert!(err.to_string().contains("no control lines"), "{err}");
}

#[tokio::test]
async fn session_log_records_rx_and_tx() {
    use gtkterm_core::session::LogOp;
    use gtkterm_protocols::logfmt::LogFormat;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.csv");
    let s = session();
    let (port, mut dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let st = s
        .log(LogOp::Start {
            path: path.clone(),
            format: LogFormat::Csv,
            append: false,
            port_label: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(st.path, path);
    let mut ev = s.subscribe();
    dev.send(&b"temp=21\r\n"[..]).await.unwrap();
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Data(c) if c.dir == Direction::Rx),
    )
    .await;
    s.send(&b"read\r"[..]).await.unwrap();
    let _ = dev.recv().await;
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Data(c) if c.dir == Direction::Tx),
    )
    .await;
    let done = s.log(LogOp::Stop).await.unwrap().unwrap();
    assert_eq!(done.bytes, 9 + 5);
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines[0], "Timestamp,Direction,Length,Data_Hex,Data_ASCII");
    assert!(
        lines[1].contains(",RX,9,\"74 65 6D 70 3D 32 31 0D 0A\",\"temp=21..\""),
        "{}",
        lines[1]
    );
    assert!(lines[2].contains(",TX,5,"), "{}", lines[2]);
    assert!(matches!(s.log(LogOp::Stop).await, Err(SessionError::NotLogging)));
    assert!(s.status().log.is_none());
}

#[tokio::test]
async fn modbus_rtu_is_decoded_in_both_directions() {
    use gtkterm_core::decoding::{DecoderMode, FrameSource};
    use gtkterm_protocols::decode::{Check, Protocol, modbus};

    let s = session();
    let (port, mut dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let st = s.set_decoder(DecoderMode::Protocol(Protocol::ModbusRtu), false);
    assert_eq!(st.protocol, Some(Protocol::ModbusRtu));

    let since = s.decoder_status().next_seq;
    s.send(modbus::rtu_frame(1, &[0x03, 0x00, 0x10, 0x00, 0x02]))
        .await
        .unwrap();
    dev.recv().await.unwrap();
    // Response split over two reads; the decoder reassembles it.
    let resp = modbus::rtu_frame(1, &[0x03, 0x04, 0x00, 0x01, 0x00, 0x02]);
    dev.send(resp[..4].to_vec()).await.unwrap();
    dev.send(resp[4..].to_vec()).await.unwrap();
    let rx = s
        .expect_frame(since, T, |f| f.source == FrameSource::Rx)
        .await
        .unwrap();
    assert_eq!(rx.frame.check, Check::Ok);
    assert!(rx.frame.summary.contains("0x0001 0x0002"), "{}", rx.frame.summary);
    let all = s.frames_since(since, 10);
    assert_eq!(all.frames.len(), 2);
    assert_eq!(all.frames[0].source, FrameSource::Tx);

    // Garbage is flushed by the inter-frame gap through the actor's poll tick.
    dev.send(vec![0x01, 0x03]).await.unwrap();
    let bad = s
        .expect_frame(all.next_seq, T, |f| f.source == FrameSource::Rx)
        .await
        .unwrap();
    assert!(matches!(bad.frame.check, Check::Failed { .. }));
    assert_eq!(s.status().decoder.check_failures, 1);
}

#[tokio::test]
async fn auto_decoder_announces_detection() {
    use gtkterm_core::decoding::DecoderMode;
    use gtkterm_protocols::decode::Protocol;

    let s = session();
    let mut ev = s.subscribe();
    let (port, dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    s.set_decoder(DecoderMode::Auto, false);
    dev.send(&b"AT+CSQ\r\n+CSQ: 23,99\r\nOK\r\nAT+CREG?\r\n+CREG: 0,1\r\nOK\r\n"[..])
        .await
        .unwrap();
    let ev = wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Decoder(d) if d.protocol.is_some()),
    )
    .await;
    let SessionEvent::Decoder(d) = ev else {
        unreachable!()
    };
    assert_eq!(d.protocol, Some(Protocol::At));
    assert!(!d.detecting);
    assert!(s.last_frames(10).iter().any(|f| f.frame.summary == "[RSP] OK"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reopening_the_same_device_never_finds_it_locked() {
    // The kernel releases a flock when the open file is finally released, which can be a moment
    // after close(); the device unlocks explicitly so an immediate reopen always succeeds.
    let pty = PtyPair::new().unwrap();
    let s = session();
    let conn = Connection::Serial(SerialConfig {
        port: pty.path_str(),
        ..SerialConfig::default()
    });
    for i in 0..200 {
        s.open(conn.clone())
            .await
            .unwrap_or_else(|e| panic!("reopen {i}: {e}"));
    }
    s.close().await.unwrap();
}

#[tokio::test]
async fn signal_monitor_records_changes_and_short_pulses() {
    use gtkterm_core::port::loopback::InputLines;
    let s = session();
    let (port, dev) = loopback::pair_with_lines("loop");
    s.attach(port).await.unwrap();
    let mut ev = s.subscribe();
    s.set_dtr(true).await.unwrap();
    dev.set_inputs(InputLines {
        cts: true,
        ..InputLines::default()
    });
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Signals(v) if v.iter().any(|x| x.changed.contains(&"CTS"))),
    )
    .await;
    dev.pulse_ri();
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Signals(v) if v.iter().any(|x| x.pulse)),
    )
    .await;
    let h = s.signal_history(None);
    let changed: Vec<Vec<&str>> = h.iter().map(|x| x.changed.clone()).collect();
    assert_eq!(
        changed,
        [vec![], vec!["DTR"], vec!["CTS"], vec!["RI"], vec!["RI"]],
        "{h:?}"
    );
    assert!(h[3].lines.ri && !h[4].lines.ri);
    let csv = s.signals_csv();
    assert_eq!(csv.lines().count(), 6);
    assert!(csv.lines().nth(3).unwrap().ends_with(",1,0,1,0,0,0,CTS"), "{csv}");
    s.clear_signals();
    assert!(s.signal_history(None).len() <= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_baud_finds_the_rate_the_device_talks_at() {
    use gtkterm_core::autobaud::auto_baud;
    use gtkterm_core::port::serial::read_termios;
    let pty = std::sync::Arc::new(PtyPair::new().unwrap());
    let s = session();
    s.open(Connection::Serial(SerialConfig {
        port: pty.path_str(),
        baudrate: 9600,
        ..SerialConfig::default()
    }))
    .await
    .unwrap();
    // The fake device "is" at 57600: readable text there, line noise at every other rate.
    let p = std::sync::Arc::clone(&pty);
    let stop = tokio_util::sync::CancellationToken::new();
    let st = stop.clone();
    let device = tokio::spawn(async move {
        let mut n = 0u8;
        while !st.is_cancelled() {
            let speed = read_termios(p.slave_fd().unwrap()).unwrap().ospeed;
            let data: Vec<u8> = if speed == 57_600 {
                b"temp=21.5 hum=40\r\n".to_vec()
            } else {
                (0..18).map(|i: u8| 0x80 | n.wrapping_add(i * 7)).collect()
            };
            n = n.wrapping_add(1);
            let _ = p.write(&data).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let r = auto_baud(
        &s,
        &[9600, 19_200, 57_600, 115_200],
        Duration::from_millis(250),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .unwrap();
    stop.cancel();
    device.await.unwrap();
    assert_eq!(r.best, Some(57_600), "{:?}", r.scores);
    assert_eq!(r.scores.len(), 3, "stops at a clear winner");
    assert!(
        s.status().description.contains("57600"),
        "reopened at the detected rate"
    );
}

/// Data the device already received is in the log even when the log stops (or the port
/// closes) right after the send: write reports and commands travel on different channels.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_the_log_right_after_a_send_keeps_the_tx_record() {
    use gtkterm_core::session::LogOp;
    use gtkterm_protocols::logfmt::LogFormat;
    let dir = tempfile::tempdir().unwrap();
    let s = session();
    for round in 0..100 {
        let (port, mut dev) = loopback::pair("loop");
        s.attach(port).await.unwrap();
        let path = dir.path().join(format!("{round}.txt"));
        s.log(LogOp::Start {
            path: path.clone(),
            format: LogFormat::Text,
            append: false,
            port_label: None,
        })
        .await
        .unwrap();
        s.send(&b"AT\r"[..]).await.unwrap();
        assert_eq!(&dev.recv().await.unwrap()[..], b"AT\r");
        let done = if round % 2 == 0 {
            s.log(LogOp::Stop).await.unwrap().unwrap()
        } else {
            s.close().await.unwrap();
            s.log(LogOp::Stop).await.unwrap().unwrap()
        };
        assert_eq!(done.bytes, 3, "round {round}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("T: AT<0D>"));
    }
}

/// A UDP send before any peer is known, or a datagram the network refuses, fails that one
/// send but keeps the port open; large sends are split into datagrams that fit IPv4.
#[tokio::test]
async fn undeliverable_udp_writes_keep_the_port_open() {
    use tokio::net::UdpSocket;
    let s = session();
    s.open(Connection::Udp {
        bind: "127.0.0.1".into(),
        local_port: 0,
        remote_host: None,
        remote_port: 0,
    })
    .await
    .unwrap();
    let mut ev = s.subscribe();
    assert!(s.send(&b"early"[..]).await.is_err(), "no remote yet");
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Info(m) if m.starts_with("not sent:")),
    )
    .await;
    assert!(
        matches!(s.status().state, PortState::Open),
        "{:?}",
        s.status().state
    );

    let addr: std::net::SocketAddr = s
        .status()
        .description
        .trim_start_matches("UDP:")
        .split("->")
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    peer.send_to(b"hello", addr).await.unwrap();
    wait_for(
        &mut ev,
        |e| matches!(e, SessionEvent::Data(c) if c.dir == Direction::Rx),
    )
    .await;
    s.send(vec![0x55; 100_000]).await.unwrap();
    let mut buf = vec![0u8; 70_000];
    let mut sizes = Vec::new();
    while sizes.iter().sum::<usize>() < 100_000 {
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();
        sizes.push(n);
    }
    assert_eq!(sizes, [65_507, 34_493]);
    assert!(matches!(s.status().state, PortState::Open));
}

/// `expect` on a fast stream does work proportional to the data, and finds a match that is
/// split across reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_stays_fast_on_a_busy_stream() {
    let s = session();
    let (port, dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let cursor = s.status().rx_bytes;
    let feeder = tokio::spawn(async move {
        let noise = vec![b'.'; 256];
        for _ in 0..8192 {
            dev.send(noise.clone()).await.unwrap();
        }
        dev.send(&b"RE"[..]).await.unwrap();
        dev.send(&b"ADY 42\n"[..]).await.unwrap();
        dev
    });
    let started = Instant::now();
    let m = s
        .expect(
            &Regex::new(r"READY (\d+)").unwrap(),
            cursor,
            Duration::from_secs(20),
        )
        .await
        .unwrap();
    let _dev = feeder.await.unwrap();
    assert_eq!(m.captures, [Some("42".to_owned())]);
    assert_eq!(m.before.len(), 8192 * 256);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
}

/// Close overtakes a connect that is still waiting, and a long break does not freeze the
/// session (data, Close and status keep working meanwhile).
#[tokio::test]
async fn close_overtakes_a_slow_connect_and_breaks_run_aside() {
    let s = session();
    let opener = {
        let s = s.clone();
        tokio::spawn(async move {
            s.open(Connection::TcpClient {
                host: "192.0.2.1".into(),
                port: 9,
                connect_timeout_ms: 10_000,
            })
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    s.close().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "close waited {:?}",
        started.elapsed()
    );
    let r = opener.await.unwrap();
    assert!(matches!(r, Err(SessionError::Cancelled)), "{r:?}");
    assert!(
        matches!(s.status().state, PortState::Closed),
        "{:?}",
        s.status().state
    );

    let (port, mut dev) = loopback::pair_with_lines("loop");
    s.attach(port).await.unwrap();
    let breaker = {
        let s = s.clone();
        tokio::spawn(async move { s.send_break(Duration::from_secs(60)).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = Instant::now();
    s.send(&b"during break"[..]).await.unwrap();
    assert_eq!(&dev.recv().await.unwrap()[..], b"during break");
    assert!(started.elapsed() < Duration::from_secs(1));
    let started = Instant::now();
    breaker.await.unwrap().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "a 60 s break is capped at 5 s: {:?}",
        started.elapsed()
    );
}
