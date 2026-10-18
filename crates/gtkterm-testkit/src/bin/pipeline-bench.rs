//! End-to-end receive throughput without the GUI: a device writes N MiB into a PTY, a session
//! reads, stores, logs (optional) and scans them for Sunshine packets. Prints MiB/s and the peak
//! memory. Run with `cargo xtask bench-pipeline` (release build).

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Parser;
use gtkterm_core::config::{Connection, SerialConfig, TerminalConfig};
use gtkterm_core::session::{LogOp, Session};
use gtkterm_core::testing::PtyPair;
use gtkterm_protocols::logfmt::LogFormat;

#[derive(Debug, Parser)]
#[command(about = "Measure the session's receive pipeline")]
struct Args {
    /// MiB to send.
    #[arg(long, default_value_t = 50)]
    megabytes: usize,
    /// Plain text lines instead of Sunshine packets.
    #[arg(long)]
    plain: bool,
    /// Also log to a raw file (in a temporary directory).
    #[arg(long)]
    log: bool,
}

/// Peak resident memory of this process in KiB (`VmHWM`).
fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let total = args.megabytes << 20;
    let line: &[u8] = if args.plain {
        b"[  12.345] sensor task: sample 4711 ok, queue 3/16, heap 177 kB free\r\n"
    } else {
        b"{imu|Roll|231,76,60|19.59|Pitch|46,204,113|-4.43|Yaw|52,152,219|30.34}\r\n"
    };
    let data: Vec<u8> = line.iter().copied().cycle().take(total).collect();
    let pty = PtyPair::new().context("cannot create a PTY")?;
    let session = Session::spawn(1, "bench", TerminalConfig::default());
    session
        .open(Connection::Serial(SerialConfig {
            port: pty.path_str(),
            ..SerialConfig::default()
        }))
        .await?;
    let dir = std::env::temp_dir().join(format!("pipeline-bench-{}", std::process::id()));
    if args.log {
        std::fs::create_dir_all(&dir)?;
        session
            .log(LogOp::Start {
                path: dir.join("bench.log"),
                format: LogFormat::Raw,
                append: false,
                port_label: None,
            })
            .await?;
    }
    let started = Instant::now();
    let writer = tokio::spawn(async move {
        for piece in data.chunks(64 * 1024) {
            pty.write(piece).await?;
        }
        Ok::<_, std::io::Error>(pty)
    });
    let deadline = started + Duration::from_secs(300);
    while session.status().rx_bytes < total as u64 {
        if Instant::now() > deadline {
            bail!("only {} of {total} bytes after 300 s", session.status().rx_bytes);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let elapsed = started.elapsed();
    let _pty = writer.await??;
    let packets = session.with_sunshine(|s| s.stats().packets);
    if args.log {
        session.log(LogOp::Stop).await?;
        let logged = std::fs::metadata(dir.join("bench.log"))?.len();
        let _ = std::fs::remove_dir_all(&dir);
        if logged != total as u64 {
            bail!("the log has {logged} of {total} bytes");
        }
    }
    let mib = total as f64 / f64::from(1 << 20);
    println!(
        "{mib:.0} MiB in {:.2} s: {:.1} MiB/s, {packets} Sunshine packets, peak RSS {} MiB",
        elapsed.as_secs_f64(),
        mib / elapsed.as_secs_f64(),
        peak_rss_kib() / 1024
    );
    Ok(())
}
