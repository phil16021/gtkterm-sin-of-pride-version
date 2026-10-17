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
