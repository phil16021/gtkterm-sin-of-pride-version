//! File transfer interoperability with lrzsz (`sx/rx/sb/rb/sz/rz`) over a socat PTY.
//! Skipped when socat or lrzsz is not installed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gtkterm_core::config::{Connection, SerialConfig, TerminalConfig};
use gtkterm_core::session::{Session, SessionError, SessionEvent};
use gtkterm_core::transfer::{TransferProtocol, TransferRequest, TransferState, TransferStatus};
use tokio::time::timeout;

fn have(bin: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {bin}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn tools() -> bool {
    let ok = ["socat", "sx", "rx", "sb", "rb", "sz", "rz"]
        .iter()
        .all(|b| have(b));
    if !ok {
        eprintln!("skipping: socat or lrzsz missing");
    }
    ok
}

/// lrzsz running on the far end of a PTY whose near end is `tty`.
struct Peer {
    /// socat; killed on drop.
    _child: tokio::process::Child,
    tty: PathBuf,
}

async fn peer(cmd: &str, cwd: &Path, tty_dir: &Path) -> Peer {
    let tty = tty_dir.join("tty");
    let child = tokio::process::Command::new("socat")
        .arg("-d0")
        .arg(format!("pty,raw,echo=0,link={}", tty.display()))
        // Start after the session opened the port (opening flushes input, which would eat the
        // first start request), and keep the PTY up after lrzsz exits (a hangup discards its
        // last ACK before we read it).
        .arg(format!(
            "SYSTEM:sleep 0.3; {cmd} 2>>{}/peer.log; sleep 3,pty,raw,echo=0",
            tty_dir.display()
        ))
        .current_dir(cwd)
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("socat");
    for _ in 0..100 {
        if tty.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Peer { _child: child, tty }
}

fn trace() {
    if std::env::var_os("TRANSFER_TRACE").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("gtkterm_core=trace")
            .with_test_writer()
            .try_init();
    }
}

async fn session(tty: &Path) -> Session {
    trace();
    let s = Session::spawn(1, "test", TerminalConfig::default());
    s.open(Connection::Serial(SerialConfig {
        port: tty.to_string_lossy().into_owned(),
        disable_port_lock: true,
        ..SerialConfig::default()
    }))
    .await
    .expect("open pty");
    s
}

async fn run(s: &Session, req: TransferRequest) -> TransferStatus {
    let t0 = std::time::Instant::now();
    timeout(
        Duration::from_secs(60),
        s.transfer(req, |st| {
            if std::env::var_os("TRANSFER_TRACE").is_some() {
                eprintln!(
                    "  {:>6} ms {:?} {:?} {}/{:?} retries {}",
                    t0.elapsed().as_millis(),
                    st.state,
                    st.file,
                    st.bytes,
                    st.size,
                    st.retries
                );
            }
        }),
    )
    .await
    .expect("transfer timed out")
    .expect("transfer started")
}

/// Wait until lrzsz has written `files` (name, minimum size) into `dir`, then stop socat.
async fn finish(p: Peer, dir: &Path, files: &[(&str, usize)]) {
    for _ in 0..200 {
        let done = files
            .iter()
            .all(|(name, len)| std::fs::metadata(dir.join(name)).is_ok_and(|m| m.len() as usize >= *len));
        if done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    drop(p);
}

fn files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("one.bin", vec![0x42]),
        ("block.bin", (0..129u32).map(|i| (i * 7) as u8).collect()),
        // More than 255 blocks: the block number wraps (X4).
        (
            "big.bin",
            (0..33 * 1024 + 5).map(|i: u32| (i ^ (i >> 8)) as u8).collect(),
        ),
        // Control bytes as payload (X2) and SUB bytes that must survive (X3).
        ("eot.bin", vec![0x04; 1000]),
        ("can.bin", vec![0x18; 1000]),
        ("sub.bin", vec![0x1A; 1000]),
    ]
}

/// XMODEM cannot transmit the length: the receiver may keep the SUB padding of the last block.
fn assert_xmodem_eq(got: &[u8], want: &[u8], what: &str) {
    assert!(
        got.len() >= want.len() && got.len() - want.len() < 1024,
        "{what}: {} vs {} bytes",
        got.len(),
        want.len()
    );
    assert_eq!(&got[..want.len()], want, "{what}");
    assert!(got[want.len()..].iter().all(|&b| b == 0x1A), "{what}: padding");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_to_lrzsz() {
    if !tools() {
        return;
    }
    for (protocol, cmd) in [
        (TransferProtocol::Xmodem, "rx -c out.bin"),
        (TransferProtocol::Xmodem1k, "rx -c out.bin"),
        (TransferProtocol::Ymodem, "rb"),
        (TransferProtocol::YmodemG, "rb"),
        (TransferProtocol::Zmodem, "rz -b"),
    ] {
        let src = tempfile::tempdir().unwrap();
        let mut all = Vec::new();
        for (name, data) in files() {
            let path = src.path().join(name);
            std::fs::write(&path, &data).unwrap();
            all.push((path, data));
        }
        let batches: Vec<Vec<(PathBuf, Vec<u8>)>> = if protocol.batch() {
            vec![all]
        } else {
            all.into_iter().map(|f| vec![f]).collect()
        };
        for batch in batches {
            eprintln!(
                "send {protocol:?} {:?}",
                batch
                    .iter()
                    .map(|(p, _)| p.file_name().unwrap().to_owned())
                    .collect::<Vec<_>>()
            );
            let out = tempfile::tempdir().unwrap();
            let p = peer(cmd, out.path(), out.path()).await;
            let s = session(&p.tty).await;
            let st = run(
                &s,
                TransferRequest::Send {
                    protocol,
                    files: batch.iter().map(|(p, _)| p.clone()).collect(),
                },
            )
            .await;
            let log = std::fs::read_to_string(out.path().join("peer.log")).unwrap_or_default();
            assert_eq!(
                st.state,
                TransferState::Finished,
                "{protocol:?}: {:?}\npeer: {log}",
                st.message
            );
            assert_eq!(st.files.len(), batch.len(), "{protocol:?}");
            s.close().await.unwrap();
            let expect: Vec<(String, usize)> = batch
                .iter()
                .map(|(path, data)| {
                    let name = if protocol.batch() {
                        path.file_name().unwrap().to_string_lossy().into_owned()
                    } else {
                        "out.bin".to_owned()
                    };
                    (name, data.len())
                })
                .collect();
            let expect: Vec<(&str, usize)> = expect.iter().map(|(n, l)| (n.as_str(), *l)).collect();
            finish(p, out.path(), &expect).await;
            for (path, want) in &batch {
                let name = path.file_name().unwrap();
                let what = format!("{protocol:?} {}", name.to_string_lossy());
                if protocol.batch() {
                    let got = std::fs::read(out.path().join(name)).unwrap_or_else(|e| panic!("{what}: {e}"));
                    assert_eq!(&got, want, "{what}");
                } else {
                    let got = std::fs::read(out.path().join("out.bin")).unwrap();
                    assert_xmodem_eq(&got, want, &what);
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn receive_from_lrzsz() {
    if !tools() {
        return;
    }
    let src = tempfile::tempdir().unwrap();
    let mut names = Vec::new();
    for (name, data) in files() {
        std::fs::write(src.path().join(name), data).unwrap();
        names.push(name);
    }
    let list = names.join(" ");
    for (protocol, cmd) in [
        (TransferProtocol::Xmodem, "sx big.bin".to_owned()),
        (TransferProtocol::Xmodem1k, "sx -k big.bin".to_owned()),
        (TransferProtocol::Ymodem, format!("sb {list}")),
        (TransferProtocol::YmodemG, format!("sb {list}")),
        (TransferProtocol::Zmodem, format!("sz -b {list}")),
    ] {
        let out = tempfile::tempdir().unwrap();
        let tty_dir = tempfile::tempdir().unwrap();
        let p = peer(&cmd, src.path(), tty_dir.path()).await;
        let s = session(&p.tty).await;
        let st = run(
            &s,
            TransferRequest::Receive {
                protocol,
                dir: out.path().to_owned(),
                name: Some("download.bin".into()),
                overwrite: false,
            },
        )
        .await;
        let log = std::fs::read_to_string(tty_dir.path().join("peer.log")).unwrap_or_default();
        assert_eq!(
            st.state,
            TransferState::Finished,
            "{protocol:?}: {:?}\npeer: {log}",
            st.message
        );
        s.close().await.unwrap();
        drop(p);
        if protocol.batch() {
            assert_eq!(st.files.len(), names.len(), "{protocol:?}: {:?}", st.files);
            for name in &names {
                let got = std::fs::read(out.path().join(name))
                    .unwrap_or_else(|e| panic!("{protocol:?} {name}: {e}"));
                assert_eq!(
                    got,
                    std::fs::read(src.path().join(name)).unwrap(),
                    "{protocol:?} {name}"
                );
            }
        } else {
            let got = std::fs::read(out.path().join("download.bin")).unwrap();
            let want = std::fs::read(src.path().join("big.bin")).unwrap();
            assert_eq!(got, want, "{protocol:?}: the final block's padding is stripped");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_owns_the_port_and_can_be_cancelled() {
    use gtkterm_core::port::loopback;
    let s = Session::spawn(1, "test", TerminalConfig::default());
    let (port, mut dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut events = s.subscribe();
    s.start_transfer(TransferRequest::Receive {
        protocol: TransferProtocol::Xmodem,
        dir: dir.path().to_owned(),
        name: None,
        overwrite: false,
    })
    .await
    .unwrap();
    assert_eq!(
        dev.recv().await.unwrap(),
        &b"C"[..],
        "the receiver asks for CRC blocks"
    );
    assert!(matches!(s.send(&b"typed"[..]).await, Err(SessionError::Busy)));
    let second = s
        .start_transfer(TransferRequest::Send {
            protocol: TransferProtocol::Xmodem,
            files: vec![dir.path().join("x")],
        })
        .await;
    assert!(matches!(second, Err(SessionError::Busy)));
    assert!(s.cancel_transfer().await);
    let st = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(SessionEvent::Transfer(st)) = events.recv().await
                && st.state != TransferState::Running
            {
                return st;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(st.state, TransferState::Cancelled);
    let cancel = dev.recv().await.unwrap();
    assert!(cancel.starts_with(&[0x18, 0x18]), "{cancel:?}");
    // The session is usable again, and protocol bytes did not end up in the history.
    s.send(&b"hi"[..]).await.unwrap();
    for _ in 0..100 {
        if s.with_chunks(|c| c.iter().count()) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(s.with_chunks(|c| c.iter().count()), 1);
    assert_eq!(s.transfer_status().unwrap().state, TransferState::Cancelled);
    assert!(!s.cancel_transfer().await);

    let missing = s
        .start_transfer(TransferRequest::Send {
            protocol: TransferProtocol::Ymodem,
            files: vec![dir.path().join("missing.bin")],
        })
        .await;
    assert!(matches!(missing, Err(SessionError::Transfer(m)) if m.contains("missing.bin")));
}
