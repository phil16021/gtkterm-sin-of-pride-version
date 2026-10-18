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
        .arg("--mcp-stdio");
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
