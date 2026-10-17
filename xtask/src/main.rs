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

fn check() -> Result<()> {
    run("cargo", &["fmt", "--all", "--check"])?;
    run("cargo", &["clippy", "--all-targets", "--", "-D", "warnings"])?;
    run("cargo", &["test"])?;
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
