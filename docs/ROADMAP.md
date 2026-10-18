# GTKTerm Sunshine — Rust Rewrite: Implementation, Test & Debugging Plan

> **Status 2026-10-18:** R0–R9 done (2.0.0-rc.1, on master). After a second audit (plan vs code,
> robustness, user view) the plan continues with R10–R15 (see the plan file mirrored below):
> R10 stabilization is **done**: stuck script slot after MCP timeouts, UDP/TCP write handling and
> slow TCP clients, GUI throughput at high data rates, incremental `expect`, EtherCAT resync,
> cancellable connect and breaks beside the actor, sysfs UART detection (no DTR glitch), bounded
> serial close, lag handling, bounded/flushable send queue, streamed raw file send, durable saves,
> MCP chart notifications, per-client approval, profile autosave and naming, action enabling,
> hardening (0600 logs, exec paths, token source) and cleanup. Soak: 4 min at ~8000 packets/s,
> RSS flat at ~190 MB. Next: R11 tabs, R12 daily-use features, then 2.0.0.

## Context

The C fork (21k LOC, GTK4) was audited in depth:
- About 60 confirmed defects, including heap overflows in the receive buffer, double frees, broken XMODEM, stack corruption in the CLI signals, and Sunshine plotting that dies after 10,000 chunks or on split packets.
- About half the modules are compiled but never wired in.
- There is no test suite, and the build is broken on a fresh clone.

The user decided to **rewrite the application in Rust**:
- **Toolkit:** gtk4-rs + libadwaita + vte4.
- **Platform:** Linux only.
- **Repo:** a Cargo workspace in this repo. The C version is preserved on a tag/branch.
- **Earlier decisions still apply:** robust daily-use tool, its own app-id next to upstream gtkterm, newest libraries everywhere, a **full MCP server**, and eventually every feature of the C version (including the dormant modules).
- **First usable release:** core I/O + terminal + logging + hex send + Sunshine + MCP. Decoders, file transfer, macros, scripts, presets and the signal monitor follow in later milestones.

What carries over from the C code:
- **Behaviour spec and file formats** (§3), extracted from the C sources, so configs, macros, presets, scripts and logs stay compatible.
- **The defect list** (§8), which becomes acceptance tests for the Rust version.
- **Assets:** icons, `data/language-specs/*.lang`, `data/examples/*.script`.

Already done before the pivot: branch `roadmap/phase0-foundation`, commit 16ca59d (removed artifacts, moved the analysis docs to `docs/analysis/`, docs index). It stays as the base.

---

## 1. Target architecture

### 1.1 Workspace layout (repo root)
```
Cargo.toml                 workspace: edition 2024, resolver 3, rust-version 1.92, shared [workspace.dependencies] + [workspace.lints]
crates/
  gtkterm-protocols/       sans-IO, pure, #![forbid(unsafe_code)]: hex/bin/dec view formatter, hex-send parser,
                           CR/LF+timestamp transform, Sunshine parser+scanner, decoders + frame assemblers,
                           XMODEM/1K/YMODEM state machines, CRC/LRC/checksums, log record formatters
  gtkterm-core/            tokio I/O: Port trait + Serial/TCP/UDP/Loopback, ChunkStore, Session actor, SessionManager,
                           config (TOML + legacy importers), logging writer, macros, presets, automation runtime,
                           transfer driver, Sunshine store, udev enumeration/hotplug. `unsafe` only in serial/sys.rs (ioctls)
  gtkterm-charts/          cairo-rs only (no GTK): chart models (line/bar/pie/scatter), scale/ticks/palette/LTTB,
                           cairo renderers → used by GUI snapshot and headless PNG export
  gtkterm-mcp/             rmcp server: tools/resources/prompts, policy, audit, stdio + Streamable-HTTP (axum) transports
  gtkterm-gui/             bin `gtkterm-sunshine`: adw::Application, windows/widgets (composite templates), VTE view,
                           chart widgets, dialogs; also hosts CLI entry (clap) and headless/MCP modes
  gtkterm-testkit/         loopback port, PTY pair, fake devices (AT modem, Modbus slave, NMEA GPS, Sunshine emitter),
                           bin `sunshine-gen`
xtask/                     `cargo xtask {install,dist,man,completions,po,check}`
fuzz/                      cargo-fuzz targets
data/                      icons, .desktop.in, metainfo.xml, ui/*.ui, gresource.xml, language-specs, examples
po/                        gettext catalogs
docs/                      README, ROADMAP (this plan), mcp.md, debugging.md, testing/hardware.md, analysis/ (historic)
```

### 1.2 Dependencies (newest, checked with `cargo search` on 2026-10-17)

| Area | Crates |
|---|---|
| UI | gtk4 **0.11** (feature `v4_22`), libadwaita **0.9** (`v1_9`), vte4 **0.10** (`v0_84`), sourceview5 **0.11** (`v5_18`), glib/gio **0.22** (`v2_88`), cairo-rs **0.22**, glib-build-tools (gresources in build.rs) |
| Async & system | tokio **1.53** (rt-multi-thread, net, io-util, time, signal, sync, fs, process), nix **0.31** (term, ioctl, fs, pty, signal), libc, udev **0.9** |
| MCP | rmcp **3.5** (`server`, `macros`, `schemars`, `transport-io`, `transport-streamable-http-server`). It supports protocol **2026-07-28** (LATEST, stateless) and is backward-compatible to 2024-11-05, so the MCP plan from the C version carries over without a hand-written JSON-RPC layer. Also axum **0.8**, schemars **1.2** |
| Data & config | serde + serde_json, toml **1.1**, regex **1.13** (automation, expect), base64, crc **3.4**, jiff **0.2** (timestamps) |
| App plumbing | clap **4.6** (+ clap_mangen, clap_complete), tracing **0.1** + tracing-subscriber **0.3**, thiserror **2**, anyhow (bin only), async-channel **2.5**, gettext-rs **0.8** |
| Testing | proptest **1.12**, insta **1.49**, rstest, tempfile, image (PNG compare), cargo-fuzz **0.13** |

**System packages** (one-time; needs your sudo):

`sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libgtksourceview-5-dev libudev-dev desktop-file-utils appstream gettext`

lrzsz, xvfb-run, socat, gdb, valgrind and the nightly toolchain are already installed. Missing cargo tools: `cargo install --locked cargo-nextest cargo-llvm-cov cargo-deny cargo-machete`. cargo-fuzz, cargo-audit and grcov are already present.

### 1.3 Runtime model
- **The tokio runtime** runs on its own thread pool, started by the app. It owns all I/O: `Session` actors, the MCP server, and the udev monitor.
- **The GTK main thread** only renders. The GUI talks to sessions through `SessionHandle` (an mpsc command channel). It receives `SessionEvent`s through an `async_channel`, consumed in `glib::spawn_future_local`. Nothing GTK-related is ever touched off the main thread.
- **`SessionManager`** (an `Arc`) is the single owner of sessions. Both the GUI and the MCP server are clients of it, so an agent and a human share live sessions and the GUI shows the agent's actions.
- **`Session`** (one actor per connection) owns:
  - the `Box<dyn Port>`
  - `ChunkStore`: a ring capped by bytes and by count, with a **monotonic byte/chunk cursor** (no index drift, which avoids C defect S1)
  - the transform state
  - `SunshineStore`
  - the decoder pipeline
  - the log writer
  - the transfer/script runner slot (exclusive RX ownership while a transfer is active)
- **Events** are broadcast through `tokio::sync::broadcast`, with lag handling: consumers re-sync via the cursor.
- **Port trait** (async):

```rust
#[async_trait]
trait Port: Send {
    async fn open(&mut self) -> Result<()>;
    async fn close(&mut self);
    async fn write_all(&mut self, data: &[u8]) -> Result<()>; // partial writes handled
    fn reader(&mut self) -> PortReader;                       // stream of Bytes | Disconnected
    fn kind(&self) -> PortKind;
    fn describe(&self) -> String;
    fn serial(&mut self) -> Option<&mut dyn SerialControl>;
}
```

  `SerialControl` covers set_dtr/rts, break(ms), modem_lines(), wait_modem_change() (TIOCMIWAIT with a poll fallback), icounts() (TIOCGICOUNT), rs485 config and flush.
- **Serial backend** (`core/src/port/serial/`): nix + `tokio::io::unix::AsyncFd`, Linux only, so we keep full control.
  - `open(O_RDWR|O_NOCTTY|O_NONBLOCK|O_CLOEXEC)`
  - `flock(LOCK_EX|LOCK_NB)` plus an optional UUCP lock in `/run/lock`
  - save the original termios, then `cfmakeraw` + explicit CSIZE/PARENB/PARODD/CMSPAR/CSTOPB/CRTSCTS/IXON/IXOFF/CLOCAL
  - **termios2 + BOTHER** for any baud rate
  - kernel RS485 via `TIOCSRS485`, with delays. If that's unsupported, fall back to TIOCMBIS/TIOCMBIC around tcdrain, run in `spawn_blocking`, never toggling.
  - DTR/RTS via TIOCMBIS/TIOCMBIC; break via `tcsendbreak` or TIOCSBRK/TIOCCBRK for a duration
  - restore the termios on close
  - all ioctl wrappers live in `sys.rs` with `// SAFETY:` notes
- **TCP/UDP:** tokio net.
  - Async connect with timeout and DNS lookup, IPv4 and IPv6.
  - TCP server: per-client tasks; the merged RX is tagged by peer; broadcast TX drops dead clients.
  - UDP: configurable remote, or learn it from the first sender; configurable max datagram size.
- **Hotplug:** a udev `MonitorBuilder` on AsyncFd sends `PortsChanged` events. Optional auto-reconnect when the configured device reappears.
- **Unix signals:** tokio `signal(SIGUSR1/SIGUSR2)` opens/closes the active session, as before (compatible).

### 1.4 GUI structure (gtkterm-gui)
Subclassed widgets with composite templates (`data/ui/*.ui` in gresources). Plain gtk4-rs, no relm4, so the idioms stay close to the GNOME docs.

**`Application`** (`adw::Application`, app-id `at.philipp.GtkTermSunshine`, placeholder):
- actions, accelerators
- startup: tokio runtime + SessionManager
- optional embedded MCP

**`MainWindow`** (`AdwApplicationWindow` + `AdwToolbarView`):
- **Top bar:** `AdwHeaderBar` with port dropdown (live udev list + /dev/pts + saved TCP/UDP profiles), baud dropdown (with custom entry), connect toggle, view-mode dropdown (all 7 modes), primary menu.
- **Content:** a `GtkPaned`.
  - Left: `TerminalView` (vte4::Terminal + search bar using `vte4::Regex` PCRE2, with case, regex and hex options).
  - Right: `VisualizationPanel` (`AdwTabView` + `AdwTabBar`, one tab per packet ID, closable and reorderable).
- **Bottom bar:** `StatusBar` widget (profile button, endpoint string `port baud-bits-parity-stop`, state, RX/TX counters, TX queue, DTR/RTS buttons, CTS/DSR/DCD/RI LEDs, logging and MCP indicators).
- **Messages:** `AdwToastOverlay` for transient ones, `AdwBanner` for the error/disconnected state.

**TerminalView:**
- Formats RX through `protocols::view::Formatter` (ASCII/HEX/BIN/DEC/MIXED/BIN_MIXED/DEC_MIXED, index, hex width 8/10/16/24/32, timestamps), then `feed()`s VTE.
- Keyboard input goes to `Session::send` (with local echo).
- Clipboard copy/paste, and right-click popover (created once).

**ChartView** (one per tab): renders `gtkterm-charts` into `snapshot.append_cairo`, using theme colours from `widget.color()` and `AdwStyleManager`, plus Pango text.
- Controllers: scroll to zoom, drag to pan, motion for a tooltip of the nearest point, click a legend entry to toggle that series.
- Tab toolbar: Pause, Follow, history length, chart type (Line/Bar/Pie/Scatter), Export CSV, Export PNG, Clear (replaces Renew), Close.

**Dialogs:**
- `AdwPreferencesDialog` with pages: Connection (serial/TCP/UDP and every port field), Terminal (font, colours, rows/cols, scrollback, cursor, bell, echo, CR/LF, timestamps, delay/wait-char), Visualization, Logging, MCP (enable, port, token, policy, allowed dirs), Scripting (allow exec).
- Also `AdwAlertDialog`, `GtkFileDialog`, `AdwAboutDialog` (GPL-3.0-or-later) and `AdwShortcutsDialog`, the last generated from the single accelerator table.

**State:** window size, maximised, pane position and panel visibility are kept in `state.toml`. No GSettings, so it runs uninstalled without a schema.

### 1.5 Shortcut map
It never steals plain Ctrl+letter (those go to the device) and never uses Super+Space.

| Shortcut | Action |
|---|---|
| Ctrl+Shift+C / Ctrl+Shift+V | copy / paste |
| Ctrl+Shift+A | select all |
| Ctrl+Shift+F | search |
| Ctrl+Shift+L | clear screen |
| Ctrl+Shift+K | clear scrollback |
| Ctrl+Shift+B | send break |
| Ctrl+Shift+G | visualization panel |
| Ctrl+Shift+P | preferences / port config |
| Ctrl+Shift+R | send file |
| Ctrl+Shift+S | save raw |
| Ctrl+Shift+H | hex send |
| Ctrl+Shift+Q | quit |
| F5 / F6 | open / close port |
| F7 / F8 | DTR / RTS |
| F1 | about |
| Ctrl+? | shortcuts dialog |

A unit test fails on duplicate accelerators.

---

## 2. MCP server (crate gtkterm-mcp)

Built on rmcp 3.5 with the `#[tool_router]`/`#[tool]` macros and schemars 1.x input/output schemas. Results carry `structuredContent` plus text.

**Capabilities:** tools, resources (with subscribe and list_changed), prompts, logging, completions, progress, cancellation, pagination, and tasks for long calls where the client negotiates them.

**Protocol versions:** 2026-07-28 by default. rmcp negotiates down to 2025-11-25 and older.

**Run modes:**
- **Embedded in the GUI:** Preferences → MCP. Streamable HTTP on 127.0.0.1/::1 via `axum` with the rmcp `StreamableHttpService`.
  - Middleware checks `Authorization: Bearer <token>` (the token lives in `$XDG_RUNTIME_DIR/gtkterm-sunshine/mcp-token`, mode 0600) and validates `Origin`.
  - A status-bar indicator and an "Agent activity" pane show what the agent does.
- **Headless:** `gtkterm-sunshine --mcp-stdio [--profile X]`. GTK is never initialised, so it works without a display. Register with `claude mcp add gtkterm -- gtkterm-sunshine --mcp-stdio`.
- **Headless HTTP:** `--mcp-http [addr:]port`.

**Tools:**

| Group | Tools |
|---|---|
| Ports | `list_ports`, `open_port`, `close_port`, `configure_port`, `get_port_status` |
| I/O | `send` (text/hex/base64, line ending, per-char delay); `read` (since_cursor, max_bytes, timeout; returns `next_cursor`); `expect` (optionally send, then regex/bytes/timeout; returns match + captures + preceding data); `send_break`; `set_signals`; `get_signals`; `clear_buffer`; `search_buffer` |
| Sunshine | `sunshine_list_series`, `sunshine_get_series` (time or count window, LTTB downsampling), `sunshine_set_chart_type`, `sunshine_export_csv`, `sunshine_export_png` (headless cairo), `sunshine_clear`, `sunshine_pause` |
| Logging | `log_start`, `log_stop`, `log_status` |

These arrive in later milestones (each with its backend): `decoder_set`, `decoder_get_frames`, `modbus_request`, `file_send`, `file_receive`, `macro_list`, `macro_run`, `script_run`, `script_stop`, `profile_*`, `preset_*`, `signal_history`.

**Resources:**
- `gtkterm://ports` ⟳
- `gtkterm://session/{id}/status` ⟳
- `…/rx{?since,encoding}` ⟳ (updates throttled to ≤10 Hz)
- `…/sunshine` ⟳ and `…/sunshine/{packet_id}` ⟳
- `…/frames` ⟳
- `…/log`
- `gtkterm://profiles`
- `gtkterm://docs/sunshine-protocol`

**Prompts:** `debug-serial-device`, `analyze-sunshine-data`, `modbus-scan`, `at-modem-check`, `write-sunshine-firmware-snippet`.

**Policy:**
- Off by default.
- Levels:
  - `read-only`: write tools are hidden.
  - `interactive`: the GUI default; the first write per session needs confirmation through an `AdwAlertDialog` (Allow once / Always / Deny).
  - `full`: the headless default.
- `exec` in scripts is off unless `--mcp-allow-exec` is passed and the setting is enabled.
- File tools are restricted to allowed directories, after canonicalisation with symlink-escape checks.
- Size, timeout and rate caps.
- JSONL audit log.

---

## 3. Compatibility contract

Kept from the C version (the full spec was extracted by the agent and is summarised here).

- **Sunshine packets:** `{ID|Name|R,G,B|v [v…]|Name|v…}`.
  - Dot decimal separator.
  - A colour field must be exactly 3 integers 0–255.
  - Multiple space-separated values per series: 1 value means Y; ≥2 values are (X,Y) pairs.
  - Lines end at `\r`, `\n` or `\r\n`.
  - **Extension:** an optional chart-type prefix on the ID, `L:`/`B:`/`P:`/`S:`. Without a prefix you get a line chart (same as today).
  - **Stricter than C:** reject NaN/inf/overflow; ≥1 value per series; non-empty ID; nothing after `}`; valid UTF-8 and name length ≤64; reject `1,5`-style fields where a value is expected.
- **Config:**
  - **New file:** `~/.config/gtkterm-sunshine/config.toml`, with `[profiles.<name>]` tables holding the same field names as the old keys (port, baudrate, bits, stopbits, parity, flow_control, term_* …) and the same defaults, except rows and columns get their correct defaults (25×80).
  - **One-time import** from `~/.config/.gtktermrc` (GKeyFile; sections become profiles; booleans are `true`/`false`). If the file is in the upstream parsecfg format, it is skipped with a hint.
  - Removing `default` resets it to the defaults, as before.
- **CLI:** same long options (`--port`, `--baudrate`, `--bits`, `--stopbits`, `--parity`, `--flow_control`, `--rs485_rts_time_before_tx/after_tx`, `--disable_port_lock`, `--term_wait_delay`, `--term_wait_char`, `--term_echo`, `--term_raw_filename`, `--term_rows`, `--term_columns`, `--term_auto_cr`, `--term_auto_lf`, `--use_config`, `--show_config`, `--save_config`, `--list_config`, `--remove_config`).
  - Kebab-case aliases are accepted.
  - The same short letters, **except** `-h` stays help, so auto-LF becomes `-j` (documented).
  - `on|off` booleans, plus `true|false`.
  - The `--show_config` output format is kept.
  - New options: `--tcp host:port`, `--tcp-listen port`, `--udp [local:]lport->host:rport`, `--profile`, `--mcp-stdio`, `--mcp-http`, `--mcp-read-only`, `--mcp-allow-exec`, `--log-file/--log-format`, `-v/--verbose`.
- **Baud list:** 300 … 2000000, including 500000 (it was missing from C's validation), plus a free custom value via BOTHER.
- **Behaviour that was stored but never implemented in C** is now actually implemented:
  - wait delay per line / wait char
  - mark/space parity
  - 1.5 stop bits: rejected with an error, since Linux termios has no 1.5
- **View-mode text formats:** byte-exact to the C table (index `%08X: `, HEX `%02X `, the mixed-mode padding, and so on), except the counters are per terminal and not global. Covered by insta snapshot tests.
- **Timestamp prefix:**
  - The C format `[%d.%02uh.%02um.%02us.%03u]` (days since the epoch, UTC) is replaced by a selectable format: `HH:MM:SS.mmm` local (default), relative `+S.mmm`, or ISO-8601.
  - The first line now gets a timestamp too.
- **Macros:**
  - Import `~/.config/gtkterm/macros.json` (`{"macros":[{name,shortcut,commands:[{type 0|1|2,data,delay}]}]}`), decoding `g_strescape` octal escapes.
  - The new store `~/.config/gtkterm-sunshine/macros.json` uses the same schema but standard JSON escapes.
  - The editor line syntax `send: …` / `wait: N ms` is kept, and `expect: <regex>` is added.
  - Shortcuts actually bind; recording actually records.
- **Presets:**
  - The 10 built-in presets are kept as listed: Arduino, ESP32/ESP8266, GPS, HC-05, Modbus RTU 8E1@19200, Modem RTS/CTS, Raspberry Pi, STM32, Legacy 7E1@2400, High Speed 921600 RTS/CTS.
  - The JSON schema is kept for import/export; `stop_bits:"1.5"` is mapped to an error.
- **Logging:**
  - Formats: raw / text (`[HH:MM:SS.mmm] R|T: …<%02X>`) / CSV (`Timestamp,Direction,Length,Data_Hex,Data_ASCII`) / JSON (`{"session":{…},"data":[…]}`, now with a valid footer) / XML (`<gtkterm_log version="1.0"><packets>…`).
  - Session filename `YYYYMMDD_HHMMSS_<port>_<profile>.<ext>`, sanitised.
  - Menu semantics kept: Start, Pause/Resume, Stop, Clear.
- **Automation scripts:**
  - The same language: `send` (escapes `\n \r \t \\ \xHH`), `expect <regex>` (5 s default), `delay`, `set_dtr`, `set_rts`, `break [ms]`, `log`, `exec`, `${var}`, `${MATCH_n}`, `${EXEC_OUTPUT}`.
  - Additions: indented `#` comments, `timeout <ms>`, `if_match`/`goto` labels (implements C's CONDITIONAL stub), `include`.
  - Regex dialect: Rust `regex` (documented differences from GRegex).
  - The shipped `data/examples/*.script` files must run unchanged (test).
- **TCP/UDP endpoint strings:** `TCP:host:port`, `TCP-Server:port`, `UDP:local:lport->remote:rport`.
- **XMODEM:**
  - Checksum, CRC (CCITT 0x1021 init 0, high byte first) and 1K variants.
  - SOH/STX framing, block numbers from 1, padding with 0x1A.
  - 10 retries, 10 s timeout.
  - Cancel with CAN CAN.
  - Adds YMODEM batch (block-0 header) and YMODEM-G. ZMODEM goes through the lrzsz `sz`/`rz` bridge.
- **Decoders:** the same 7 (AT, NMEA, Modbus RTU/ASCII, Modbus TCP, CAN, EtherCAT, RS232 signals) and the same output style (`[MODBUS-RTU] Slave:%02X Func:%02X (name) … [CRC:OK]`), with the defects fixed. SLCAN is added.
- **Signal monitor:** 300 s history; CSV `Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals`.
- **Charts:** the same four types and defaults (1000 points for line/scatter, 100 for bar), legend, title and ±10% padding, now theme-aware. The Welcome page becomes an `AdwStatusPage` empty state.

---

## 4. Milestones

Each milestone lands on the `rust-rewrite` branch with CI green. **Rule:** tests come first; every §8 defect is guarded by a test that fails on the C behaviour.

### R0 — Scaffold & tooling (S)
- Tag `legacy-c-v1.99.1` on master. Branch `rust-rewrite` from `roadmap/phase0-foundation`.
- On that branch: `git rm -r src include convert po meson.build meson_post_install.py doc .circleci`. Keep `data/` assets and `docs/`.
- Create the workspace and crate skeletons.
- `[workspace.lints]`: `unsafe_code = "deny"` (overridden only in `core::port::serial::sys`), `clippy::pedantic` warn with a curated allow-list, `missing_docs` on public APIs.
- Add `rustfmt.toml`, `deny.toml` (licences compatible with GPL-3.0-or-later; advisories) and `.config/nextest.toml`.
- **CI:** `.forgejo/workflows/ci.yml` on the local forgejo-runner. Jobs: fmt, clippy `-D warnings`, nextest (unit + integration), `xvfb-run` GUI tests, `cargo deny check`, `cargo audit`, `cargo machete`, llvm-cov report artifact, nightly fuzz smoke (60 s per target).
- `xtask check` runs the same steps locally.
- Update `CLAUDE.md` for the Rust layout and commands.

### R1 — Protocols & core I/O (L)
- **`protocols`:**
  - Transform (CR/LF, timestamps, wait-char/delay planning).
  - View formatter (all 7 modes).
  - Hex-send parser (`DE AD`, `0xDE`, `\xDE`, `de:ad`; errors with a position).
  - CRC library helpers.
- **`core`:**
  - Port trait and the Serial/TCP client/TCP server/UDP/Loopback ports.
  - ChunkStore with cursor.
  - Session actor and SessionManager.
  - udev enumeration and hotplug.
  - Config (TOML, legacy keyfile importer, profiles, validation with typed errors).
  - CLI (clap) with the full compatible option set and the config ops.
  - Unix signals.
- **Headless smoke:** `gtkterm-sunshine --headless --port /tmp/ttyA` prints RX to stdout. This is also a handy debug tool.

### R2 — Sunshine core & charts (M)
- **`protocols::sunshine`:** parser and a streaming scanner with a partial-line carry-over, capped at 4 KiB.
- **`core::sunshine::SunshineStore`:**
  - Per-ID chart type (prefix or user override, persisted per ID).
  - Ring-buffer series with real timestamps.
  - Stats (packets, parse errors, drops).
  - Pause.
  - CSV export.
- **`gtkterm-charts`:**
  - Scales with relative epsilon, nice ticks, and `%g`-style labels.
  - Palette by index.
  - Line/bar (from a zero baseline)/pie/scatter renderers, legend layout.
  - LTTB.
  - PNG export through a cairo `ImageSurface`.

### R3 — MCP server (M–L)
- The §2 tools for Ports, I/O, Sunshine and Logging, plus resources, prompts, policy and audit.
- stdio and HTTP transports.
- A `--mcp-dump-schema` flag generates `docs/mcp.md`.

### R4 — GUI MVP → release `2.0.0-alpha.1` (L)
- Everything in §1.4.
- Logging UI.
- Hex-send popover with history.
- Send raw file / save raw.
- Preferences for every field.
- Profiles popover (load/save/delete actually work).
- Dark mode via `AdwStyleManager`.
- Embedded MCP toggle.
- i18n scaffolding (gettext-rs, `po/`).
- `xtask install --prefix ~/.local`: binary, desktop file, metainfo, icons, man page and completions. No conflict with upstream gtkterm paths.

### R5 — Macros, presets, profiles UX (M)
- Macro editor (`AdwDialog` + GtkSourceView 5 via `sourceview5` **0.11**, feature `v5_18`). Recording, shortcut binding, import of the old macros.json.
- Presets: built-ins plus user presets, and "Save as preset".
- MCP: `macro_*`, `profile_*`, `preset_*`.

### R6 — Decoders & data view (M–L)
- Frame assemblers: line-based (AT, NMEA), RTU with 3.5-character gap or CRC sync, ASCII `:`…CRLF, MBAP length, candump/SLCAN lines, EtherCAT length.
- The 7 decoders plus SLCAN, with `detect()` scoring for Auto.
- **Data view pane:** a `GtkColumnView` with time, direction, hex/ASCII, decoded summary and CRC ✓/✗, plus an expander for the fields.
- MCP: `decoder_set`, `decoder_get_frames`, `modbus_request`.

### R7 — File transfer (M)
- XMODEM / CRC / 1K, YMODEM and YMODEM-G as sans-IO state machines, driven by the session with exclusive RX.
- ZMODEM through an lrzsz bridge (spawn `sz`/`rz` with their stdin/stdout pumped to and from the port). The option is hidden if lrzsz is missing.
- Progress shown in the status bar; cancel.
- MCP: `file_send`, `file_receive` with progress notifications.

### R8 — Automation, port control, signal monitor (M)
- **Script runner:** an iterative interpreter with a cancellation token. `exec` uses argv-based `tokio::process` and is never fed device data unquoted; it stays opt-in. Script editor with the shipped `.lang` highlighting.
- **Port control:** advanced serial settings in Preferences (mark/space, VMIN/VTIME, flush, break duration, RS485 flags) and an experimental auto-baud (tries the rate list and scores printable ratio plus TIOCGICOUNT framing errors).
- **Signal monitor pane:** a 300 s timeline of DTR/RTS/CTS/DSR/DCD/RI from TIOCMIWAIT events, with CSV export.
- MCP: `script_run`, `script_stop`, `signal_history`.

### R9 — Release `2.0.0` (S–M)
- Feature-parity checklist signed off.
- Hardware checklist done.
- Docs: README, `docs/mcp.md`, `docs/debugging.md`, `docs/testing/hardware.md`, man page.
- Translations template.
- Optional Flatpak manifest (`org.gnome.Platform` 49+), with serial access via `--device=all`.
- Merge `rust-rewrite` into master. The C code stays on the tag.

---

## 5. Test strategy

| Layer | Tooling | Content |
|---|---|---|
| Unit (`protocols`, `charts`) | `cargo nextest`, rstest tables, **proptest**, **insta** snapshots | formatter output per mode (snapshot); transform CR/LF matrix; hex parser; Sunshine parser vectors + property "split anywhere = same packets"; decoder reference vectors (Modbus `01 03 00 00 00 0A C5 CD`, `:010300000001FB`, NMEA GGA/RMC, candump/SLCAN); XMODEM/YMODEM sender↔receiver in-memory with injected loss/corruption, CRC test vector `"123456789"→0x31C3`; chart scale/ticks/LTTB properties; log formatter snapshots (JSON/XML parse-validated) |
| Integration (`core`) | `#[tokio::test(start_paused = true)]` for timeouts; **`nix::pty::openpty`** pairs; tokio TCP/UDP on loopback v4/v6 | serial open/close/reopen ×100, flock conflict, termios readback (CSIZE/PARENB/CSTOPB/CRTSCTS/IXON), 1 MB byte-exactness, slow-reader partial writes, EOF → Disconnected; TCP client, TCP server with 3 clients and 1 dropping, connect timeout to 192.0.2.1; UDP learn-remote; full pipeline port → session → ChunkStore/Sunshine/logging; 20k chunks keep updating; config import golden files (old `.gtktermrc`, old `macros.json`); CLI matrix (`assert_cmd`); example scripts run against fake devices |
| Interop | lrzsz `sx/rx/sb/rb/sz/rz` over a socat or openpty pair (skipped if missing) | send/receive 0, 1, 127, 128, 129, 33 KB and 1 MB files, and files full of 0x04, 0x18 and 0x1A; then `cmp` |
| MCP | rmcp **client** feature in Rust integration tests; insta snapshot of `tools/list` + schemas; `npx @modelcontextprotocol/inspector@latest --cli` in CI | every tool's happy path and schema errors (−32602); version negotiation (2026-07-28 and 2025-11-25); cancellation of `expect`; progress during transfers; resource subscribe → updated; policy levels; path traversal; HTTP 401 on a bad token, 403 on a bad Origin; stdio via `tokio::process` on the real binary |
| GUI | `xvfb-run -a`, `GSK_RENDERER=cairo`, `G_DEBUG=fatal-criticals`; a test binary using `gtk::init` on the main thread | build the window against a loopback session, inject packets, assert tab count and chart types, activate every action, open and close every dialog, keyboard → port bytes, duplicate-accelerator check, `.ui` validation (`gtk4-builder-tool validate`), chart PNG pixel-diff with tolerance |
| Fuzz | cargo-fuzz (nightly) | sunshine scanner, hex parser, each decoder + assembler, XMODEM/YMODEM receiver, config/macros/presets importers, script parser, MCP JSON-RPC input via rmcp |
| Packaging | `desktop-file-validate`, `appstreamcli validate`, `xtask install --destdir` | file list contains no upstream gtkterm paths |

**Coverage goal** (llvm-cov): ≥85% for `protocols` and `charts`, ≥70% for `core` and `mcp`.

**Manual hardware checklist** (`docs/testing/hardware.md`):
- USB-UART loopback (TX↔RX, RTS↔CTS, DTR↔DSR)
- 921600 and custom 250000 baud
- RS485 adapter direction (checked with a logic analyser)
- unplugging while streaming, then auto-reconnect
- ZMODEM to a real target
- a live Claude Code session driving the device through MCP

## 6. Debugging toolkit (`docs/debugging.md`)

- **Logging:** `tracing` everywhere with spans per session and port.
  - `RUST_LOG=gtkterm_core=debug,gtkterm_mcp=trace`.
  - `--verbose` and `--log-file` (JSON lines).
  - Headless mode logs to stderr; stdout is reserved for MCP stdio.
- **Async:** `tokio-console` via the `console` cargo feature (`console-subscriber`) to inspect stuck tasks.
- **GTK:** `GTK_DEBUG=interactive` (Inspector), `G_DEBUG=fatal-criticals` under `rust-gdb`.
- **Debuggers:** `rust-gdb` / `rust-lldb` (installed) with `RUST_BACKTRACE=1`, and `panic = "abort"` off in dev.
- **Memory and UB:**
  - Miri for the pure crates (`cargo +nightly miri test -p gtkterm-protocols`).
  - ASan for core: `RUSTFLAGS=-Zsanitizer=address cargo +nightly test -p gtkterm-core --target x86_64-unknown-linux-gnu`.
  - valgrind only for FFI-heavy GUI paths, with the GLib suppressions.
- **Performance:** criterion benches (formatter, scanner, store, chart render at 10k points), a `cargo xtask bench-pipeline` that streams 50 MB through a PTY, `samply` / `perf` + `cargo flamegraph`, and GTK frame timing via the Inspector.
- **Serial:**
  - `socat -d -d -x pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB` (hex trace)
  - `strace -e ioctl,read,write -p PID`
  - `gtkterm-sunshine --headless --hexdump`
- **Data generator:** `sunshine-gen` (testkit bin). Rate, IDs with chart prefixes, split/garbage/burst/NaN modes, plus fake AT/Modbus/NMEA device modes. Targets a PTY, TCP or UDP endpoint.
- **MCP:** `npx @modelcontextprotocol/inspector@latest` (interactive UI), `--mcp-http` + curl recipes, the JSONL audit log.
- **Fuzz crash triage:** `cargo fuzz tmin` → turn the minimised input into a regression test under `tests/regressions/`.

## 7. Verification (end-to-end, per release)

1. `cargo xtask check` (fmt, clippy, nextest, deny, audit, machete) and `xvfb-run cargo nextest run -p gtkterm-gui` all pass; the llvm-cov thresholds are met.
2. Run `socat … /tmp/ttyA /tmp/ttyB &`, then `cargo run -p gtkterm-gui -- --port /tmp/ttyA --baudrate 115200`, then `cargo run -p gtkterm-testkit --bin sunshine-gen -- /tmp/ttyB --rate 1000 --split --ids P0,B:P1,P:P2,S:P3`. Expect:
   - 4 tabs with the right chart types, still live after more than 20k chunks
   - pause, zoom, tooltip and CSV/PNG export work
   - the log file has valid JSON
3. TCP: `ncat -lk 5000` with `--tcp 127.0.0.1:5000`, packets both ways. UDP: the same with `ncat -u`.
4. MCP:
   - `claude mcp add gtkterm -- ./target/debug/gtkterm-sunshine --mcp-stdio`, then in Claude Code: "open /tmp/ttyA at 115200, wait for a Sunshine packet and summarise the Temperature series".
   - Repeat against the GUI over HTTP and watch the activity pane.
   - Run the Inspector CLI `tools/list`.
5. Later milestones: XMODEM/YMODEM against `rx`/`sb` with `cmp`; the example scripts against the fake modem; decoders against `sunshine-gen --device modbus`.
6. `cargo xtask install --destdir $SCRATCH/stage`, then `desktop-file-validate` and `appstreamcli validate`; the installed paths don't collide with the gtkterm package.

## 8. C defects → Rust acceptance tests

Each one is a named regression test (`tests/regressions/c_*.rs` or a unit test).

| ID | C defect (file:line in legacy-c) | Rust guard |
|---|---|---|
| B1 | Repage `memmove` in the wrong direction, plus an unbounded `\n` scan (`gtkterm_buffer.c:487`) | 64 KB with no `\n` and 1 MB random binary through the ChunkStore: no panic, exact bytes |
| B2 | Heap overflow on memcpy after repage (`buffer.c:366,430`) | capacity properties (proptest) |
| B3 | Auto-CR/LF never set `cr_received` (`buffer.c:377`) | CR/LF transform matrix |
| B4 | Buffer dispose `g_free`s GObjects (`buffer.c:167`) | ownership-safe by construction; drop-order test of the session |
| S1 | Sunshine stops after 10k chunks (`manager.c:316`) | 20k-chunk live test |
| S2 | Split packets dropped (`scanner.c:136`) | split-at-every-offset property |
| S3 | Re-scan on every panel reopen (`window.c:2454`) | toggle the panel 3×: same point counts |
| S4 | `1,5` read as a colour, which shifts the fields; NaN/inf accepted | parser vectors |
| S5 | Only line charts ever created (`manager.c:249`) | prefix/override routing test |
| S6 | Axis drift, flat thresholds, everything blue | scale/palette unit tests |
| X1 | XMODEM first block lost on the `C`/NAK handshake (`xmodem.c:363`) | in-memory loopback + lrzsz interop |
| X2 | EOT/CAN detected inside payload (`xmodem.c:525`) | files full of 0x04 and 0x18 |
| X3 | CTRL-Z stripped from every block (`xmodem.c:494`) | file full of 0x1A |
| X4 | Block-255 wrap-around duplicate NAK'd forever (`xmodem.c:477`) | file larger than 32 KB |
| P1 | RS485 RTS toggled, not set; `usleep` on the UI thread (`serial_port.c:874`) | `SerialControl` API test on a mock backend; hardware checklist |
| P2 | DTR/RTS menu toggles did nothing (`serial_port.c:1234`) | mock backend asserts TIOCMBIS/BIC calls |
| P3 | Custom baud via `ASYNC_SPD_CUST` kept stale `c_cflag` (`serial_port.c:267`) | termios readback after a reconfigure from 7E2 RTS to 8N1 none |
| P4 | Partial writes dropped (`serial_port.c:882`) | slow-reader PTY test |
| P5 | Close reported failure on success (`window.c:1464`) | GUI action test |
| T1 | TCP server only read the newest client; connect blocked the UI | 3-client test, connect timeout |
| U1 | UDP hostname → NULL address | `localhost` resolves |
| C1 | CLI config signals corrupted the stack (`gtkterm.c:198`) | `assert_cmd` config-ops matrix |
| C2 | Config bugs: show_index → block_cursor key, inverted port lock, swapped rows/cols | config round-trip property |
| M1 | Macros: missing NULL sentinel, use-after-free on edit, invalid JSON escapes | macro store round-trip with `äöü\x00\x1b` |
| L1 | Logging never wrote data; double free on export; no JSON/XML footer | log pipeline + format validation |
| A1 | Automation `exec` with device-controlled `${MATCH_n}`, run synchronously | policy test: `exec` refused by default; argv-only spawn |
| K1 | Ctrl+Shift+V bound 4×; Ctrl+a/q/F stole control bytes | duplicate-accelerator test; plain Ctrl+letter reaches the port |
| D1 | Decoders: OOB `strchr`/`strstr`, Modbus ASCII `len-7`, NMEA degrees, CAN offset | decoder vectors + fuzz |
| I1 | App-id, desktop `Exec`, icon and config collided with upstream | packaging path test |
