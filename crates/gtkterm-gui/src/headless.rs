//! Modes without a window: terminal on stdin/stdout, MCP over stdio, MCP over HTTP.

use std::io::Write as _;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use gtkterm_core::chunks::Direction;
use gtkterm_core::manager::{ManagerEvent, SessionManager};
use gtkterm_core::script::{Script, ScriptOptions};
use gtkterm_core::session::{LogOp, PortState, Session, SessionEvent};
use gtkterm_mcp::transport::{HttpSecurity, random_token, serve_http, serve_stdio};
use gtkterm_mcp::{GtkTermMcp, ServerOptions};
use gtkterm_protocols::transform::{Transform, TransformOptions};
use gtkterm_protocols::view::{ViewFormatter, ViewMode, ViewSettings};
use tokio::io::AsyncReadExt;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::broadcast::error::RecvError;

use crate::cli::{Cli, Resolved};

/// Shared setup: a session manager with the resolved profile's session.
fn setup(r: &Resolved) -> (Arc<SessionManager>, Session) {
    let manager = SessionManager::new(r.config.clone(), Some(r.config_path.clone()));
    manager.set_macros(r.macros.0.clone(), Some(r.macros.1.clone()));
    let session = manager.create(&r.profile_name, r.profile.terminal.clone());
    (manager, session)
}

async fn start_log(cli: &Cli, session: &Session, port_label: String) -> Result<()> {
    if let Some(path) = &cli.log_file {
        let format = cli.effective_log_format();
        session
            .log(LogOp::Start {
                path: path.clone(),
                format,
                append: true,
                port_label: Some(port_label),
            })
            .await
            .with_context(|| format!("cannot log to {}", path.display()))?;
        eprintln!("logging ({format:?}) to {}", path.display());
    }
    Ok(())
}

fn mcp_options(cli: &Cli) -> ServerOptions {
    ServerOptions {
        read_only: cli.mcp_read_only,
        allowed_dirs: cli.mcp_allow_dir.clone(),
        allow_exec: cli.mcp_allow_exec,
        audit: Some(gtkterm_mcp::default_audit_path()),
        ..ServerOptions::default()
    }
}

/// `--mcp-stdio`: serve one MCP client on stdin/stdout. Diagnostics go to stderr.
pub async fn run_mcp_stdio(cli: Cli, r: Resolved) -> Result<()> {
    let (manager, session) = setup(&r);
    start_log(&cli, &session, r.profile.connection.summary()).await?;
    if r.explicit_connection {
        session
            .open(r.profile.connection.clone())
            .await
            .context("cannot open the requested connection")?;
    }
    serve_stdio(GtkTermMcp::new(manager, mcp_options(&cli))).await?;
    Ok(())
}

async fn start_http(
    cli: &Cli,
    manager: &Arc<SessionManager>,
) -> Result<Option<gtkterm_mcp::transport::HttpHandle>> {
    let Some(addr) = cli.mcp_http else { return Ok(None) };
    let token = cli.mcp_token.clone().unwrap_or_else(random_token);
    let security = HttpSecurity {
        token: Some(token.clone()),
        ..HttpSecurity::default()
    };
    let handle = serve_http(
        GtkTermMcp::new(Arc::clone(manager), mcp_options(cli)),
        addr,
        security,
    )
    .await
    .with_context(|| format!("cannot listen on {addr}"))?;
    eprintln!("MCP endpoint: {}  (Authorization: Bearer {token})", handle.url);
    eprintln!(
        "  claude mcp add --transport http gtkterm {} --header \"Authorization: Bearer {token}\"",
        handle.url
    );
    Ok(Some(handle))
}

/// `--headless` (and `--mcp-http` without a GUI).
pub async fn run_headless(cli: Cli, r: Resolved) -> Result<()> {
    let (manager, session) = setup(&r);
    let http = start_http(&cli, &manager).await?;
    if !cli.headless {
        // MCP over HTTP only: open the connection if one was given, then serve until Ctrl-C.
        if r.explicit_connection {
            session
                .open(r.profile.connection.clone())
                .await
                .context("cannot open the requested connection")?;
        }
        tokio::signal::ctrl_c().await?;
        if let Some(h) = http {
            h.shutdown().await;
        }
        return Ok(());
    }

    // Parse the script first: a typo should not wait for a device.
    let script = match &cli.script {
        Some(path) => Some(Script::load(path, None).map_err(|e| anyhow::anyhow!("{e}"))?),
        None => None,
    };
    // Subscribe and start logging before opening so no early data is missed.
    let mut events = session.subscribe();
    start_log(&cli, &session, r.profile.connection.summary()).await?;
    session
        .open(r.profile.connection.clone())
        .await
        .with_context(|| format!("cannot open {}", r.profile.connection.summary()))?;
    eprintln!("connected to {} — Ctrl-C to quit", session.status().description);

    let terminal = r.profile.terminal.clone();
    let mut transform = Transform::new(
        TransformOptions {
            auto_cr: terminal.auto_cr,
            auto_lf: terminal.auto_lf,
            timestamps: terminal.timestamps,
            timestamp_format: terminal.timestamp_format,
        },
        jiff::tz::TimeZone::system(),
        jiff::Timestamp::now(),
    );
    let mut hexdump = cli.hexdump.then(|| {
        ViewFormatter::new(ViewSettings {
            mode: ViewMode::Mixed,
            show_index: true,
            hex_chars: 16,
            columns: 80,
        })
    });

    let stdin_session = session.clone();
    let stdin_task = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = stdin.read(&mut buf).await {
            if n == 0 || stdin_session.send(buf[..n].to_vec()).await.is_err() {
                break;
            }
        }
    });
    // Signal streams are created once: a signal arriving between loop iterations is kept.
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut usr1 = signal(SignalKind::user_defined1())?;
    let mut usr2 = signal(SignalKind::user_defined2())?;
    let mut stdout = std::io::stdout();
    let mut script_done: tokio::task::JoinHandle<Option<Result<(), String>>> = match script {
        Some(script) => {
            let (m, s) = (Arc::clone(&manager), session.clone());
            let opts = ScriptOptions {
                allow_exec: cli.allow_exec,
                ..ScriptOptions::default()
            };
            let mut log = manager.subscribe();
            tokio::spawn(async move {
                let printer = tokio::spawn(async move {
                    loop {
                        let ev = match log.recv().await {
                            Ok(ev) => ev,
                            Err(RecvError::Lagged(n)) => {
                                eprintln!("[{n} script events lost]");
                                continue;
                            }
                            Err(RecvError::Closed) => break,
                        };
                        match ev {
                            ManagerEvent::ScriptLog { message, .. } => eprintln!("[script] {message}"),
                            ManagerEvent::ScriptFinished { .. } => break,
                            _ => {}
                        }
                    }
                });
                let r = m.run_script(&s, &script, &opts).await;
                let _ = printer.await;
                Some(r.map(|_| ()).map_err(|e| e.to_string()))
            })
        }
        None => tokio::spawn(std::future::pending()),
    };
    let result = loop {
        tokio::select! {
            done = &mut script_done => match done {
                Ok(Some(Ok(()))) => {
                    eprintln!("[script finished]");
                    break Ok(());
                }
                Ok(Some(Err(e))) => break Err(anyhow::anyhow!("script failed: {e}")),
                _ => break Ok(()),
            },
            _ = sigint.recv() => break Ok(()),
            _ = sigterm.recv() => break Ok(()),
            _ = usr1.recv() => {
                if let Err(e) = session.open(r.profile.connection.clone()).await {
                    eprintln!("SIGUSR1: cannot reopen: {e}");
                }
            }
            _ = usr2.recv() => { let _ = session.close().await; }
            ev = events.recv() => match ev {
                Ok(SessionEvent::Data(c)) if c.dir == Direction::Rx || terminal.echo => {
                    let out = match &mut hexdump {
                        Some(f) => f.format(&c.data),
                        None => transform.apply(&c.data, c.ts),
                    };
                    stdout.write_all(&out)?;
                    stdout.flush()?;
                }
                Ok(SessionEvent::Info(msg)) => eprintln!("[{msg}]"),
                Ok(SessionEvent::State(PortState::Error(e))) => break Err(anyhow::anyhow!("port error: {e}")),
                Ok(SessionEvent::State(PortState::Closed)) => eprintln!("[port closed; SIGUSR1 reopens]"),
                Err(RecvError::Lagged(n)) => eprintln!("[{n} events lost: output was too slow]"),
                Ok(_) => {}
                Err(RecvError::Closed) => bail!("session ended"),
            }
        }
    };
    stdin_task.abort();
    if session.status().log.is_some() {
        let _ = session.log(LogOp::Stop).await;
    }
    let _ = session.close().await;
    if let Some(h) = http {
        h.shutdown().await;
    }
    result
}
