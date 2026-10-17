# CLAUDE.md

Guidance for Claude Code in this repository.

## Project

GTKTerm Sunshine: a Linux serial/TCP/UDP terminal with live data visualization ("Sunshine" packets)
and a built-in MCP server. It is being **rewritten in Rust** on the `rust-rewrite` branch; the
original C/GTK4 code is preserved at tag `legacy-c-v1.99.1` (use `git show legacy-c-v1.99.1:src/...`
to consult it). The plan, compatibility contract and C-defect regression list are in `docs/ROADMAP.md`.

## Workspace

| Crate | Role | Needs GTK dev libs |
|---|---|---|
| `crates/gtkterm-protocols` | sans-IO, pure: view formatter, transforms, hex parser, Sunshine parser/scanner, decoders, XMODEM/YMODEM, log formats | no |
| `crates/gtkterm-core` | tokio I/O: ports (serial via nix/termios2, TCP, UDP, loopback), sessions, config, logging | no |
| `crates/gtkterm-mcp` | rmcp MCP server (stdio + Streamable HTTP) | no |
| `crates/gtkterm-charts` | cairo chart models/renderers | cairo |
| `crates/gtkterm-gui` | bin `gtkterm-sunshine`: gtk4-rs + libadwaita + vte4 | yes |
| `crates/gtkterm-testkit` | loopback/PTY helpers, fake devices, `sunshine-gen` | no |
| `xtask` | `cargo xtask check` | no |

`default-members` excludes the GTK/cairo crates so `cargo build`/`cargo test` work without the
GUI dev packages; use `-p gtkterm-gui` / `--workspace` for those.

## Commands

```bash
cargo test                              # default members (no GTK needed)
cargo test --workspace                  # everything (needs libgtk-4-dev, libadwaita-1-dev, libvte-2.91-gtk4-dev)
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask check                       # fmt + clippy + all tests (GUI under xvfb-run when GTK dev libs exist)
cargo xtask install --destdir $SCRATCH/stage   # packaging check
GTKTERM_GUI_TESTS=1 GSK_RENDERER=cairo xvfb-run -a cargo test -p gtkterm-gui
cargo +nightly fuzz run <target>        # fuzz/ (cargo-fuzz)
```

Virtual serial ports for manual testing:
`socat -d -d pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB`

## Conventions

- Tests first: every fix/feature lands with a test; C defects from ROADMAP §8 each get a named regression test.
- `unsafe` is denied workspace-wide; only `gtkterm-core`'s serial `sys` module may allow it, with `// SAFETY:` comments.
- Protocol logic stays sans-IO in `gtkterm-protocols` (no clocks, no I/O; timestamps are passed in).
- GTK objects are only touched on the main thread; I/O runs on the tokio runtime and talks to the GUI via channels.
- Plain Ctrl+letter shortcuts belong to the device; app shortcuts use Ctrl+Shift or F-keys. Never bind Super+Space.
- GUI code (`crates/gtkterm-gui/src/gui/`) uses plain gtk4-rs widgets wired in Rust (no templates);
  `#[gtk::test]` tests build the real window on a loopback port and fail on GTK warnings
  (two documented upstream warnings are whitelisted in `window.rs`).
- The GUI test only runs with `GTKTERM_GUI_TESTS=1` (never on the real desktop); it writes screenshots to `target/gui-screenshots/`.
- Sunshine packet format: `{ID|Name|R,G,B|value|Name2|value2…}`; optional chart prefix `L:`/`B:`/`P:`/`S:` on the ID.
