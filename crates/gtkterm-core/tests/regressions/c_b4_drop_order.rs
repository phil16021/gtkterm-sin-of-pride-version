//! B4: the C buffer's dispose freed GObjects with `g_free`. Ownership makes that impossible in
//! Rust; what is left to guard is that dropping the handles in any order shuts the session down,
//! closes the port and releases the device lock.

use std::time::Duration;

use gtkterm_core::config::{Config, Connection, SerialConfig, TerminalConfig};
use gtkterm_core::manager::SessionManager;
use gtkterm_core::port::loopback;
use gtkterm_core::testing::PtyPair;
use tokio::sync::broadcast::error::RecvError;

const LIMIT: Duration = Duration::from_secs(3);

#[tokio::test]
async fn c_b4_dropping_the_manager_first_closes_the_port() {
    let manager = SessionManager::new(Config::default(), None);
    let session = manager.create("b4", TerminalConfig::default());
    let (port, mut dev) = loopback::pair("loop");
    session.attach(port).await.unwrap();
    let mut events = session.subscribe();
    dev.send(&b"data\n"[..]).await.unwrap();
    tokio::time::timeout(LIMIT, async {
        while session.rx_cursor() < 5 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(manager);
    drop(session);
    // The actor stops: the device sees the host go away and the events end.
    assert!(tokio::time::timeout(LIMIT, dev.recv()).await.unwrap().is_none());
    let end = tokio::time::timeout(LIMIT, async {
        loop {
            match events.recv().await {
                Err(RecvError::Closed) => break,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "event stream did not end");
}

#[tokio::test]
async fn c_b4_dropping_the_subscriber_first_keeps_the_session_working() {
    let manager = SessionManager::new(Config::default(), None);
    let session = manager.create("b4", TerminalConfig::default());
    let (port, mut dev) = loopback::pair("loop");
    session.attach(port).await.unwrap();
    drop(session.subscribe());
    let clone = session.clone();
    drop(session);
    clone.send(&b"still here"[..]).await.unwrap();
    assert_eq!(&dev.recv().await.unwrap()[..], b"still here");
}

#[tokio::test]
async fn c_b4_dropping_everything_releases_the_device_lock() {
    let pty = PtyPair::new().unwrap();
    let conn = Connection::Serial(SerialConfig {
        port: pty.path_str(),
        ..SerialConfig::default()
    });
    {
        let manager = SessionManager::new(Config::default(), None);
        let session = manager.create("first", TerminalConfig::default());
        session.open(conn.clone()).await.unwrap();
        let other = manager.create("other", TerminalConfig::default());
        assert!(
            other.open(conn.clone()).await.is_err(),
            "the device is locked while open"
        );
        drop(other);
        drop(manager);
    }
    // Reopening needs the exclusive lock of the first session to be gone; it is released when
    // its actor finishes, shortly after the last handle is dropped.
    let manager = SessionManager::new(Config::default(), None);
    let session = manager.create("second", TerminalConfig::default());
    let deadline = tokio::time::Instant::now() + LIMIT;
    loop {
        match session.open(conn.clone()).await {
            Ok(()) => break,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => panic!("the device stayed locked: {e}"),
        }
    }
}
