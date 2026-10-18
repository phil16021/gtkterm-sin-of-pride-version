//! Script runs against fake devices on loopback ports, including the shipped examples.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::time::Duration;

use gtkterm_core::config::{Config, TerminalConfig};
use gtkterm_core::manager::SessionManager;
use gtkterm_core::port::loopback::{self, DeviceEnd};
use gtkterm_core::script::{Script, ScriptError, ScriptOptions, run_script};
use gtkterm_core::session::Session;
use tokio_util::sync::CancellationToken;

fn example(name: &str) -> Script {
    Script::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/examples")
            .join(name),
        None,
    )
    .unwrap()
}

/// Answer each received line with `reply(line)`.
fn serve(
    mut dev: DeviceEnd,
    reply: impl Fn(&str) -> Option<String> + Send + 'static,
) -> tokio::task::JoinHandle<DeviceEnd> {
    tokio::spawn(async move {
        let mut buf = String::new();
        while let Ok(Some(b)) = tokio::time::timeout(Duration::from_secs(3), dev.recv()).await {
            buf.push_str(&String::from_utf8_lossy(&b));
            while let Some(i) = buf.find(['\r', '\n']) {
                let line: String = buf.drain(..=i).collect();
                let line = line.trim();
                if !line.is_empty()
                    && let Some(r) = reply(line)
                {
                    dev.send(r).await.unwrap();
                }
            }
        }
        dev
    })
}

async fn session_on(port: gtkterm_core::port::OpenPort) -> Session {
    let s = Session::spawn(1, "test", TerminalConfig::default());
    s.attach(port).await.unwrap();
    s
}

async fn run(
    s: &Session,
    script: &Script,
    opts: &ScriptOptions,
) -> Result<gtkterm_core::script::ScriptReport, ScriptError> {
    run_script(s, script, opts, &CancellationToken::new(), |_| {}).await
}

#[tokio::test]
async fn example_modem_script_runs_unchanged() {
    let (port, dev) = loopback::pair("loop");
    let s = session_on(port).await;
    let _modem = serve(dev, |cmd| {
        Some(match cmd {
            "AT" => "\r\nOK\r\n".into(),
            "AT+CGMI" => "\r\nGTKTerm Labs\r\nOK\r\n".into(),
            "AT+CGMM" => "\r\nFake-4G\r\nOK\r\n".into(),
            "AT+CSQ" => "\r\n+CSQ: 23,99\r\n\r\nOK\r\n".into(),
            "AT+CGSN" => "\r\n490154203237518\r\n\r\nOK\r\n".into(),
            _ => "\r\nERROR\r\n".into(),
        })
    });
    let report = run(&s, &example("example_modem.script"), &ScriptOptions::default())
        .await
        .unwrap();
    assert_eq!(report.log.last().unwrap(), "Modem test completed successfully!");
    assert_eq!(report.vars["MATCH_0"], "490154203237518");
    assert!(report.received.contains("+CSQ: 23,99"));
}

#[tokio::test]
async fn example_embedded_script_runs_against_a_bootloader() {
    let (port, dev) = loopback::pair_with_lines("loop");
    let s = session_on(port).await;
    // The prompt appears only after a DTR reset pulse (low, then high).
    let mut lines = dev.watch_lines();
    let dev = std::sync::Arc::new(tokio::sync::Mutex::new(dev));
    let d = std::sync::Arc::clone(&dev);
    let reset = tokio::spawn(async move {
        let mut was_low = false;
        while lines.changed().await.is_ok() {
            let dtr = lines.borrow().dtr;
            if !dtr {
                was_low = true;
            } else if was_low {
                d.lock()
                    .await
                    .send(&b"\r\nBootloader v2.1\r\n> "[..])
                    .await
                    .unwrap();
                return;
            }
        }
    });
    let script = example("example_embedded.script");
    let runner = tokio::spawn(async move { run(&s, &script, &ScriptOptions::default()).await });
    reset.await.unwrap();
    let mut buf = String::new();
    loop {
        let b = tokio::time::timeout(Duration::from_secs(5), async { dev.lock().await.recv().await })
            .await
            .expect("script stalled");
        // The runner owns the session: when it is done, the port closes.
        let Some(b) = b else { break };
        buf.push_str(&String::from_utf8_lossy(&b));
        while let Some(i) = buf.find('\n') {
            let line: String = buf.drain(..=i).collect();
            let reply = match line.trim().trim_start_matches('\u{3}') {
                "erase" => "Erase complete\r\n",
                "load" => "Ready for data\r\n",
                "verify" => "CRC: OK\r\n",
                "boot" => "Application started\r\n",
                _ => continue,
            };
            dev.lock().await.send(reply.as_bytes().to_vec()).await.unwrap();
        }
        if runner.is_finished() {
            break;
        }
    }
    let report = runner.await.unwrap().unwrap();
    assert_eq!(report.log.last().unwrap(), "Firmware update completed!");
    assert!(dev.lock().await.lines().dtr, "DTR left high");
}

#[tokio::test]
async fn control_flow_variables_and_failures() {
    let (port, dev) = loopback::pair("loop");
    let s = session_on(port).await;
    let _dev = serve(dev, |cmd| match cmd {
        "PING" => Some("PONG 7\r\n".into()),
        "BAD" => Some("ERROR 42\r\n".into()),
        _ => None,
    });
    let text = "\
set tries 0
again:
send PING\\r
expect PONG (\\d+)
if ${MATCH_1} != 7 goto wrong
set tries ${tries}x
if ${tries} != 0xxx goto again
send BAD\\r
if_match[1000] ERROR (\\d+) goto failed
fail no error reply
failed:
log error ${MATCH_1} after ${tries}
timeout 200
expect never
wrong:
fail unreachable
";
    let script = Script::parse("flow", text, None).unwrap();
    let err = run(&s, &script, &ScriptOptions::default()).await.unwrap_err();
    let ScriptError::Failed { at, message, report } = err else {
        panic!("expected a failure")
    };
    assert_eq!(at.line, 14, "{message}");
    assert!(message.contains("timed out after 200 ms"), "{message}");
    assert_eq!(report.log, ["error 42 after 0xxx"]);
    assert_eq!(report.vars["tries"], "0xxx");

    let script = Script::parse("exit", "log a\nexit\nlog b\n", None).unwrap();
    assert_eq!(
        run(&s, &script, &ScriptOptions::default()).await.unwrap().log,
        ["a"]
    );

    // A loop without waits stays cancellable.
    let script = Script::parse("spin", "top:\ngoto top\n", None).unwrap();
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.cancel();
    });
    let r = run_script(&s, &script, &ScriptOptions::default(), &cancel, |_| {}).await;
    assert!(matches!(r, Err(ScriptError::Cancelled)));
}

#[tokio::test]
async fn exec_is_opt_in_and_never_uses_a_shell() {
    let (port, dev) = loopback::pair("loop");
    let s = session_on(port).await;
    // The device answers with shell metacharacters (ROADMAP A1).
    let _dev = serve(dev, |_| {
        Some("NAME: x; touch /tmp/pwned-by-device $(id)\r\n".into())
    });
    let text =
        "send who\\r\nexpect NAME: (.*)\\r\nexec printf %s ${MATCH_1}\nlog [${EXEC_OUTPUT}] ${EXEC_STATUS}\n";
    let script = Script::parse("exec", text, None).unwrap();
    let err = run(&s, &script, &ScriptOptions::default()).await.unwrap_err();
    assert!(err.to_string().contains("exec is disabled"), "{err}");
    let opts = ScriptOptions {
        allow_exec: true,
        ..ScriptOptions::default()
    };
    let report = run(&s, &script, &opts).await.unwrap();
    assert_eq!(
        report.log,
        ["[x; touch /tmp/pwned-by-device $(id)] 0"],
        "passed as one argument"
    );
    assert!(!Path::new("/tmp/pwned-by-device").exists());
}

#[tokio::test]
async fn one_automation_per_session() {
    let m = SessionManager::new(Config::default(), None);
    let s = m.create("t", TerminalConfig::default());
    let (port, _dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let slow = Script::parse("slow", "delay 500\n", None).unwrap();
    let m2 = std::sync::Arc::clone(&m);
    let s2 = s.clone();
    let first = tokio::spawn(async move { m2.run_script(&s2, &slow, &ScriptOptions::default()).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let quick = Script::parse("quick", "log hi\n", None).unwrap();
    let r = m.run_script(&s, &quick, &ScriptOptions::default()).await;
    assert!(matches!(r, Err(ScriptError::Busy(ref n)) if n == "slow"), "{r:?}");
    assert!(m.stop_macro(s.id()), "stop works for scripts too");
    assert!(matches!(first.await.unwrap(), Err(ScriptError::Cancelled)));
}

#[tokio::test]
async fn file_paths_respect_the_sandbox() {
    let (port, _dev) = loopback::pair("loop");
    let s = session_on(port).await;
    let inside = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"x").unwrap();
    let text = format!("send_file xmodem {}\n", outside.path().join("secret").display());
    let script = Script::parse("sb", &text, Some(inside.path())).unwrap();
    let opts = ScriptOptions {
        allowed_dirs: Some(vec![inside.path().to_owned()]),
        ..ScriptOptions::default()
    };
    let err = run(&s, &script, &opts).await.unwrap_err();
    assert!(err.to_string().contains("outside the allowed"), "{err}");
}

/// A run whose future is dropped (an MCP call that timed out) frees the session's slot and
/// reports the end, so the next macro or script can start.
#[tokio::test]
async fn dropping_a_running_script_frees_the_slot() {
    let m = SessionManager::new(Config::default(), None);
    let s = m.create("t", TerminalConfig::default());
    let (port, _dev) = loopback::pair("loop");
    s.attach(port).await.unwrap();
    let mut events = m.subscribe();
    let slow = Script::parse("slow", "delay 5000\n", None).unwrap();
    let r = tokio::time::timeout(
        Duration::from_millis(100),
        m.run_script(&s, &slow, &ScriptOptions::default()),
    )
    .await;
    assert!(r.is_err(), "timed out");
    assert_eq!(m.running_macro(s.id()), None);
    let mut finished = false;
    while let Ok(ev) = events.try_recv() {
        if let gtkterm_core::manager::ManagerEvent::ScriptFinished { error, .. } = ev {
            assert_eq!(error.as_deref(), Some("cancelled"));
            finished = true;
        }
    }
    assert!(finished, "ScriptFinished was sent");
    let quick = Script::parse("quick", "log hi\n", None).unwrap();
    let report = m.run_script(&s, &quick, &ScriptOptions::default()).await.unwrap();
    assert_eq!(report.log, ["hi"]);
}

/// `log` in a long loop keeps only the newest lines in the report.
#[tokio::test]
async fn the_report_log_is_bounded() {
    let (port, _dev) = loopback::pair("loop");
    let s = session_on(port).await;
    let text = "set n 0\ntop:\nlog line ${n}\nset n ${n}x\nif ${n} != 0xxxxxxxxxxxxxx goto top\n";
    let script = Script::parse("loop", text, None).unwrap();
    let report = run(&s, &script, &ScriptOptions::default()).await.unwrap();
    assert_eq!(report.log.len(), 14);
    let many = format!("{}log end\n", "log x\n".repeat(10_500));
    let report = run(
        &s,
        &Script::parse("many", &many, None).unwrap(),
        &ScriptOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(report.log.len(), 10_000);
    assert_eq!(report.log.last().unwrap(), "end");
}
