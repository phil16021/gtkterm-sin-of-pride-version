//! Developer tasks:
//!
//! * `cargo xtask check` — fmt, clippy and every test that can run here.
//! * `cargo xtask install [--prefix DIR] [--destdir DIR]` — release build and installation of the
//!   binary, desktop file, metainfo, icons, man page and shell completions (default prefix ~/.local).
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const APP_ID: &str = "at.philipp.GtkTermSunshine";
const BIN: &str = "gtkterm-sunshine";

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
    for krate in ["gtkterm-gui", "gtkterm-charts"] {
        run(
            "cargo",
            &[
                "clippy",
                "-p",
                krate,
                "--no-default-features",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )?;
    }
    run("cargo", &["test"])?;
    run("cargo", &["test", "-p", "gtkterm-gui", "--no-default-features"])?;
    run(
        "cargo",
        &["test", "-p", "gtkterm-charts", "--no-default-features"],
    )?;
    if have_gtk() {
        run(
            "cargo",
            &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
        )?;
        run("cargo", &["test", "-p", "gtkterm-mcp", "--features", "png"])?;
        run(
            "xvfb-run",
            &[
                "-a",
                "env",
                "GTKTERM_GUI_TESTS=1",
                "GSK_RENDERER=cairo",
                "cargo",
                "test",
                "-p",
                "gtkterm-gui",
                "-p",
                "gtkterm-charts",
            ],
        )?;
    } else {
        eprintln!("note: GTK development packages missing, skipping GUI crates");
    }
    Ok(())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_owned()
}

fn install_file(src: &Path, dest: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(src, dest).with_context(|| format!("{} -> {}", src.display(), dest.display()))?;
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode))?;
    eprintln!("  {}", dest.display());
    Ok(())
}

fn install(args: &[String]) -> Result<()> {
    let mut prefix = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".local"))
        .context("HOME not set")?;
    let mut destdir: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--prefix" => prefix = it.next().context("--prefix needs a value")?.into(),
            "--destdir" => destdir = Some(it.next().context("--destdir needs a value")?.into()),
            other => bail!("unknown option {other}"),
        }
    }
    let root = root();
    let features: &[&str] = if have_gtk() {
        &[]
    } else {
        &["--no-default-features"]
    };
    let mut build = vec!["build", "--release", "-p", "gtkterm-gui"];
    build.extend_from_slice(features);
    run("cargo", &build)?;
    let base = match &destdir {
        Some(d) => d.join(prefix.strip_prefix("/").unwrap_or(&prefix)),
        None => prefix.clone(),
    };
    eprintln!("installing into {}", base.display());
    let bin = root.join("target/release").join(BIN);
    install_file(&bin, &base.join("bin").join(BIN), 0o755)?;
    let data = root.join("data");
    install_file(
        &data.join(format!("{APP_ID}.desktop")),
        &base.join(format!("share/applications/{APP_ID}.desktop")),
        0o644,
    )?;
    install_file(
        &data.join(format!("{APP_ID}.metainfo.xml")),
        &base.join(format!("share/metainfo/{APP_ID}.metainfo.xml")),
        0o644,
    )?;
    for size in [48, 64, 128, 256] {
        let rel = format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png");
        install_file(&data.join(&rel), &base.join("share").join(&rel), 0o644)?;
    }
    let tmp = root.join("target/xtask-install");
    std::fs::create_dir_all(&tmp)?;
    run(
        bin.to_str().context("path")?,
        &["--generate-man", tmp.to_str().context("path")?],
    )?;
    install_file(
        &tmp.join(format!("{BIN}.1")),
        &base.join(format!("share/man/man1/{BIN}.1")),
        0o644,
    )?;
    for (shell, rel) in [
        ("bash", format!("share/bash-completion/completions/{BIN}")),
        ("zsh", format!("share/zsh/site-functions/_{BIN}")),
        ("fish", format!("share/fish/vendor_completions.d/{BIN}.fish")),
    ] {
        let out = Command::new(&bin)
            .args(["--generate-completions", shell])
            .output()?;
        if !out.status.success() {
            bail!("completion generation for {shell} failed");
        }
        let file = tmp.join(shell);
        std::fs::write(&file, out.stdout)?;
        install_file(&file, &base.join(rel), 0o644)?;
    }
    for example in ["example_embedded.script", "example_modem.script"] {
        install_file(
            &data.join("examples").join(example),
            &base.join(format!("share/gtkterm-sunshine/examples/{example}")),
            0o644,
        )?;
    }
    if destdir.is_none() {
        let share = base.join("share");
        let _ = Command::new("gtk4-update-icon-cache")
            .args(["-q", "-t", "-f"])
            .arg(share.join("icons/hicolor"))
            .status();
        let _ = Command::new("update-desktop-database")
            .arg(share.join("applications"))
            .status();
    }
    eprintln!("done. Run `{BIN}` or start \"GTKTerm Sunshine\" from the application menu.");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("install") => install(&args[1..]),
        _ => bail!("usage: cargo xtask check | cargo xtask install [--prefix DIR] [--destdir DIR]"),
    }
}
