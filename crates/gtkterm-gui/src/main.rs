//! GTKTerm Sunshine entry point: parses the command line and starts the GUI, the headless
//! terminal or the MCP server.
// A binary: `pub` inside modules only means crate-wide.
#![allow(unreachable_pub)]

mod cli;
#[cfg(feature = "gui")]
mod gui;
mod headless;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use crate::cli::{Cli, run_config_op};

/// What `--trace-file` records unless `GTKTERM_TRACE` says otherwise.
const TRACE_FILE_FILTER: &str = "info,gtkterm_core=debug,gtkterm_mcp=debug,gtkterm_sunshine=debug";

fn init_tracing(verbose: u8, trace_file: Option<&std::path::Path>) -> std::io::Result<()> {
    let default = match verbose {
        0 => "warn",
        1 => "info,gtkterm_core=info,gtkterm_mcp=info",
        2 => "info,gtkterm_core=debug,gtkterm_mcp=debug,gtkterm_sunshine=debug",
        _ => "debug,gtkterm_core=trace,gtkterm_mcp=trace,gtkterm_sunshine=trace",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    // Always stderr: stdout carries terminal data or the MCP stdio protocol.
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter);
    // JSON lines with the session/port spans of every event, for later analysis (jq).
    let file = match trace_file {
        Some(path) => {
            let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
            let filter = EnvFilter::try_from_env("GTKTERM_TRACE")
                .unwrap_or_else(|_| EnvFilter::new(TRACE_FILE_FILTER));
            Some(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_span_list(true)
                    .with_writer(std::sync::Mutex::new(file))
                    .with_filter(filter),
            )
        }
        None => None,
    };
    let registry = tracing_subscriber::registry().with(stderr).with(file);
    #[cfg(feature = "console")]
    let registry = registry.with(console_subscriber::spawn());
    registry.init();
    Ok(())
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("gtkterm-io")
        .build()
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli::packaging(&cli) {
        Ok(true) => return ExitCode::SUCCESS,
        Ok(false) => {}
        Err(e) => {
            eprintln!("gtkterm-sunshine: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = init_tracing(cli.verbose, cli.trace_file.as_deref()) {
        eprintln!("gtkterm-sunshine: cannot write the trace file: {e}");
        return ExitCode::FAILURE;
    }
    if let Some(addr) = cli.mcp_http.filter(|a| !a.ip().is_loopback()) {
        eprintln!(
            "gtkterm-sunshine: warning: the MCP endpoint listens on {addr}; anyone on the network who \
             learns the token can drive the device, and the token travels unencrypted"
        );
    }
    let resolved = match cli.resolve() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("gtkterm-sunshine: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(msg) = &resolved.import_message {
        eprintln!("{msg}");
    }
    if resolved.op.is_some() {
        return match run_config_op(resolved) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("gtkterm-sunshine: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let windowless = cli.mcp_stdio || cli.headless || !cfg!(feature = "gui");
    if !windowless {
        #[cfg(feature = "gui")]
        return gui::run(cli, resolved);
    }
    if !cli.files.is_empty() {
        eprintln!(
            "gtkterm-sunshine: script files open in the window's editor; use --script FILE without a window"
        );
        return ExitCode::from(2);
    }
    if !cli.mcp_stdio && !cli.headless && cli.mcp_http.is_none() {
        eprintln!("gtkterm-sunshine was built without the GUI; use --headless, --mcp-stdio or --mcp-http");
        return ExitCode::from(2);
    }
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("gtkterm-sunshine: cannot start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = rt.block_on(async move {
        if cli.mcp_stdio {
            headless::run_mcp_stdio(cli, resolved).await
        } else {
            headless::run_headless(cli, resolved).await
        }
    });
    // A pending blocking read on stdin would otherwise keep the runtime (and the process) alive.
    rt.shutdown_background();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gtkterm-sunshine: {e:#}");
            ExitCode::FAILURE
        }
    }
}
