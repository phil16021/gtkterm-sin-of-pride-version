//! Developer tasks:
//!
//! * `cargo xtask check` — what CI runs: fmt, translations, clippy, every test (nextest), cargo
//!   deny/audit/machete and the MCP Inspector schema check.
//! * `cargo xtask cov` — line coverage with per-crate minimums.
//! * `cargo xtask inspector` — the MCP Inspector lists the tools and checks their schemas.
//! * `cargo xtask bench-pipeline` — receive throughput without the GUI (criterion benches:
//!   `cargo bench -p gtkterm-protocols|gtkterm-core|gtkterm-charts`).
//! * `cargo xtask install [--prefix DIR] [--destdir DIR]` — release build and installation of the
//!   binary, desktop file, metainfo, icons, man page, shell completions and translations
//!   (default prefix ~/.local).
//! * `cargo xtask po` — update the translation template and merge it into `po/*.po`.
mod po;

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

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `cargo xtask check`: everything CI runs — formatting, translations, clippy, every test
/// (nextest; GUI under Xvfb with GTK criticals fatal), doctests, supply chain (deny, audit,
/// machete) and the MCP Inspector's schema check.
fn check() -> Result<()> {
    run("cargo", &["fmt", "--all", "--check"])?;
    po::check(&root())?;
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
    run("cargo", &["nextest", "run"])?;
    run("cargo", &["test", "--doc"])?;
    run(
        "cargo",
        &["nextest", "run", "-p", "gtkterm-gui", "--no-default-features"],
    )?;
    run(
        "cargo",
        &["nextest", "run", "-p", "gtkterm-charts", "--no-default-features"],
    )?;
    if have_gtk() {
        run(
            "cargo",
            &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
        )?;
        run(
            "cargo",
            &["nextest", "run", "-p", "gtkterm-mcp", "--features", "png"],
        )?;
        run(
            "xvfb-run",
            &[
                "-a",
                "env",
                "GTKTERM_GUI_TESTS=1",
                "GSK_RENDERER=cairo",
                "G_DEBUG=fatal-criticals",
                "cargo",
                "nextest",
                "run",
                "-p",
                "gtkterm-gui",
                "-p",
                "gtkterm-charts",
            ],
        )?;
    } else {
        eprintln!("note: GTK development packages missing, skipping GUI crates");
    }
    run("cargo", &["deny", "check"])?;
    if have("cargo-audit") {
        run("cargo", &["audit", "--deny", "warnings"])?;
    } else {
        eprintln!("note: cargo-audit missing (cargo install cargo-audit); cargo deny checked the advisories");
    }
    run("cargo-machete", &["."])?;
    if have("npx") {
        inspector()?;
    } else {
        eprintln!("note: npx missing, skipping the MCP Inspector check");
    }
    Ok(())
}

/// `cargo xtask inspector`: the official MCP Inspector lists the tools over stdio; its schema
/// portability check must find no errors.
fn inspector() -> Result<()> {
    run("cargo", &["build", "-p", "gtkterm-gui", "--no-default-features"])?;
    let bin = root().join("target/debug").join(BIN);
    eprintln!(
        "$ npx @modelcontextprotocol/inspector --cli {} --mcp-stdio --method tools/list",
        bin.display()
    );
    let out = Command::new("npx")
        .args(["-y", "@modelcontextprotocol/inspector@2.11.0", "--cli"])
        .arg(&bin)
        .args(["--mcp-stdio", "--method", "tools/list"])
        .output()
        .context("running the MCP Inspector")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        bail!("MCP Inspector failed: {stderr}");
    }
    let tools = stdout.matches("\"inputSchema\"").count();
    if tools < 40 {
        bail!("MCP Inspector listed only {tools} tools:\n{stdout}");
    }
    let summary = stderr
        .lines()
        .find(|l| l.starts_with("Schema portability"))
        .unwrap_or("");
    eprintln!("  {tools} tools; {summary}");
    if summary.contains(" errors") && !summary.contains(": 0 errors") {
        bail!(
            "tool schemas have portability errors; see `npx @modelcontextprotocol/inspector --cli ... --strict`"
        );
    }
    Ok(())
}

/// `cargo xtask bench-pipeline`: 50 MiB through a PTY into a session (release build), with
/// Sunshine packets, and as plain text with a log file.
fn bench_pipeline() -> Result<()> {
    run(
        "cargo",
        &[
            "build",
            "--release",
            "-p",
            "gtkterm-testkit",
            "--bin",
            "pipeline-bench",
        ],
    )?;
    let bin = root().join("target/release/pipeline-bench");
    let bin = bin.to_str().context("path")?;
    run(bin, &["--megabytes", "50"])?;
    run(bin, &["--megabytes", "50", "--plain", "--log"])
}

/// Line coverage that `cargo xtask cov` requires, per crate.
const COVERAGE: [(&str, f64); 5] = [
    ("gtkterm-protocols", 85.0),
    ("gtkterm-charts", 85.0),
    ("gtkterm-core", 70.0),
    ("gtkterm-mcp", 70.0),
    ("gtkterm-testkit", 0.0),
];

/// `cargo xtask cov`: line coverage of the library crates (llvm-cov over nextest) with
/// per-crate minimums; the HTML report goes to `target/llvm-cov/html`.
fn coverage() -> Result<()> {
    let root = root();
    let json = root.join("target/llvm-cov/summary.json");
    std::fs::create_dir_all(json.parent().context("path")?)?;
    let mut args = vec!["llvm-cov", "nextest"];
    for (krate, _) in COVERAGE {
        args.extend(["-p", krate]);
    }
    args.extend(["--json", "--summary-only", "--output-path"]);
    let path = json.to_str().context("path")?.to_owned();
    args.push(&path);
    run("cargo", &args)?;
    run("cargo", &["llvm-cov", "report", "--html"])?;
    let text = std::fs::read_to_string(&json)?;
    let mut failed = Vec::new();
    for (krate, min) in COVERAGE {
        let (covered, count) = crate_lines(&text, &format!("/crates/{krate}/src/"));
        let pct = if count == 0 {
            100.0
        } else {
            covered as f64 * 100.0 / count as f64
        };
        eprintln!("  {krate:<20} {pct:5.1} % of {count} lines (minimum {min} %)");
        if pct < min {
            failed.push(krate);
        }
    }
    if !failed.is_empty() {
        bail!("coverage below the minimum: {}", failed.join(", "));
    }
    Ok(())
}

/// Covered and total lines of the files below `dir` in an llvm-cov JSON summary.
fn crate_lines(json: &str, dir: &str) -> (u64, u64) {
    // {"filename":"/…/crates/x/src/a.rs","summary":{…,"lines":{"count":N,"covered":M,…}…}}
    let mut covered = 0;
    let mut count = 0;
    for part in json.split("{\"filename\":\"").skip(1) {
        let Some((file, rest)) = part.split_once('"') else {
            continue;
        };
        if !file.contains(dir) {
            continue;
        }
        let Some(lines) = rest.split("\"lines\":{").nth(1) else {
            continue;
        };
        let num = |key: &str| {
            lines
                .split(&format!("\"{key}\":"))
                .nth(1)
                .and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
        };
        count += num("count");
        covered += num("covered");
    }
    (covered, count)
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

/// Where an installed file comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    /// The release binary.
    Binary,
    /// A file below `data/`.
    Data(String),
    /// A file generated into `target/xtask-install/`.
    Generated(String),
}

/// Every installed file: source, destination relative to the prefix, mode.
fn layout(languages: &[String]) -> Vec<(Source, String, u32)> {
    let mut files = vec![(Source::Binary, format!("bin/{BIN}"), 0o755)];
    for (file, dir) in [
        (format!("{APP_ID}.desktop"), "applications"),
        (format!("{APP_ID}.metainfo.xml"), "metainfo"),
    ] {
        files.push((
            Source::Generated(file.clone()),
            format!("share/{dir}/{file}"),
            0o644,
        ));
    }
    for lang in languages {
        let rel = format!("locale/{lang}/LC_MESSAGES/{}.mo", po::DOMAIN);
        files.push((
            Source::Generated(format!("share/{rel}")),
            format!("share/{rel}"),
            0o644,
        ));
    }
    for size in [48, 64, 128, 256] {
        let rel = format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png");
        files.push((Source::Data(rel.clone()), format!("share/{rel}"), 0o644));
    }
    files.push((
        Source::Generated(format!("{BIN}.1")),
        format!("share/man/man1/{BIN}.1"),
        0o644,
    ));
    for (shell, rel) in [
        ("bash", format!("share/bash-completion/completions/{BIN}")),
        ("zsh", format!("share/zsh/site-functions/_{BIN}")),
        ("fish", format!("share/fish/vendor_completions.d/{BIN}.fish")),
    ] {
        files.push((Source::Generated(shell.to_owned()), rel, 0o644));
    }
    for lang in [
        "gtkterm-macro.lang",
        "gtkterm-script.lang",
        "at-commands.lang",
        "hexdump.lang",
        "modbus-ascii.lang",
        "nmea.lang",
    ] {
        files.push((
            Source::Data(format!("language-specs/{lang}")),
            format!("share/{BIN}/language-specs/{lang}"),
            0o644,
        ));
    }
    for example in ["example_embedded.script", "example_modem.script"] {
        files.push((
            Source::Data(format!("examples/{example}")),
            format!("share/{BIN}/examples/{example}"),
            0o644,
        ));
    }
    files
}

/// Generate the files of [`Source::Generated`] into `tmp`.
fn generate(root: &Path, bin: &Path, tmp: &Path) -> Result<()> {
    std::fs::create_dir_all(tmp)?;
    let data = root.join("data");
    for (kind, file) in [
        ("--desktop", format!("{APP_ID}.desktop")),
        ("--xml", format!("{APP_ID}.metainfo.xml")),
    ] {
        po::translate_data(root, kind, &data.join(&file), &tmp.join(&file))?;
    }
    po::compile(root, &tmp.join("share"))?;
    run(
        bin.to_str().context("path")?,
        &["--generate-man", tmp.to_str().context("path")?],
    )?;
    for shell in ["bash", "zsh", "fish"] {
        let out = Command::new(bin)
            .args(["--generate-completions", shell])
            .output()?;
        if !out.status.success() {
            bail!("completion generation for {shell} failed");
        }
        std::fs::write(tmp.join(shell), out.stdout)?;
    }
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
    let tmp = root.join("target/xtask-install");
    generate(&root, &bin, &tmp)?;
    for (source, rel, mode) in layout(&po::languages(&root)) {
        let src = match source {
            Source::Binary => bin.clone(),
            Source::Data(p) => root.join("data").join(p),
            Source::Generated(p) => tmp.join(p),
        };
        install_file(&src, &base.join(rel), mode)?;
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
        Some("po") => po::update(&root()),
        Some("cov") => coverage(),
        Some("inspector") => inspector(),
        Some("bench-pipeline") => bench_pipeline(),
        _ => {
            bail!(
                "usage: cargo xtask check | cov | inspector | bench-pipeline | po | install [--prefix DIR] [--destdir DIR]"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression I1: the C fork installed `gtkterm`, `gtkterm.desktop`, its icon and config
    /// under the upstream names, so it replaced or collided with GTKTerm 1.x.
    #[test]
    fn c_i1_installed_paths_do_not_collide_with_upstream() {
        let files = layout(&["de".to_owned()]);
        for (_, rel, _) in &files {
            for part in rel.split('/') {
                let stem = part.split('.').next().unwrap_or(part).trim_start_matches('_');
                assert_ne!(stem, "gtkterm", "{rel} uses the upstream name");
            }
        }
        assert!(files.iter().any(|(_, r, _)| r == "bin/gtkterm-sunshine"));
        let mut rels: Vec<_> = files.iter().map(|(_, r, _)| r).collect();
        rels.sort();
        rels.dedup();
        assert_eq!(rels.len(), files.len(), "two sources for one destination");
        // Data sources exist.
        for (source, rel, _) in &files {
            if let Source::Data(p) = source {
                assert!(root().join("data").join(p).is_file(), "{rel}: missing data/{p}");
            }
        }
    }

    #[test]
    fn coverage_summary_is_read_per_crate() {
        let json = r#"{"data":[{"files":[{"filename":"/w/crates/a/src/x.rs","summary":{"lines":{"count":10,"covered":7,"percent":70}}},{"filename":"/w/crates/b/src/y.rs","summary":{"lines":{"count":4,"covered":1}}},{"filename":"/w/crates/a/src/z.rs","summary":{"functions":{"count":99,"covered":0},"lines":{"count":10,"covered":10}}}]}]}"#;
        assert_eq!(crate_lines(json, "/crates/a/src/"), (17, 20));
        assert_eq!(crate_lines(json, "/crates/b/src/"), (1, 4));
        assert_eq!(crate_lines(json, "/crates/c/src/"), (0, 0));
    }

    #[test]
    fn c_i1_desktop_file_names_this_app() {
        let desktop = std::fs::read_to_string(root().join(format!("data/{APP_ID}.desktop"))).unwrap();
        assert!(
            desktop.lines().any(|l| l.starts_with("Exec=gtkterm-sunshine")),
            "{desktop}"
        );
        assert!(
            desktop.lines().any(|l| l == format!("Icon={APP_ID}")),
            "{desktop}"
        );
    }
}
