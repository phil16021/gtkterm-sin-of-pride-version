//! Developer tasks: `cargo xtask check`.
use std::process::Command;

use anyhow::{Context, Result, bail};

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    eprintln!("$ {cmd} {}", args.join(" "));
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("spawning {cmd}"))?;
    if !status.success() {
        bail!("{cmd} {} failed with {status}", args.join(" "));
    }
    Ok(())
}

/// True if the GTK 4 / libadwaita / VTE development files are installed.
fn have_gtk() -> bool {
    ["gtk4", "libadwaita-1", "vte-2.91-gtk4"].iter().all(|p| {
        Command::new("pkg-config")
            .args(["--exists", p])
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn check() -> Result<()> {
    run("cargo", &["fmt", "--all", "--check"])?;
    run("cargo", &["clippy", "--all-targets", "--", "-D", "warnings"])?;
    run(
        "cargo",
        &[
            "clippy",
            "-p",
            "gtkterm-gui",
            "--no-default-features",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run("cargo", &["test"])?;
    run("cargo", &["test", "-p", "gtkterm-gui", "--no-default-features"])?;
    if have_gtk() {
        run(
            "cargo",
            &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
        )?;
        run(
            "xvfb-run",
            &["-a", "cargo", "test", "-p", "gtkterm-gui", "-p", "gtkterm-charts"],
        )?;
    } else {
        eprintln!("note: GTK development packages missing, skipping GUI crates");
    }
    Ok(())
}

fn main() -> Result<()> {
    let task = std::env::args().nth(1);
    if task.as_deref() == Some("check") {
        check()
    } else {
        bail!("usage: cargo xtask check")
    }
}
