# Feature parity with the C GTKTerm (tag `legacy-c-v1.99.1`)

Every user-facing feature of the C version, compared with 2.0. In the inventory below the C
status is one of:
- **wired**: worked end to end.
- **partial**: reachable but broken.
- **stub**: a "not yet implemented" message or nothing.
- **dead**: compiled but never reachable from the UI.

"Test" names the automated guard; *hardware* means `docs/testing/hardware.md`.

Result: every wired, partial and stub feature is implemented. Every dead module is wired in, with
the exceptions in the last section.

## Menus and actions

| C feature (status) | 2.0 | Test |
|---|---|---|
| Clear screen / scrollback (wired) | menu, Ctrl+Shift+L / Ctrl+Shift+K | GUI test activates every action |
| Send raw file (stub) | *Send File…* → *Raw*, Ctrl+Shift+R | `tests/transfer.rs`, GUI test |
| Save raw file (stub) | *Save Received Data…*, Ctrl+Shift+S | GUI test |
| Quit (wired; accelerator Ctrl+Q took a control character) | Ctrl+Shift+Q | `shortcuts::no_duplicates_and_no_plain_ctrl_letters` |
| Copy / Paste / Select all (wired; Select all was bound to Ctrl+Shift+V) | Ctrl+Shift+C / V / A, context menu | same (K1) |
| Find with case, regex and hex options (wired; Ctrl+F stole ^F) | Ctrl+Shift+F, same options | GUI test |
| Log to file / Resume / Stop / Clear (partial: nothing was written) | Start, Pause/Resume, Stop, Clear; raw/text/CSV/JSON/XML | `session_log_records_rx_and_tx`, logfmt snapshots (L1) |
| Prefer dark theme (wired, not saved) | follows the desktop (libadwaita); charts and editors adapt | — |
| Port configuration dialog (wired) | Preferences → Connection, every field including RS485 timing, lock and break | GUI test |
| Main window configuration (stub) | Preferences → Terminal: font, colours, scrollback, cursor, bell; the window size is remembered (`term_rows`/`term_columns` are kept in the config for compatibility but not applied) | GUI test |
| Local echo, CR LF auto, timestamps (partial: buffer bug) | Local echo, Auto CR, Auto LF, timestamps in three formats | transform matrix (B3), `headless_terminal_on_pty` |
| Macros dialog (wired; recording and shortcuts were stubs) | editor with recording, shortcuts and a macro bar | `macros` tests, GUI test |
| Load / save / delete configuration (stub) | profile popover in the status bar | GUI test, `cli` config-ops tests (C1) |
| Send break (wired) | Ctrl+Shift+B, configurable length | `send_default_break` tests |
| Open / close port F5 / F6 (wired) | F5 toggles and F6 closes; SIGUSR1 / SIGUSR2 | GUI test (SIGUSR2), headless |
| Toggle DTR / RTS F7 / F8 (partial: wrong bit mask) | F7 / F8, status-bar buttons | loopback modem lines (script DTR reset test), GUI test |
| View ASCII / hex / binary / decimal / three mixed modes (wired) | all seven, header dropdown and menu kept in sync | formatter snapshots |
| Hex chars 8/10/16/24/32 (stub: no parameter) | *Hex Width* menu, Preferences | GUI test |
| Show index (partial: could not be switched off) | toggle | formatter tests |
| Show timestamps / direction (stub: data view was NULL) | terminal timestamps; frames pane shows time and direction | frames tests |
| Send hexadecimal data (stub) | Ctrl+Shift+H with history | hex parser tests, GUI test |
| Visualization panel (wired; line charts only) | Ctrl+Shift+G; line, bar, pie, scatter; zoom, pan, export | charts tests, GUI test (S1–S6) |
| About (wired) | About dialog with troubleshooting information | GUI test |

## Header bar, status bar, terminal

| C feature (status) | 2.0 | Test |
|---|---|---|
| Port dropdown, scanned once (wired) | live udev list plus rescan, `/dev/pts`, "(not found)" marker | `ports` tests |
| Baud dropdown, 17 rates (wired) | the same 17 rates (500000 now also passes validation); any rate in Preferences or `-s` (termios2) | termios readback (P3) |
| Connect button and state label (wired) | Connect toggle, banner with *Reconnect* on errors | GUI test (P5) |
| Infobar for every message (wired) | toasts and a banner | — |
| Status: profile, port string, state, DTR/RTS buttons, CTS/CD/DSR/RI LEDs (wired) | same, plus RX/TX counters, TX queue, log, MCP, macro and transfer indicators | GUI test |
| Window title "GtkTerm - port" (wired) | "GTKTerm Sunshine — [profile] …" | — |
| VTE terminal, keyboard to the port (wired) | same; plain Ctrl+letter reaches the device | `shortcuts` test (K1) |
| Font / colours / cursor / scrollback / rows / cols from config (wired, no UI) | Preferences, applied live | GUI test |
| Visual bell (buggy: set the audible bell) | the terminal flashes instead of beeping | GUI test |
| Wait delay / wait char (stub) | per-line delay and wait character | session pacing tests |
| Window size, maximized, fullscreen remembered (wired, GSettings) | `state.toml`; F11 fullscreen; Ctrl+plus/minus/0 zoom | `state` tests |

## Command line and configuration

| C feature (status) | 2.0 | Test |
|---|---|---|
| All `--port`/`--baudrate`/… options with short letters (mostly wired; `-g`, `-h` broken) | same names and letters; auto-LF is `-j`; kebab-case aliases | `cli` tests, `assert_cmd` matrix (C1) |
| `--show_config`, `--save_config`, `--remove_config`, `--use_config`, `--list_config` (wired) | same, same output format | `cli` tests |
| `.gtktermrc` keyfile, all keys (wired with bugs C2) | imported once into `config.toml`, with C2 fixed | `imports_c_fork_keyfile`, `imports_upstream_1x_keyfile` |
| `gtkterm_conv` for upstream 1.x files (wired, separate binary) | automatic import of `~/.gtktermrc`, including `shortcut::action` macros | `imports_upstream_1x_parsecfg_macros` |
| Port lock with flock (partial: inverted) | flock; `--disable_port_lock` | flock conflict test |
| udev: close when the device disappears (wired) | same; the error banner offers *Reconnect* | hardware #7 |
| SIGUSR1 opens, SIGUSR2 closes (wired) | same, in the GUI and headless | GUI test, headless |
| Man page (outdated) | generated from the options, plus FILES, SIGNALS, EXAMPLES | `xtask install` |
| Translations fr, hu, ru, de, zh_CN, nl_NL (partial: most strings unmarked) | gettext catalogue covering all GUI strings; German complete | `xtask check` (template current, `msgfmt --check`) |

## Modules that were dead in C

| C module | 2.0 | Test |
|---|---|---|
| TCP client/server, UDP ports | `--tcp`, `--tcp-listen`, `--udp`, Preferences | TCP 3-client, UDP learn-remote (T1, U1) |
| Port control (mark/space, flush, break, auto-baud, signals) | Preferences and menu; auto-baud is experimental | `auto_baud` PTY test, hardware #15 |
| Presets (10 built-ins, JSON) | Preferences → *Apply preset*, *Save as preset*, import | `presets` tests |
| Signal line monitor with CSV | Ctrl+Shift+M, 300 s, pulse recovery | `signals` tests, hardware #16 |
| Toolbar | header bar and menu (same actions) | — |
| Shortcut manager and window | single accelerator table, shortcuts dialog (Ctrl+?) | `shortcuts` test |
| File transfer: XMODEM/CRC/1K, YMODEM, YMODEM-G, ZMODEM | all; ZMODEM via lrzsz | lrzsz interop (X1–X4) |
| Automation scripts | script runner and editor, `--script`, MCP `script_run` | `tests/script.rs` (A1) |
| Protocol decoders (AT, Modbus RTU/TCP, NMEA, CAN, EtherCAT, RS232) | all, plus Modbus ASCII, SLCAN and auto-detect | decoder vectors and fuzz targets (D1) |
| Logging formats and session file names | all five formats, `YYYYMMDD_HHMMSS_port_profile.ext` | logfmt snapshots (L1) |
| Data view (timestamps, direction, filter) | frames pane (Ctrl+Shift+D) with filter and details | frames tests |
| Rich status bar (counters, rates, LEDs, protocol) | status bar (counters, LEDs, indicators) | GUI test |
| GtkSourceView language specs | macro and script editors with highlighting | GUI test |

## Not carried over

- The data view's *delta* timestamp mode (dead in C). Terminal timestamps are local time,
  relative to the connection, or ISO 8601.
- The shortcut table's tab actions (new/close/next tab). The C version had no tabs. Run a
  second window instead (the application is not single-instance).
- The C translations. Their messages describe the old dialogs. German is translated anew;
  the template `po/gtkterm-sunshine.pot` is ready for other languages.
- `G_APPLICATION_HANDLES_OPEN` without a handler. File arguments were never supported.
