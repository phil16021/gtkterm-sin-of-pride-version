//! End-to-end tests of the `gtkterm-sunshine` binary (headless modes).
#![allow(clippy::unwrap_used)]

use std::process::Stdio;
use std::time::Duration;

use assert_cmd::Command;
use gtkterm_core::testing::PtyPair;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BIN: &str = env!("CARGO_BIN_EXE_gtkterm-sunshine");

fn cmd(config: &std::path::Path) -> Command {
    let mut c = Command::new(BIN);
    c.arg("--config").arg(config);
    c
}

#[test]
fn help_and_version() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    let help = String::from_utf8(out.stdout).unwrap();
    for opt in [
        "--port",
        "--baudrate",
        "--term_echo",
        "--show_config",
        "--mcp-stdio",
        "--headless",
        "--tcp",
    ] {
        assert!(help.contains(opt), "help lacks {opt}");
    }
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("gtkterm-sunshine")
    );
}

#[test]
fn config_operations() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    cmd(&cfg)
        .args([
            "-U",
            "bench",
            "-p",
            "/dev/ttyUSB7",
            "-s",
            "250000",
            "-e",
            "on",
            "-S",
        ])
        .assert()
        .success();
    let out = cmd(&cfg).arg("-L").output().unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Configurations found:\n\n[bench]\n[default]\n"
    );
    let out = cmd(&cfg).args(["-V", "bench"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("baudrate                 : 250000")
            && text.contains("term_echo                : true"),
        "{text}"
    );
    let out = cmd(&cfg).args(["-b", "12"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr).unwrap().contains("bits"));
}

#[tokio::test]
async fn headless_terminal_on_pty() {
    let dir = tempfile::tempdir().unwrap();
    let pty = PtyPair::new().unwrap();
    let log = dir.path().join("session.txt");
    let mut child = tokio::process::Command::new(BIN)
        .arg("--config")
        .arg(dir.path().join("c.toml"))
        .args([
            "--headless",
            "-vv",
            "-p",
            &pty.path_str(),
            "-g",
            "on",
            "--log-file",
        ])
        .arg(&log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();

    // Wait until the binary has opened the port (stdin data is only sent once connected).
    let mut stderr = child.stderr.take().unwrap();
    let (err_tx, mut err_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            let _ = err_tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
        }
    });
    let mut errlog = String::new();
    while !errlog.contains("connected to") {
        match tokio::time::timeout(Duration::from_secs(5), err_rx.recv()).await {
            Ok(Some(s)) => errlog.push_str(&s),
            other => panic!(
                "not connected ({other:?}, exit {:?}); stderr:\n{errlog}",
                child.try_wait()
            ),
        }
    }

    pty.write(b"hello\nworld").await.unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while !got.ends_with(b"world") {
        let Ok(read) = tokio::time::timeout(Duration::from_secs(3), stdout.read(&mut buf)).await else {
            while let Ok(s) = err_rx.try_recv() {
                errlog.push_str(&s);
            }
            panic!("no stdout; got {got:?}; stderr:\n{errlog}");
        };
        let n = read.unwrap();
        assert!(n > 0, "stdout closed");
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"hello\r\nworld", "auto CR (-g on) inserts CR before LF");

    stdin.write_all(b"AT\r").await.unwrap();
    stdin.flush().await.unwrap();
    assert_eq!(pty.read_exact(3).await.unwrap(), b"AT\r");

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id().unwrap() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "{status:?}");
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(
        logged.contains("R: hello<0A>") && logged.contains("T: AT<0D>"),
        "{logged}"
    );
}

#[derive(Debug, Clone, Default)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

#[tokio::test]
async fn mcp_stdio_drives_a_pty() {
    let dir = tempfile::tempdir().unwrap();
    let pty = PtyPair::new().unwrap();
    let mut command = tokio::process::Command::new(BIN);
    command
        .arg("--config")
        .arg(dir.path().join("c.toml"))
        .arg("--mcp-stdio")
        .env("XDG_DATA_HOME", dir.path());
    let client = Client
        .serve(TokioChildProcess::new(command).unwrap())
        .await
        .expect("MCP handshake with the binary");

    let call = |name: &str, args: serde_json::Value| {
        CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().cloned().unwrap())
    };
    let r = client
        .call_tool(call(
            "open_port",
            json!({"connection": {"type": "serial", "port": pty.path_str()}}),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{:?}", r.content);

    pty.write(b"{env|Temp|21.5}\n").await.unwrap();
    let mut listed = serde_json::Value::Null;
    for _ in 0..100 {
        listed = client
            .call_tool(call("sunshine_list_series", json!({})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        if listed["stats"]["packets"] == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(listed["charts"][0]["series"][0]["last"], json!([21.5]));

    let r = client
        .call_tool(call("send", json!({"data": "ping", "line_ending": "lf"})))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true));
    assert_eq!(pty.read_exact(5).await.unwrap(), b"ping\n");
    client.cancel().await.unwrap();

    // Every call is in the audit log.
    let audit = std::fs::read_to_string(dir.path().join("gtkterm-sunshine/mcp-audit.jsonl")).unwrap();
    let tools: Vec<String> = audit
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["tool"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(tools.first().map(String::as_str), Some("open_port"));
    assert_eq!(tools.last().map(String::as_str), Some("send"));
    assert!(audit.contains(r#""data":"ping""#), "{audit}");
}

#[tokio::test]
async fn headless_script_exits_with_its_result() {
    let dir = tempfile::tempdir().unwrap();
    let pty = PtyPair::new().unwrap();
    let script = dir.path().join("probe.script");
    std::fs::write(&script, "send AT\\r\nexpect[3000] OK\nlog modem ok\n").unwrap();
    let run = |script: &std::path::Path| {
        tokio::process::Command::new(BIN)
            .arg("--config")
            .arg(dir.path().join("c.toml"))
            .args(["--headless", "-p", &pty.path_str(), "--script"])
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
    };
    let (out, ()) = tokio::join!(run(&script), async {
        let got = tokio::time::timeout(Duration::from_secs(5), pty.read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, b"AT\r");
        pty.write(b"\r\nOK\r\n").await.unwrap();
    });
    let out = out.unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("[script] modem ok"), "{err}");

    std::fs::write(&script, "log waiting\nexpect[200] never\n").unwrap();
    let out = run(&script).await.unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("probe.script:2: timed out after 200 ms"), "{err}");

    std::fs::write(&script, "bogus\n").unwrap();
    let out = run(&script).await.unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("probe.script:1: unknown command"));
}

/// Sessions of a running window, through its embedded MCP endpoint (raw HTTP, 2026-07-28).
fn mcp_sessions(port: u16, token: &str) -> String {
    use std::io::{Read as _, Write as _};
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_ports","arguments":{},"_meta":{"io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"},"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nMcp-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\n\
         Mcp-Name: list_ports\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

/// A second launch hands its command line to the running window: with a port it adds a tab
/// that connects; without arguments it only raises the window. Runs on a private D-Bus session
/// (never the desktop's, where it could reach a real window) and needs an X server (xvfb-run).
#[test]
fn second_launch_opens_a_tab_in_the_running_window() {
    if std::env::var_os("GTKTERM_GUI_TESTS").is_none() || std::env::var_os("DISPLAY").is_none() {
        return;
    }
    if std::env::var_os("GTKTERM_PRIVATE_BUS").is_none() {
        // Re-run this test inside its own bus.
        let status = std::process::Command::new("dbus-run-session")
            .args(["--", std::env::current_exe().unwrap().to_str().unwrap()])
            .args([
                "--exact",
                "second_launch_opens_a_tab_in_the_running_window",
                "--nocapture",
            ])
            .env("GTKTERM_PRIVATE_BUS", "1")
            .status()
            .expect("dbus-run-session (package dbus) is needed for this test");
        assert!(status.success());
        return;
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _rt = rt.enter(); // the PTY pairs register with the reactor
    let home = tempfile::tempdir().unwrap();
    let env = |c: &mut std::process::Command| {
        c.env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .env("GSK_RENDERER", "cairo")
            // A private bus would start its own accessibility bus, whose cleanup removes the
            // desktop's at-spi socket; tests need no accessibility.
            .env("GTK_A11Y", "none")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
    };
    let a = PtyPair::new().unwrap();
    let b = PtyPair::new().unwrap();
    let mcp_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut primary = std::process::Command::new(BIN);
    primary.args([
        "-p",
        &a.path_str(),
        "--mcp-http",
        &mcp_port.to_string(),
        "--mcp-token",
        "t0k",
    ]);
    env(&mut primary);
    let mut primary = primary.spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !mcp_sessions_ready(mcp_port) {
        assert!(std::time::Instant::now() < deadline, "the window did not come up");
        std::thread::sleep(Duration::from_millis(200));
    }
    let run = |args: &[&str]| {
        let mut c = std::process::Command::new(BIN);
        c.args(args);
        env(&mut c);
        c.output().unwrap()
    };
    // No arguments: the window is raised, nothing else happens.
    assert!(run(&[]).status.success());
    // A port: a new tab connects to it; options of the window itself are reported as ignored.
    let out = run(&[
        "--new-tab",
        "-p",
        &b.path_str(),
        "-s",
        "9600",
        "--log-file",
        "x.log",
    ]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ignored: --log-file"),
        "{out:?}"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let sessions = mcp_sessions(mcp_port, "t0k");
        if sessions.contains(&format!("{} 9600-8-N-1", b.path_str())) {
            assert!(sessions.contains(&a.path_str()), "{sessions}");
            assert_eq!(
                sessions.matches("\"id\":").count() - 1,
                2,
                "two sessions: {sessions}"
            );
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no new tab: {sessions}");
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = primary.kill();
    let _ = primary.wait();
}

fn mcp_sessions_ready(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        && mcp_sessions(port, "t0k").contains("sessions")
}
