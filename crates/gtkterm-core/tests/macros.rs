//! Macro playback against a simulated device.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use gtkterm_core::config::Config;
use gtkterm_core::macros::{Macro, MacroCommand, MacroError, MacroStore, parse_text};
use gtkterm_core::manager::{ManagerEvent, SessionManager};
use gtkterm_core::port::loopback;

fn manager_with(text: &str) -> Arc<SessionManager> {
    let m = SessionManager::new(Config::default(), None);
    let mut store = MacroStore::default();
    store.insert_unique(Macro {
        name: "m".into(),
        shortcut: None,
        commands: parse_text(text).unwrap(),
    });
    m.set_macros(store, None);
    m
}

#[tokio::test]
async fn send_expect_wait_sequence() {
    let mgr = manager_with(
        "send: ATI\\r\nexpect[2000]: OK\\r\\n\nwait: 50 ms\nsend[30]: AT+CSQ\\r\nexpect: \\+CSQ: (\\d+)\n",
    );
    let session = mgr.first_or_create();
    let (port, mut dev) = loopback::pair("dev");
    session.attach(port).await.unwrap();
    let device = tokio::spawn(async move {
        assert_eq!(dev.recv().await.unwrap(), &b"ATI\r"[..]);
        dev.send(&b"Model X\r\nOK\r\n"[..]).await.unwrap();
        let t = Instant::now();
        assert_eq!(dev.recv().await.unwrap(), &b"AT+CSQ\r"[..]);
        assert!(
            t.elapsed() >= Duration::from_millis(75),
            "wait + delay before the second send"
        );
        // Reply arrives immediately: expect must still see it (cursor taken before the send).
        dev.send(&b"+CSQ: 23,99\r\nOK\r\n"[..]).await.unwrap();
        dev
    });
    let report = mgr.run_macro(&session, "m").await.unwrap();
    let _dev = device.await.unwrap();
    assert_eq!(report.steps.len(), 5);
    assert!(
        report.steps[4].detail.contains("+CSQ: 23"),
        "{:?}",
        report.steps[4]
    );
    assert!(report.received.contains("Model X"));
}

#[tokio::test]
async fn expect_timeout_reports_step() {
    let mgr = manager_with("send: x\nexpect[80]: NEVER\nsend: not reached\n");
    let session = mgr.first_or_create();
    let (port, mut dev) = loopback::pair("dev");
    session.attach(port).await.unwrap();
    let err = mgr.run_macro(&session, "m").await.unwrap_err();
    match err {
        MacroError::Step {
            step,
            message,
            report,
        } => {
            assert_eq!(step, 2);
            assert!(message.contains("NEVER"));
            assert_eq!(report.steps.len(), 1);
        }
        other => panic!("{other}"),
    }
    assert_eq!(dev.drain(), b"x");
}

#[tokio::test]
async fn cancel_and_one_at_a_time() {
    let mgr = manager_with("wait: 5 s\nsend: late\n");
    let session = mgr.first_or_create();
    let (port, mut dev) = loopback::pair("dev");
    session.attach(port).await.unwrap();
    let mut events = mgr.subscribe();
    let runner = {
        let (mgr, session) = (Arc::clone(&mgr), session.clone());
        tokio::spawn(async move { mgr.run_macro(&session, "m").await })
    };
    loop {
        if let ManagerEvent::MacroStarted { name, .. } = events.recv().await.unwrap() {
            assert_eq!(name, "m");
            break;
        }
    }
    assert_eq!(mgr.running_macro(session.id()).as_deref(), Some("m"));
    assert!(matches!(
        mgr.run_macro(&session, "m").await,
        Err(MacroError::Busy(_))
    ));
    assert!(mgr.stop_macro(session.id()));
    assert!(matches!(runner.await.unwrap(), Err(MacroError::Cancelled)));
    loop {
        if let ManagerEvent::MacroFinished { error, .. } = events.recv().await.unwrap() {
            assert_eq!(error.as_deref(), Some("macro cancelled"));
            break;
        }
    }
    assert!(mgr.running_macro(session.id()).is_none());
    assert!(dev.drain().is_empty());
    assert!(matches!(
        mgr.run_macro(&session, "nope").await,
        Err(MacroError::NotFound(_))
    ));
}

#[tokio::test]
async fn update_macros_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("macros.json");
    let mgr = SessionManager::new(Config::default(), None);
    mgr.set_macros(MacroStore::default(), Some(path.clone()));
    mgr.update_macros(|s| {
        s.upsert(
            None,
            Macro {
                name: "hello".into(),
                shortcut: Some("F9".into()),
                commands: vec![MacroCommand::Send {
                    data: "hi\\n".into(),
                    delay_ms: 0,
                }],
            },
        )
    })
    .unwrap();
    assert_eq!(MacroStore::load(&path).unwrap().list()[0].name, "hello");
}
