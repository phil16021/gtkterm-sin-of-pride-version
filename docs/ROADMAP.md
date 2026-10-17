# GTKTerm "Sin of Pride" — Audit, Implementation, Test & Debugging Plan

## Context

A full audit of the fork (21k LOC C/GTK4) turned up three main problems:

1. **Memory-safety bugs on the main data path** (Serial → Buffer → Terminal/Sunshine) that the device itself can trigger.
2. **About half the code is compiled but never instantiated:** TCP/UDP ports, 7 protocol decoders, XMODEM, automation, toolbar, statusbar, shortcuts, presets, port-control, signal-monitor, dataview. The logging and macro-recording entry points are also never called.
3. **No real test suite.** The only `test()` is desktop-file-validate. CI is a stale GTK3 CircleCI job. A fresh clone can't even configure, because `doc/meson.build` is git-ignored.

Decisions taken:
- **Goal:** a robust tool for daily use.
- **Unused modules:** wire them all in (fix each one, then integrate it).
- **Identity:** rename to its own app-id so it installs next to upstream gtkterm.
- **Repo hygiene:** delete build artifacts, move the analysis docs to `docs/`.
- **Dependencies:** install via apt.
- **Libraries:** use the newest versions everywhere (Ubuntu 26.04 baseline, see 0.8). Add libadwaita for modern UI widgets.
- **MCP:** implement a full MCP server (Phase 7) so AI agents can drive serial/TCP/UDP devices and read the Sunshine data.

Rule for every phase: **write a failing regression test first, then fix.** Each phase ends with `meson test` green under ASan/UBSan.

Key evidence: findings were checked directly in the code, and cppcheck `--library=gtk` was run (results in the scratchpad).

---

## Phase 0 — Foundation: build, hygiene, identity, test & CI scaffold (S–M)

### 0.1 Environment
- `sudo apt install libglib2.0-dev libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libgudev-1.0-dev libjson-glib-dev libgtksourceview-5-dev libsoup-3.0-dev libpcre2-dev meson ninja-build desktop-file-utils afl++ lrzsz` (gcovr is already in `~/.local/bin`).
- Already present: valgrind, cppcheck, clang-tidy, gdb, lcov, socat, xvfb-run.
- Build with meson/ninja only. The STM32 `cmake` on PATH doesn't matter here.

### 0.2 Fix build breakage
- `.gitignore:8` ignores `doc`, so `doc/meson.build` is untracked, yet `meson.build:58` does `subdir('doc')`. Remove the ignore line, commit the file, and add `doc/html` to the ignores instead.
- Add the missing `dependency('libpcre2-8')` (used at `gtkterm_window.c:39`).
- Add `_GNU_SOURCE` for `strcasestr` (`gtkterm_rs232_decoder.c:174`) and `#include <unistd.h>` in window.c.
- Fix the build warnings: the unused statics `on_gtkterm_toggle_radio*` (`window.c:1840,2476`), and the printf `%u`/int mismatches (`gtkterm_buffer.c:469`, `can_decoder.c:376`, `modbus_tcp_decoder.c:279`).
- Build hardening:
  - Add `warning_level=2` to the meson `project()` default_options.
  - Add a `-Dwerror` CI job.
  - Add meson options `-Dtests=true`, `-Dfuzz=false` and `-Dconverter=true`.

### 0.3 Repo hygiene
- `git rm` these tracked files: `test_memory`, `test_performance`, `gmon.out`, and `code_analyse_kodritsch.pdf` (it is markdown, not a PDF). Delete the untracked `data_parser.o` and `fuzz_harness.o`.
- Move `test_memory.c`, `test_performance.c` and `performance_analysis.txt` to `docs/analysis/demo/`. They are course demos and don't touch GTKTerm.
- Move the 11 analysis/architecture `.md` files to `docs/analysis/` and add a `docs/README.md` index. Keep `README.md`, `CLAUDE.md` and `COPYING` at the root.
- Mark stale claims in those docs. They advertise TCP/UDP, decoders, logging and XModem as working, and FIXES_APPLIED claims "keine Warnings".

### 0.4 Own identity (coexists with upstream)
- **App-id:** `at.philipp.GtkTermSunshine` (placeholder; confirm the final ID). It appears in:
  - `gtkterm.c:313`
  - the gresource prefix `/com/github/jeija/gtkterm` (gresource.xml, every `gtk_builder`/`texture_new_from_resource` path, `window.c:1897`)
  - the schema id and filename (`data/com.github.jeija.gtkterm.gschema.xml`, plus `gtkterm.c:156` and `window.c:2573,2589`)
  - the icon name (`gtkterm_main.ui:7`, plus the installed icon filenames in `data/meson.build`)
  - the desktop file (`Exec=gtkterm-sin-of-pride-version`, add a trailing `;` to Categories)
  - the gettext domain
  - the man page (rename it to `gtkterm-sin-of-pride-version.1` and install it once, not twice: `meson.build:60` and `data/meson.build:63`)
- **Config file:** `g_get_user_config_dir()/gtkterm-sunshine/gtktermrc`, replacing `~/.config/.gtktermrc` (`gtkterm_defaults.h:62`). Add a one-time import from the old file *only if* it parses as a GKeyFile. Upstream 1.x uses parsecfg format; `convert/` covers that case.
- Rename `gtkterm_conv` to `gtkterm-sunshine-conv` and point its output at the new path.
- **Running from the build dir without installing:**
  - Compile the schema into the build dir with `gnome.compile_schemas()`, plus a `devenv` / `meson devenv` that sets `GSETTINGS_SCHEMA_DIR`.
  - Look up the schema with `g_settings_schema_source_lookup` and fall back to defaults, so the app never aborts when the schema is missing.
- Fix `meson_post_install.py`: `gtk-update-icon-cache` needs a directory argument.
- Update `RELEASE_DATE`, the version, and `po/POTFILES.in`, which lists 10 files that don't exist.

### 0.5 Logging hygiene (needed for debugging)
- Replace every `g_print("DEBUG…")` with `g_debug()` under `G_LOG_DOMAIN "gtkterm"` (set per target with `-DG_LOG_DOMAIN`). The hot ones are `terminal.c:258`, `manager.c:186,312,323`, `window.c:2451` and the many in serial_port.c.
- Turn it on with `G_MESSAGES_DEBUG=gtkterm`.

### 0.6 Test scaffold
- Add a `tests/meson.build` with suites `unit`, `integration`, `gui` and `validate`. Use GLib `g_test` with TAP output (`protocol: 'tap'`).
- Build a `gtkterm_core` **static library** from all non-window sources. Both the executable and the tests link against it. This is the most important testability seam.
- Validation tests:
  - `desktop-file-validate` (already exists, `data/meson.build:22`)
  - `glib-compile-schemas --strict --dry-run`
  - `gtk4-builder-tool validate` for each `data/*.ui`
  - `appstream-util validate-relax` if metainfo gets added
- Add test setups:
  - `meson test --setup=asan`: `ASAN_OPTIONS=detect_leaks=1`, an `LSAN_OPTIONS` suppression file for GLib/fontconfig, `G_DEBUG=fatal-criticals,gc-friendly`, `G_SLICE=always-malloc`.
  - `meson test --setup=valgrind`: wrapper `valgrind --error-exitcode=1 --leak-check=full --suppressions=/usr/share/glib-2.0/valgrind/glib.supp --suppressions=tests/gtk.supp`.

### 0.7 CI (Forgejo, local runner)
- The repo remote is `forgejo:philipp/…` and `~/bin/forgejo-runner` is already running, so use Forgejo Actions instead of CircleCI.
- Add `.forgejo/workflows/ci.yml` with these jobs:
  - **build-gcc:** debug, `-Dwerror=true`
  - **build-clang:** asan+ubsan, `meson test --setup=asan`
  - **valgrind:** the unit suite
  - **static:** `cppcheck --library=gtk` and `run-clang-tidy -p build` with a `.clang-tidy` checklist (clang-analyzer-*, bugprone-*, cert-*; minus GObject-noise rules like reserved-identifier); fail only on new findings
  - **coverage:** `-Db_coverage=true`, `gcovr --html`
  - **gui:** `xvfb-run` + `GDK_BACKEND=x11` smoke test
- Delete `.circleci/`.

### 0.8 Dependency baseline: newest libraries everywhere
Versions are the current Ubuntu 26.04 candidates, checked with `apt-cache policy`. Raise every `dependency()` minimum in `meson.build` to:

| Library | Min version | New APIs adopted |
|---|---|---|
| glib-2.0 / gio | **2.88** | `g_autoptr`/`g_autofree` throughout, `GTask`, `GSubprocess`, `g_signal_connect_object`, `g_clear_handle_id`, `GFile` async I/O |
| gtk4 | **4.22** | `GtkFileDialog`, `GtkAlertDialog`, list models, `gtk_widget_get_color` for chart theming |
| libadwaita-1 | **1.9** (new dependency) | `AdwApplication`, `AdwApplicationWindow`/`AdwToolbarView`, `AdwHeaderBar`, `AdwAlertDialog`, `AdwBanner` (replaces GtkInfoBar), `AdwToastOverlay` (status messages), `AdwPreferencesDialog` (port/terminal/MCP settings), `AdwAboutDialog`, `AdwShortcutsDialog` (replaces the deprecated GtkShortcutsWindow), `AdwTabView`/`AdwTabBar` for Sunshine chart tabs (close/reorder for free), `AdwStyleManager` for dark mode |
| vte-2.91-gtk4 | **0.84** | |
| gtksourceview-5 | **5.18**, now **required** | |
| json-glib-1.0 | **1.10** | |
| gudev-1.0 | **238** | |
| libsoup-3.0 | **3.6** (new dependency, for MCP HTTP) | |
| libpcre2-8 | **10.46** | |
| meson | **≥ 1.10** | |

Build settings:
- **C standard:** `c_std=gnu23` (GCC 15 default). Use `nullptr`, `bool`, `[[nodiscard]]` where it helps.
- **Pin the API surface** so deprecated or too-new API shows up as an error in `-Dwerror` CI:
  - `-DGLIB_VERSION_MIN_REQUIRED=GLIB_VERSION_2_88 -DGLIB_VERSION_MAX_ALLOWED=GLIB_VERSION_2_88`
  - `-DGDK_VERSION_MIN_REQUIRED=GDK_VERSION_4_22 -DGDK_VERSION_MAX_ALLOWED=GDK_VERSION_4_22`
  - `-DADW_VERSION_MIN_REQUIRED=ADW_VERSION_1_9`
- **Tooling, also latest:**
  - Python helpers run as uv scripts with inline `# /// script` metadata and unpinned or latest deps (`mcp`, `pyserial`).
  - Node tools via `npx @modelcontextprotocol/inspector@latest`.
  - AFL++ from apt.
- **Trade-off:** this builds only on distros ≥ Ubuntu 26.04 / Fedora 43-era. That's acceptable for a daily-use fork; say so in the README.

**DoD:** a fresh clone configures and builds warning-free, `meson test --suite validate` is green, and CI runs on push.

---

## Phase 1 — Critical memory safety & correctness (M–L)

Each item gets a regression test in `tests/unit/` before the fix.

### 1.1 Buffer rewrite (`gtkterm_buffer.c`): the root cause of most crashes
Bugs:
- `repage()` at lines 487-491 runs `memmove` in the **wrong direction**. Its `\n` scan has no bound, so it reads past the heap buffer on binary data.
- The `memcpy(buffer+tail, …)` at lines 366/430 is unchecked after a repage, which is a heap overflow.
- The emit length `tail-old_tail` (lines 424-442) can underflow.
- Auto-CR/LF never works because `cr_received` is never set (lines 377-408).

Fix: split the logic into a **pure transform** and a **storage layer**:
- `gtkterm_buffer_transform(const guint8*, gsize, GtkTermTransformState*, GString *out)` handles CR/LF/timestamp, takes an injectable clock, and is unit-testable.
- Storage becomes a bounded `GByteArray` ring (head/tail, size cap) that trims to a line boundary *with* bounds. The emitted data comes from the local `GString`, never from a slice of the ring.

Chunk store:
- Replace the `GPtrArray` with `remove_index(0)` (O(n), and the 10k cap breaks Sunshine) with a `GQueue` or ring.
- Add a monotonic `total_chunks_added` counter and a new signal `chunk-added(GtkTermDataChunk*)`.
- Cap the store by bytes as well as by count.

Dispose (lines 163-169):
- Replace the `g_free()` calls on GObjects with `g_clear_object`.
- Stop freeing the `term_conf` the buffer doesn't own; the terminal frees it too, so today it is a double free.
- Disconnect the handlers and chain up.

GBytes ownership:
- The emitter owns the GBytes and unrefs it after `g_signal_emit`. Remove the handler-side unref at line 436 and the leaking early returns at 338/358.
- Fix the leak in `terminal.c:173`: the echo GBytes is never unref'd.

Tests:
- 64 KB with no `\n`, 1 MB of random binary, `\r\n`/`\r`/`\n` matrices × auto-CR/LF.
- Timestamps across chunk boundaries.
- 20k chunks, then confirm the counter keeps advancing.

### 1.2 Object lifetimes
- **Terminal dispose** (`terminal.c:362-369`): use g_clear_pointer/g_clear_object, close and unref the port, unref `term_buffer` (leaked at line 325), and chain up to VteTerminal. Also unparent the right-click popover, which today accumulates (lines 520-521).
- **Serial port:**
  - Move `g_signal_new("serial-data-received")` from instance `constructed` (lines 559, 576-577) to `class_init`. The same applies to the `serial-data-transmit` signal currently registered in the terminal's class_init (`terminal.c:440`).
  - Store the `g_unix_signal_add` and udev source IDs and remove them in dispose.
  - The async read callback (lines 923-952) must hold `g_object_ref(self)`, free its errors, and treat a 0-byte read as a disconnect.
- **Window dispose** (`window.c:2789`): unref `logging`, `macro_manager` and `action_group`. Don't re-create a popover on every profile click (line 2492).
- **Logging export** (`logging.c:576-579`): double close/unref of the stream. Set `priv->stream = NULL` before unreffing `temp_log`.
- **App quit** (`gtkterm.c:87-89`): option groups are double-unref'd, and `section` is freed without being cleared. Remove those lines and use `g_application_quit`.

### 1.3 Command line & configuration
- **Signals** (`gtkterm.c:198,209,233`): `config_save`, `config_list` and `config_remove` are declared with 0 params and a POINTER return, but are emitted with args and an `&int`. That corrupts the stack. Replace the signal indirection with **direct function calls** and check `get_error()` for NULL.
- **Short options:** add `-g`/`-h` to `GtkTermCLIShortOption` (`config.c:116`), but rename `-h`, because GLib reserves it for help. Fix the swapped help texts (`cmdline.c:240-241`).
- **Config bugs:**
  - `:762` writes show_index into the BLOCK_CURSOR key.
  - `:923` inverts disable_port_lock.
  - `:766/812` save view_mode as a string but read it as an integer.
  - `:961-962` swap the rows/cols defaults.
  - `:964` passes boolean `false` as a string.
  - `:537` has an operator-precedence bug.
  - `:1197` double-frees the GOption error.
  - `:438/482/496/638` call `g_propagate_error` onto an already-set error.
  - `:544-572` and `:1079` leak `g_key_file_get_string` results (found by cppcheck).
  - 500000 baud is missing from the validation list (`:1017`).
- **Header-bar dropdowns** are fixed at 115200 / index 0 (`window.c:2322,2339`) and overwrite `-s`/`-p` and the saved config on connect. Initialise them *from* the config, and make F5 use the same source.
- Tests:
  - Round-trip every config key: save → load → compare.
  - The cmdline matrix through `g_application_run` with `argv` in a test, using a temp `XDG_CONFIG_HOME`.

### 1.4 Macros
- **`macros.c:61-68`:** add a NULL sentinel (`g_malloc0((n/2+1)*…)`), guard against a NULL `string_list`, and reject an odd count.
- **`gtkterm_macro.c`:**
  - Add a real `finalize`. Today `gtkterm_macro_free` frees fields while the list store still holds references.
  - Use `g_object_unref` as the hash value-destroy function.
  - Duplicate the shortcut keys.
  - Re-key the hash on rename.
  - Generate unique names ("New Macro N", `window.c:951`).
  - Use `JsonBuilder`/`JsonGenerator`, replacing the hand-built JSON plus `g_strescape`, which produces invalid JSON for non-ASCII text (lines 423-433).
  - Guard a missing `"data"` member (lines 549-553).
- **Macro editor:** remove the remove+insert on save (`window.c:1112`); update the item in place and call `g_list_model_items_changed`. The Close button needs a response.
- Tests: JSON round-trip with `"äöü\x00\x1b"` payloads; rename/delete/re-add sequences under ASan.

**DoD:** ASan- and valgrind-clean on the unit suite. The "open → stream 10 MB of random data → close → reopen → quit" integration test is clean.

---

## Phase 2 — Serial port correctness (M)

All in `gtkterm_serial_port.c`.

**termios setup** (lines 267-295):
- Start from `cfmakeraw`, set CSIZE/PARENB/PARODD/CSTOPB/CRTSCTS/IXON explicitly, and set CLOCAL as configured.
- Use `cfsetispeed`/`cfsetospeed` for standard rates, including the B230400…B4000000 constants.
- Use `termios2` + `BOTHER` (`TCSETS2`) for arbitrary rates. Remove the `ASYNC_SPD_CUST` path and its unchecked `TIOCGSERIAL`.
- Add mark/space parity (`CMSPAR`), which PortControl already models.
- Replace `g_assert_not_reached` on bad config (line 295) with a GError.
- Check every `tcgetattr`/`tcsetattr` return value.

**Open/close:**
- Make `open()` idempotent (lines 352-363).
- Create the cancellable only after the fd opens successfully.
- On lock failure, save errno first and skip the termios restore when no backup exists (lines 383-395, 774-777).
- Add an optional UUCP lockfile in `/run/lock`.
- `close()` returns 0/-1 as documented; `window.c:1464` currently shows "Failed to close" on every successful close.

**Control lines:**
- **RS485:** use kernel `TIOCSRS485` (`struct serial_rs485`, delays). If that's unsupported, fall back to `TIOCMBIS`/`TIOCMBIC` around `tcdrain`, not a toggle (lines 874-905, 1081-1084). Remove the `usleep` on the main loop.
- **DTR/RTS menu actions:** use `TIOCMBIS`/`TIOCMBIC` directly (lines 1234-1249).
- **Signal poll timer:** clear its ID when the callback returns `G_SOURCE_REMOVE` (lines 1137-1140). On a PTY, report "modem lines unsupported" once instead of dying silently.

**Writes:** use `g_output_stream_write_all_async` with a TX queue (line 882) so CTS-low no longer blocks the UI. Show the TX queue depth in the statusbar.

**Hotplug:** use the existing gudev client to rescan the port list on add/remove, and auto-reconnect when the port reappears (optional toggle).

**Tests** (integration, using in-process `openpty()`; no socat needed in CI):
- open, close, reopen ×100
- the flock conflict between two instances
- RX/TX byte-exactness at 1 MB
- partial-write simulation (small PTY buffer and a slow reader)
- EOF when the slave closes, giving a clean disconnect state
- a termios readback after `configure` (`tcgetattr` on the slave, checking CSIZE/PARENB/CSTOPB/flow-control flags)

PTYs don't support `TIOCMGET`/`TIOCSRS485`/`BOTHER`, so those are covered by a **manual hardware checklist** (`docs/testing/hardware.md`): a USB-UART loopback (TX↔RX, RTS↔CTS, DTR↔DSR), 921600 and 250000 baud, RS485 adapter direction, and checking the line signals with a logic analyser.

---

## Phase 3 — Port abstraction: Serial + TCP + UDP behind `GtkTermPort` (L)

`src/gtkterm_port.h` already defines the interface (vfuncs plus `data_received`/`state_changed`/`error_occurred` signals). It is currently implemented only by TCP/UDP, and `read` is a no-op there.

**Refactor:**
1. `GtkTermSerialPort` implements `GtkTermPort` via `G_IMPLEMENT_INTERFACE`. Its legacy `serial-data-received`/`notify::port-status` signals become adapters, and are then removed.
2. Remove `read` from the interface; it is push-based through `data_received`. Fix the length type, which is emitted as `gsize` but declared `G_TYPE_UINT64` (`gtkterm_port.c:47`).
3. **`GtkTermBuffer` and `GtkTermTerminal` take a `GtkTermPort*`** instead of `GtkTermSerialPort*` (`buffer.c:108`, `terminal.c:322`). Serial-only features (signals, break, RS485) go behind `GTKTERM_IS_SERIAL_PORT` or a small optional `GtkTermSerialControl` interface.
4. Add a **`GtkTermLoopbackPort`** (test-only, in `tests/`) for unit and GUI tests with no OS resources.
5. **Add a `GtkTermSession` object** (new `src/gtkterm_session.c`). It is a GTK-free core owning the port, buffer, Sunshine store, decoder pipeline, logging and the transfer engine. It is the single place where a port, buffer, logging, decoders and Sunshine are wired together.
   - `GtkTermTerminal` and the window become **views** of a session. The buffer must no longer reference `GtkTermTerminal`; the VTE echo goes through `gtkterm_session_send()`.
   - This is required for both the headless MCP mode (Phase 7) and headless tests.
   - The app holds a `GListStore` of sessions. For now the window shows one; later this allows multiple ports as `AdwTabView` tabs.

**TCP fixes** (`gtkterm_tcp_port.c`):
- Use async connect with a timeout and a cancellable; the current connect is synchronous on the UI thread (line 178).
- In server mode, keep one read source per client (line 129). A client disconnecting must not close the server (lines 73-80).
- Broadcast writes need a separate GError per client, `write_all`, and removal of dead clients (line 326).
- Fix the address leak and the `const` return (lines 461-464).
- `get_stats` must ref the `GDateTime`.
- Add optional TCP keepalive and auto-reconnect.

**UDP fixes** (`gtkterm_udp_port.c`):
- Resolve hostnames with `GResolver`; today `g_inet_address_new_from_string` returns NULL for them (lines 134, 154).
- Support IPv6 and a configurable maximum datagram size (currently 4 KB with silent truncation).
- Fix the `connected_time` leak.

**UI & config:**
- Add a "Connection type" dropdown (Serial / TCP client / TCP server / UDP) to the port config dialog and the header bar.
- New config keys: `port_type`, `host`, `tcp_port`, `udp_local_port`, `udp_remote`.
- Cmdline: `--tcp host:port`, `--tcp-listen port`, `--udp local:remote`.
- The window title and statusbar show the endpoint.

**Tests:**
- TCP client against an in-process `GSocketService` echo server.
- TCP server with 3 concurrent clients, one of which disconnects.
- UDP send/receive on 127.0.0.1 and ::1.
- Connect timeout to an unroutable address (TEST-NET 192.0.2.1).
- Every test runs the same "buffer receives exactly the bytes" assertion through each port type.

---

## Phase 4 — Sunshine visualization: fixes & features (M–L)

### 4.1 Critical/high fixes
- **Live updates die after 10,000 chunks** (`manager.c:316` vs `buffer.c:325-328`). Switch the manager to the Phase 1 `chunk-added` signal and drop `last_scanned_chunk_index`.
- **Packets split across reads are lost** (`scanner.c:136-140`). Keep the partial line between feeds and cap it at 4 KiB. Add an explicit `gtkterm_sunshine_scanner_feed(const guint8*, gsize)`, which also gives a testable seam.
- **Re-opening the panel adds every packet again** (`window.c:2454`). Scan history only when the manager is created.
- **Parser hardening** (`data_parser.c`):
  - Treat a field as a colour only if it is exactly 3 integers in 0–255. This fixes `1,5`-style comma decimals shifting every later field.
  - Reject `!isfinite`/ERANGE values.
  - Require ≥1 value per series and a non-empty ID.
  - Reject anything after `}` and any nested `{`/`}`.
  - Run `g_utf8_make_valid` on names and cap them at 64 chars.
  - Make `parse_color` check the end pointer.
  - Use the chunk `timestamp_us`, not the parse time (line 111).
- **Dispose:** fix the line chart's double free on repeated dispose (`line_chart.c:53-54`; use `g_clear_pointer`). The manager must ref `notebook` and `buffer`, and the `charts` hash table needs a value-destroy function (`manager.c:86,99-100`).
- **Shortcut:** `Ctrl+Shift+V` is bound to paste, select-all, **send-break** and visualization (`menu.ui:49,54,146,253`). The new shortcut map is in 6.2.

### 4.2 Chart fixes
- **Range computation:** move it out of snapshot into pure helpers (`sunshine/chart_util.c`: `compute_range()`, `nice_ticks()`, `palette_color(i)`, `hsv_to_rgb()`), unit-tested.
  - Fixes the axis drift after Renew or with an empty first series (`line_chart.c:288-335`).
  - Uses a relative epsilon instead of the 0.01/0.001 thresholds (line:327, scatter:303, bar:289).
  - Uses `%g` labels and nice-number ticks.
- **Default palette:** cycle series colours by index. Today the fallback colour is ignored and every series is blue (`line_chart.c:240,447`).
- **Bar chart:** bars grow from y_min instead of a zero baseline (`bar_chart.c:407`). Also fix the `rgb(hue,…)` that isn't HSV, draw the categories and a legend, and replace `g_strdupv` with a bounded copy.
- **Pie chart:** align the legend and slice colour indices, draw labels without relying on `\n`, show no negative percentages, and use a responsive legend.
- **Rendering:** switch text to Pango (`pango_cairo_*`). Take colours from the widget's CSS / `gtk_widget_get_color`, so the charts follow dark mode (`window.c:2411`). Make margins scale with the widget size. Replace `remove_index(0)` per point with a ring buffer.
- **Model/view split:** move the data (series ring buffers, packet stats, chart-type mapping) into a GTK-free `GtkTermSunshineStore` owned by the session. Charts become views that observe the store through an `items-changed`/`series-updated` signal. The MCP server (Phase 7) reads the same store headless. The chart tabs move from GtkNotebook to `AdwTabView`.
- **Unified API:** add a `GtkTermSunshineChart` GInterface (`handle_packet`, `clear`, `export_csv`, `snapshot_to_png`, `set_paused`). It replaces the type-check chains in `manager.c:222-234,335-346`.

### 4.3 Missing features (this is what "wire in everything" means for charts)
- **Chart type per packet ID.** Today it is hard-coded to the line chart (`manager.c:249`). Add an optional type prefix in the protocol, `{L:P0|…}` / `B:` / `P:` / `S:`, while plain IDs still give a line chart. Add a per-tab "Chart type" menu that is remembered per ID in the GKeyFile.
- **Per-tab toolbar,** extending the existing Renew-button pattern:
  - Pause/Resume
  - Export CSV (series, timestamp, value; uses `gtkterm_data_chunk_get_timestamp_string`)
  - Export PNG (render `draw_chart` into a `cairo_image_surface`)
  - Close tab
  - History length spin (it replaces the compile-time `MAX_DATA_POINTS`)
- **Real time axis.** Choose between sample index and relative time from `timestamp_us` (`gtkterm_buffer_get_session_start_time`). Today X is a shared sample counter that interleaves the series of a multi-series packet.
- **Interaction:**
  - Click a legend entry to toggle that series (the `visible` field already exists).
  - Hover tooltip showing the nearest point.
  - Scroll to zoom and drag to pan on the X axis, with a "Follow" toggle.
- **Panel:** "Clear all" wired to `manager_clear_all`. Packets / parse-errors / dropped counters in the panel footer (the stats already exist). The pane position and visibility are saved in GSettings, replacing the hard-coded 700 px (`window.c:2442`).

### 4.4 Tests
- **Parser table tests:** valid packets from CLAUDE.md, colours, missing colours, `1,5`, `nan`/`inf`/`1e400`/`0x1p3`, `{}`, `{|}`, `{a|b}`, `{a|x|1}{b|y|2}`, invalid UTF-8, a 64 KiB line, a 200-series packet.
- **Scanner feed tests:** every packet split at every byte offset (an exhaustive loop) must parse identically to the unsplit packet.
- **Manager integration:** loopback port → buffer → manager. 20k chunks keep updating. Each ID maps to one tab of the right type.
- **Chart_util unit tests:** ranges with min==max, all-negative values, a single point, NaN rejection; nice-ticks golden values.
- **Render smoke test** (gui suite, xvfb): snapshot each chart type to PNG and check non-empty pixels plus no criticals. This is an optional golden-image diff with tolerance.
- **Fuzzing:** rebuild the lost AFL harness. `fuzz_harness.o` symbols show `fgets` → `parser_new` → `parse_line` → `packet_free`.
  - New `tests/fuzz/fuzz_sunshine_parser.c` as a libFuzzer/AFL++ dual entry point, built with `-Dfuzz=true` and clang `-fsanitize=fuzzer,address`.
  - Seed corpus from the README packets.
  - Same pattern for `fuzz_scanner_feed`.

---

## Phase 5 — Wire in the dormant modules (L–XL)

Order follows dependencies; each item is "fix the audit findings → integrate → test".

**5.1 Logging** (`gtkterm_logging.c`):
- Connect `write_chunk` to the buffer's `chunk-added` signal, for RX and TX. Today "Log → To file" creates empty files.
- Write the JSON/XML footer on `stop()`.
- Sanitise `profile_name`/`port_name` for the filesystem: no `/`, no `..`, and escape them in JSON (lines 303-335).
- Use JsonGenerator for the JSON format. Check write and flush errors and report them in the statusbar.
- **Tests:** each format (raw/text/hex/JSON/XML) is round-trip parsed and validated (JSON via json-glib, XML via `xmllint` in a test script). Run the export path under ASan.

**5.2 Statusbar & toolbar:**
- Instantiate `GtkTermStatusbar` as the window's bottom bar: port endpoint, state, RX/TX counters, line signals, TX queue, logging indicator. Rebuild it on reconfigure; today it only updates on `notify::port-status`.
- Make `GtkTermToolbar` an optional bar (View → Toolbar, in GSettings). Remove the header-bar controls that duplicate it.
- Fix the toolbar's dead-code bugs: the double unref of the protocol model (`toolbar.c:248,399`) and the icon re-parent (`:355,366`).

**5.3 Shortcuts & actions:**
- Replace `gtkterm_shortcuts.c`'s table with a single source of truth tied to the real `gtkterm_window.*` action names. Today it binds plain Ctrl+C/V/A, which steals terminal control characters, and refers to non-existent `win.copy`/`win.send-break`.
- Add the actions that are missing: `send-file`, `receive-file`, `run-script`.
- Add a keyboard-shortcuts dialog built with `AdwShortcutsDialog` (libadwaita 1.8+), generated from the same shortcut table.

**5.4 Hex send** (stub at `window.c:1821`):
- Pure function `gtkterm_hex_parse("DE AD be ef 0x01 \\x02", GByteArray*, GError**)` in a new `src/gtkterm_hex.c`, unit-tested.
- Hex send dialog/entry with history (upstream 1.3 had hex history; commit 22d98fe "Push'n'Pop hex data history" is in this repo's history and can be reused).
- Also implement the other "not yet implemented" stubs: `save_raw` (dump the buffer), `send_raw` file (`open_response_cb`, `window.c:419-427`, which reads the file and sends nothing), and the load/save/remove config dialogs (`window.c:519,814,1273-1301`).

**5.5 File transfer & XMODEM** (`gtkterm_xmodem.c`, `gtkterm_file_transfer.c`):
- Fix the first block: the `'C'` handshake sends nothing, and the NAK handshake sends block 0 by seeking −128 (lines 363-379, 407-429). Keep the last block in a buffer for retransmits.
- Receiver: check for EOT/CAN only at a block boundary, and require a double CAN (lines 525-535).
- Fix the sender's EOT loop and resend on timeout (lines 196, 404).
- Strip CTRL-Z only on the final block, and make it optional (lines 494-500).
- Fix the `(guint8)` wrap-around comparison (line 477) and the leaks of `file`/`start_time`.
- Add the CRC→checksum fallback and the NULL `write_func` guard (line 140).
- Add **XMODEM-1K** and **YMODEM** (batch, block-0 header with name and size) on the same engine.
- **ZMODEM:** implement it by spawning `sz`/`rz` (lrzsz) bridged to the port fd/stream. That's far less risky than a native implementation. Hide the option when lrzsz isn't installed.
- **YMODEM-G:** remove it from the enum, or implement it as a flag on YMODEM.
- **UI:** Send/Receive File dialogs (`GtkFileDialog`), a progress bar with cancel in the statusbar, and pausing the terminal and Sunshine RX while a transfer is active.
- **Tests:**
  - CRC16-XMODEM test vector: "123456789" → 0x31C3.
  - Back-to-back sender↔receiver loopback (inject `write_func`).
  - Files of 0, 1, 127, 128, 129 and 33 KB bytes.
  - Files full of 0x04, 0x18 and 0x1A.
  - Wrap past block 255.
  - Injected loss and corruption: drop or corrupt every Nth byte, then check retries and the final byte equality.
  - **Interop test:** against lrzsz `sx`/`rx` over a socat PTY pair (integration suite, skipped if lrzsz is missing).

**5.6 Protocol decoders** (`gtkterm_protocol_decoder.c` + 7 decoders):
- **Framing layer:** a new `GtkTermFrameAssembler` per protocol (line-based for AT/NMEA, inter-frame gap 3.5 chars or CRC-sync for Modbus RTU, length-prefixed for Modbus TCP). Today every decoder assumes one chunk is one frame.
- **Before any string op:** copy to a NUL-terminated buffer, or use length-bounded `memmem`/`g_strstr_len`. This fixes OOB reads in NMEA detect and decode (`nmea:167,347`), CAN (`:101,334`), AT detect, and RS232 detect.
- **Decoder fixes:**
  - Modbus ASCII: `length-9` (`:196`), lowercase hex (`:107`), the CR/LF logic (`:143,317`).
  - NMEA coordinates: `deg=floor(v/100)` (`:120-123`); add `$P…` and `!AIVDM` passthrough.
  - CAN: the `123#DEAD` format, standard vs extended frames by ID width, `struct can_frame` data at offset 8, add **SLCAN**.
  - AT: prefix table order (`:83`).
  - RS232: the double `decode_binary_signals` (`:205`).
  - Modbus TCP: MBAP max 254.
  - EtherCAT: decode all datagrams.
- **UI:** a "Protocol" selector (Auto / None / AT / Modbus RTU / ASCII / TCP / NMEA / CAN / EtherCAT / RS232) in the toolbar and View menu. A **decoder side pane** (or the dataview, 5.7) lists the decoded frames with time, direction, summary, CRC ✓/✗, and expandable fields. Auto-detect uses the decoders' `detect` scores over a sliding window.
- **Tests:** reference vectors per decoder in a table.
  - Modbus RTU `01 03 00 00 00 0A C5 CD`
  - Modbus ASCII `:010300000001FB\r\n`
  - NMEA GGA/RMC with known checksums
  - candump/SLCAN lines
  - Short, truncated and split frames at every offset
  - A per-decoder fuzz target

**5.7 Dataview** (`gtkterm_dataview.c`, currently `window->dataview = NULL`, `window.c:256`):
- Instantiate it as an alternate view (View → Data view) for timestamps, direction and hex/ASCII. This makes `show_timestamps` and `show_direction` work; today they silently do nothing.
- Fix the hex offset underflow `byte_offset + i - length` in format_hex.
- Hook in the decoder pane from 5.6.

**5.8 Port control & signal monitor:**
- **Port control** (`gtkterm_port_control.c`): it opens a **second fd** on the device (line 754). Turn it into a dialog/model that operates *through* the active `GtkTermSerialPort` API.
  - Map its config onto `port_config_t`; remove the duplicate `GtkTermPortConfig` that presets also use.
  - An unknown baud rate must be an error, not a silent B9600 (line 86).
  - Remove the fake `detect_baud_rate` (lines 438-490), or implement it for real: try the rates in order while scoring printable ratio and framing errors via `TIOCGICOUNT`. **Recommendation:** implement it, behind an "experimental" label.
- **Signal monitor:** a pane graphing DTR/RTS/CTS/DSR/DCD/RI over time from the serial port's signal poll; uses the `TIOCMIWAIT`/`TIOCGICOUNT` counters if available. Fix `update()` after dispose (line 330), wire up `update_interval`, and handle export I/O errors.

**5.9 Presets** (`gtkterm_presets.c`):
- Validate the JSON members, since a missing one gives a NULL hash key (lines 281-300).
- Unify presets with the config profiles: a preset is a named port config.
- UI: a preset dropdown in the port dialog, plus "Save as preset".
- Re-enable `gtkterm_preset_clone` if it's needed, otherwise delete it.

**5.10 Automation** (`gtkterm_automation.c`):
- **Security:** `exec` runs `g_spawn_command_line_sync` on variable-expanded text that includes `${MATCH_n}` from the device (line 341). Change it to: argv-based async spawn (`g_subprocess_newv`), no device data ever expanded into argv, and `exec` disabled unless the setting "Allow scripts to run commands" is on (with a confirmation the first time).
- Turn the recursion into an iterative state machine (lines 188-384).
- Fix the regex leak on re-run (line 251), cap `receive_buffer` (line 548), make `start()` safe while WAITING (line 502), release BREAK (line 313), allow indented comments (line 391), and implement `CONDITIONAL` (lines 370-377).
- Add a `load_from_string` seam.
- **UI:** Run Script… / Stop, with the script output in the statusbar or a log pane. Ship the `data/examples/*.script` files as install data.
- **Tests:** run each example script against a loopback port with a scripted fake device; cover timeouts, regex captures, and `exec` refused by default.

**5.11 Macro recording:** call `gtkterm_macro_manager_record_command` from the terminal transmit path while recording is on.

**5.12 GtkSourceView:** it is linked but unused, and `data/language-specs/` isn't installed. Use it in the script editor (automation) and the macro editor with the shipped `.lang` files. Install them to `datadir/gtkterm-sunshine/language-specs` and add that to the `GtkSourceLanguageManager` search path.

---

## Phase 6 — UI modernisation & polish (M)

**6.1 GTK ≥4.10 deprecations** (the target is GTK 4.22):

| Deprecated | Where | Replacement |
|---|---|---|
| `GtkDialog`, `gtk_dialog_new_with_buttons` | `window.c:693` (port config), `macro_editor.ui` | `AdwPreferencesDialog` (port/terminal/MCP settings), `AdwDialog` (macro editor) |
| `gtk_message_dialog_new` | `:430` | `AdwAlertDialog` |
| `GtkFileChooserNative` | `:504, :1347` | `GtkFileDialog` |
| `GtkInfoBar` | `:403` and the template | `AdwBanner` (persistent states) and `AdwToast` (transient messages) |
| `gtk_widget_show`/`hide` | various | `gtk_widget_set_visible` |
| `gtk_show_about_dialog` | `window.c:1899` | `AdwAboutDialog` |
| `GtkStatusbar` | `config_terminal_dialog.ui` | remove the file if it is unused |

Also:
- The window template moves to `AdwApplicationWindow` + `AdwToolbarView` (header bar, optional toolbar, statusbar as the bottom bar).
- The dark-mode toggle (`window.c:2411`) goes through `AdwStyleManager`.
- The 0.8 version macros make any remaining deprecated call a build error.

**6.2 Shortcut map** (keeps the terminal usable, never uses Super+Space):

| Shortcut | Action |
|---|---|
| Ctrl+Shift+C / Ctrl+Shift+V | copy / paste |
| Ctrl+Shift+A | select all |
| Ctrl+Shift+B | send break |
| Ctrl+Shift+G | visualization panel |
| Ctrl+Shift+S | port config |
| Ctrl+Shift+R | save raw |
| Ctrl+Shift+Q | quit |
| F1 | about (help) |
| Ctrl+Shift+F | find |

Plain Ctrl+letter goes to the device, so remove `<Ctrl>a/q/F` (`menu.ui:262,34,61`). A test parses `menu.ui` plus the shortcut table and fails on duplicate accelerators.

**6.3 Menus and the About dialog:**
- Make echo, CR/LF, timestamp, show-index and view mode **stateful actions** (`g_action_new_stateful`). The menus then show check/radio states and stay in sync with the header dropdown.
- Fix hex width (the `menu.ui:208-224` items need `target=` and the action needs parameter type `i`). Fix `show_index` always being TRUE (`window.c:1797`).
- About: fix the leaks (`window.c:1879-1914`, also found by cppcheck), remove the duplicate `comments`, set the licence to GPL-3.0 (not LGPL), and fix the website and authors.

**6.4 i18n:** fix POTFILES and wrap the user-visible strings in `_()`; window.c currently has about one. Add `meson compile gtkterm-sunshine-pot` to CI.

**6.5 Docs:** update the man page options to match the code; they currently list `-c`/`-o`/`--speed`/`-L`. Rewrite `README.md` to cover features, the protocol spec for the Sunshine packet incl. chart prefixes, and build/test/debug instructions. Update `CLAUDE.md`, whose architecture section, Ctrl+Shift+V note, and statement that the ports are wired would all be wrong.

---

## Phase 7 — Full MCP server (L)

**Goal:** AI agents (Claude Code, Claude Desktop, any MCP client) can list, open and configure ports, send and receive data, wait for responses, read Sunshine series and decoded frames, run transfers, macros and scripts, and control logging. It works both against the running GUI (shared session, the human sees everything) and headless (CI, automation).

### 7.1 Protocol target
- **Primary:** MCP spec **2026-07-28**. It is stateless: no `initialize` handshake, no `Mcp-Session-Id`, and Streamable HTTP has no GET stream endpoint.
- **Backward compatibility:** **2025-11-25**, including the initialize/initialized handshake and session header, because current clients may still negotiate it. Version negotiation is done per request/connection.
- Verify every wire detail against the official spec at modelcontextprotocol.io before implementing; the summary here comes from secondary sources.
- **JSON-RPC 2.0:** requests, notifications, batch rejection per spec, and the standard error codes (−32700/−32600/−32601/−32602/−32603) plus tool-level `isError` results.

### 7.2 Architecture
New `src/mcp/` folder, built into the `gtkterm_core` library, GTK-free:

| File | Role |
|---|---|
| `mcp_server.c/.h` | `GtkTermMcpServer` GObject: method dispatch table, capability advertisement, version negotiation, pagination cursors, cancellation (`notifications/cancelled` → `GCancellable`), progress (`notifications/progress` from `progressToken`), logging (`logging/setLevel`, `notifications/message`, mapped from `g_log` domain `gtkterm`), completion (`completion/complete` for port names, profile names, packet IDs, macro names). Tool input/output JSON Schemas are declared statically and validated on input. Tool results include `structuredContent` plus text content. |
| `mcp_tools.c` | tool registry: name, title, description, `inputSchema`, `outputSchema`, annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`), handler. Async handlers return through `GTask`. |
| `mcp_resources.c` | resources, resource templates, `resources/subscribe` → `notifications/resources/updated`, `notifications/resources/list_changed` |
| `mcp_prompts.c` | prompt templates |
| `mcp_transport_stdio.c` | newline-delimited JSON over stdin/stdout via `GUnixInputStream`/`GDataInputStream` async; stdout is reserved for protocol, all logs go to stderr |
| `mcp_transport_http.c` | Streamable HTTP via **libsoup 3** `SoupServer` on `POST /mcp`: JSON response or `text/event-stream` (SSE) for streamed progress/notifications on long calls. Binds to 127.0.0.1/::1 only. Validates the `Origin` header (DNS-rebinding protection). Requires `Authorization: Bearer <token>`; the token is randomly generated, written to `$XDG_RUNTIME_DIR/gtkterm-sunshine/mcp-token` (mode 0600) and shown in Preferences. Honors `MCP-Protocol-Version`. |
| `mcp_policy.c` | permission policy (7.5) |

Everything runs on the GLib main loop (libsoup async, async stream reads), so there are no extra threads. Handlers call **`GtkTermSession`** (Phase 3) and **`GtkTermSunshineStore`** (Phase 4) only, never widgets.

### 7.3 Run modes
- **Embedded (GUI):** Preferences → "MCP server" toggle, plus port and token. The agent then works on the live sessions. An `AdwBanner` or statusbar icon shows "Agent connected (N calls)", and every agent write is echoed into an "Agent activity" log pane.
- **Headless stdio:** `gtkterm-sin-of-pride-version --mcp-stdio [--profile X]` runs a `GApplication` with no window (`G_APPLICATION_NON_UNIQUE`, no GTK init, so it works without a display). Register it with `claude mcp add gtkterm -- gtkterm-sin-of-pride-version --mcp-stdio`.
- **Headless HTTP:** `--mcp-http [127.0.0.1:]PORT` for multiple clients or remote-over-SSH-tunnel use.
- **Cmdline:** `--mcp-read-only` and `--mcp-allow-exec` flags (7.5).

### 7.4 Capabilities
**Tools** (each one maps onto session APIs built in Phases 1–5):

| Group | Tools |
|---|---|
| Ports | `list_ports` (serial via gudev with vendor/product/serial number, plus `/dev/pts` and the configured TCP/UDP profiles); `open_port` (type serial/tcp_client/tcp_server/udp + parameters, or `profile`); `close_port`; `configure_port` (baud, bits, parity incl. mark/space, stop bits, flow control, RS485 timings); `get_port_status` (state, stats, signals, TX queue) |
| I/O | `send` (`data` + `encoding`: text/hex/base64, `line_ending`: none/cr/lf/crlf, optional `delay_ms_per_char`); `read` (`since_cursor`, `max_bytes`, `timeout_ms`, `encoding`; returns data + `next_cursor`. Every session keeps a monotonically increasing byte cursor over the chunk store, so agents never miss or double-read); `expect` (send optional, then wait for a regex/PCRE2 or a byte count or a timeout; returns the match, captures and the preceding data. This is the main agent primitive for request/response devices); `send_break`; `set_signals` (DTR/RTS); `get_signals`; `clear_buffer`; `search_buffer` (regex over history) |
| Sunshine | `sunshine_list_series` (packet IDs, series, chart type, counts, min/max/last); `sunshine_get_series` (window by time or count, downsampling via LTTB to `max_points`); `sunshine_set_chart_type`; `sunshine_export_csv`; `sunshine_clear`; `sunshine_pause` |
| Decoders | `decoder_set` (protocol/auto); `decoder_get_frames` (cursor-based, with CRC validity); `modbus_request` (build an RTU/ASCII/TCP request with CRC, send it, wait for and decode the response; a high-value convenience tool) |
| Logging | `log_start` (format, path inside the allowed log directory); `log_stop`; `log_status` |
| Transfer | `file_send` / `file_receive` (XMODEM/1K/YMODEM/ZMODEM; reports progress notifications, cancellable; paths restricted to the allowed directories) |
| Macros & automation | `macro_list`; `macro_run`; `script_run` (automation script text or file, with `exec` refused unless explicitly allowed); `script_stop` |
| Config | `profile_list`; `profile_get`; `profile_save`; `preset_list`; `preset_apply` |

Long-running calls (`expect` with a long timeout, file transfers, scripts) accept `progressToken`, honor cancellation, and use MCP tasks where the negotiated version supports them.

**Resources** (subscribable ones are marked ⟳):
- `gtkterm://ports` ⟳
- `gtkterm://session/{id}/status` ⟳
- `gtkterm://session/{id}/rx{?since,encoding}` ⟳ (`notifications/resources/updated` is throttled to ≤10 Hz)
- `gtkterm://session/{id}/sunshine` ⟳ and `gtkterm://session/{id}/sunshine/{packet_id}` ⟳ (JSON series)
- `gtkterm://session/{id}/frames` ⟳ (decoded protocol frames)
- `gtkterm://session/{id}/log`
- `gtkterm://profiles`
- `gtkterm://docs/sunshine-protocol` (the packet spec, so agents can write firmware that emits it)

**Prompts:**
- `debug-serial-device` (probe baud rates and line endings, identify the protocol)
- `analyze-sunshine-data` (trends, anomalies over the selected series)
- `modbus-scan` (scan unit IDs and registers)
- `at-modem-check`
- `write-sunshine-firmware-snippet` (C/MicroPython code emitting the packet format)

### 7.5 Safety & policy
Agents writing to real hardware can have physical consequences.
- **Off by default.** HTTP binds to localhost only, with Origin validation and a bearer token. stdio trusts the parent process.
- **Policy levels:** `read-only` (status/read/sunshine/frames only; write tools are hidden from `tools/list`), `interactive` (default for embedded mode: the first write per session shows an `AdwAlertDialog` "Agent wants to write to /dev/ttyUSB0 — Allow once / Always / Deny"), and `full` (default for headless stdio, because the user started it explicitly).
- **Always off unless explicitly enabled:** the automation `exec` action (`--mcp-allow-exec` plus a Preferences switch).
- **File paths:** file tools are restricted to configured allowed directories (default: the log dir and `~/Downloads/gtkterm`). Paths are canonicalised with `g_canonicalize_filename`, and symlink escapes are rejected.
- **Limits:** caps on read size (1 MiB per call), expect timeout (≤10 min), send size, and requests per second. Notifications are throttled.
- **Audit:** every tool call (name, args summary, result status) goes into the "Agent activity" pane and an optional JSONL audit log.

### 7.6 MCP tests
- **Unit:** a golden-file dispatcher test. `tests/mcp/cases/*.json` holds request → expected response pairs, run against a server with a loopback session. It covers:
  - every tool's schema validation: missing/extra/wrong-typed arguments must return −32602
  - pagination, unknown method, malformed JSON, notifications without an id
  - version negotiation (2026-07-28 vs 2025-11-25 handshake)
  - cancellation of an `expect` in progress
  - progress notifications during a transfer
  - resource subscribe → updated notification after data arrives
  - each policy level hiding or refusing write tools
  - path traversal attempts on file tools
- **Transport:**
  - **stdio:** spawn the headless binary with `GSubprocess` and talk to it over pipes.
  - **HTTP:** use libsoup's `SoupSession` against an in-process server. Test with a wrong token → 401, a bad Origin → 403, SSE streaming, and concurrent clients.
- **End-to-end** (integration suite):
  - `tests/mcp/e2e_client.py` is a uv script using the official `mcp` Python SDK (latest). It starts `--mcp-stdio`, creates a socat or openpty PTY pair with `tools/sunshine-gen.py` on the far side, then runs `list_ports` → `open_port` → `expect` (a fake device answering "AT" with "OK") → `sunshine_get_series` (checks values) → `modbus_request` against a fake Modbus slave → `file_send` via YMODEM to `rb` → `close_port`.
  - The same scenario runs over HTTP.
- **Conformance:**
  - `npx @modelcontextprotocol/inspector@latest --cli` scripted calls (tools/list, resources/list, prompts/list, a tools/call) in CI.
  - The official MCP conformance test suite, if one is published for 2026-07-28; check at implementation time.
- **Fuzz:** a `fuzz_mcp_jsonrpc` target fed random/mutated JSON-RPC messages into the dispatcher, seeded with the golden cases.
- **Manual:** `claude mcp add gtkterm -- …/gtkterm-sin-of-pride-version --mcp-stdio`, then ask Claude Code to "open /tmp/ttyA at 115200, wait for a Sunshine packet and summarise the Temperature series". Repeat against the GUI over HTTP (`claude mcp add --transport http gtkterm http://127.0.0.1:PORT/mcp --header "Authorization: Bearer …"`) and watch the activity pane.

### 7.7 Docs
`docs/mcp.md`: setup for Claude Code and Claude Desktop, the tool/resource/prompt reference (generated from the registry by a `--mcp-dump-schema` flag, so it never goes stale), the security model, and example agent sessions.

---

## Test plan summary

| Suite | Runner | Content |
|---|---|---|
| `validate` | meson test | desktop-file-validate, schema `--strict`, `gtk4-builder-tool validate` on each .ui, duplicate-accelerator check, POTFILES existence |
| `unit` | g_test (TAP), headless | buffer transform and ring, data_chunk, config round-trip, cmdline, macros.c, macro JSON, hex parser, Sunshine parser/scanner/chart_util, XMODEM/YMODEM loopback + CRC, every decoder's vectors and frame assembler, logging formats, automation interpreter |
| `integration` | g_test + openpty / GSocket loopback (+ socat/lrzsz where available, skip otherwise) | serial open/close/reopen/lock/termios readback/1 MB exactness/EOF; TCP client and server with multiple clients; UDP v4/v6; full pipeline port→buffer→terminal+Sunshine+logging; XMODEM interop with `sx`/`rx` |
| `gui` | `xvfb-run -a`, `GDK_BACKEND=x11`, `G_DEBUG=fatal-criticals` | launch the app against a loopback/PTY port, inject packets, assert tabs and chart types, toggle every action via `g_action_group_activate_action`, open and close every dialog, quit; render-to-PNG smoke per chart |
| `mcp` | g_test golden cases + GSubprocess/libsoup transport tests + uv/`mcp` SDK e2e + Inspector CLI | see 7.6 |
| `fuzz` (opt-in) | AFL++ / libFuzzer | sunshine parser, scanner feed, each decoder, hex parser, macro JSON load, config keyfile load, automation script load, MCP JSON-RPC dispatcher |

**Coverage goal:** ≥80% lines for `src/sunshine/*`, buffer, hex, xmodem and decoders; ≥60% overall (gcovr report as a CI artifact).

**Test helpers** (`tests/support/`):
- `gtkterm-test-port.c` (loopback `GtkTermPort`)
- `pty-pair.c` (openpty wrapper with a non-blocking slave)
- `fake-device.c` (scripted request/response peer)
- `tmp-config.c` (sets `XDG_CONFIG_HOME` to a `g_dir_make_tmp` directory)
- `packets/*.txt` (shared vectors, reused as fuzz seeds)

**Manual hardware checklist:** `docs/testing/hardware.md`, covering USB-UART loopback, high and custom baud, RS485, DTR/RTS/CTS lines, hotplug (unplug while streaming), and ZMODEM to real `rz` on a target.

## Debugging toolkit (documented in `docs/debugging.md`, wired into meson)

- **Logging:** `G_MESSAGES_DEBUG=gtkterm`, with optional per-area sub-domains (`gtkterm-serial`, `gtkterm-sunshine`, …).
- **GLib/GTK diagnostics:**
  - `G_DEBUG=fatal-criticals gdb --args …` to break on the first Gtk-CRITICAL.
  - `GTK_DEBUG=interactive` for the GTK Inspector (widget tree, CSS, actions).
  - `GOBJECT_DEBUG=instance-count` with `GTK_DEBUG=interactive`'s Statistics tab for object leaks.
- **Sanitizers:** `meson setup build-asan -Db_sanitize=address,undefined -Db_lundef=false` plus `meson test --setup=asan`, with a suppression file for fontconfig/mesa leaks.
- **Valgrind:** `meson test --setup=valgrind`, or for the app `G_SLICE=always-malloc G_DEBUG=gc-friendly valgrind --leak-check=full --suppressions=/usr/share/glib-2.0/valgrind/glib.supp --suppressions=tests/gtk.supp ./build/src/gtkterm-sin-of-pride-version`.
- **Static analysis:** `ninja -C build clang-tidy` (meson builtin with `.clang-tidy`), `cppcheck --library=gtk --project=build/compile_commands.json`, and `scan-build meson compile -C build-sa`. These replace the one-file clang-tidy recipe in CODE_ANALYSIS_TOOLS.md.
- **Profiling:** `perf record -g` and `sysprof` replace gprof, which can't profile the shared GTK libraries meaningfully. Add a benchmark (`tests/bench/bench_pipeline.c`) that streams 50 MB through buffer, Sunshine and logging and reports MB/s and frame times. The existing `test_performance.c` is only a demo.
- **Serial I/O debugging:**
  - `socat -d -d -x pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB` (hex trace of both directions)
  - `strace -e ioctl,read,write -p <pid>` to check the termios/TIOCM calls
  - `interceptty` or a logic analyser for real lines
- **Packet generator:** a `tools/sunshine-gen.py` (uv script) that writes configurable Sunshine/NMEA/Modbus streams to a PTY or TCP port at a set rate, with split, garbage and burst modes. It's used for manual testing, the benchmark, and GUI tests.
- **Fuzzing:** `meson setup build-fuzz -Dfuzz=true` with `CC=clang`, then `./build-fuzz/tests/fuzz/fuzz_sunshine_parser corpus/ -max_total_time=600`, or AFL++ `afl-fuzz -i tests/packets -o out -- ./fuzz_target @@`.

---

## Execution order & milestones

| Milestone | Content | Exit criteria |
|---|---|---|
| **M0** | Phase 0 | clean clone builds; validate suite + CI green |
| **M1** | Phase 1 | unit and integration suites ASan/valgrind-clean; the 10 MB stream test passes |
| **M2** | Phase 2 + 4.1 | serial PTY suite green; Sunshine works past 20k chunks and with split packets |
| **M3** | Phase 3 | the same pipeline tests pass for serial, TCP and UDP |
| **M4** | Phase 4.2–4.4 | all 4 chart types reachable; export/pause/zoom; parser fuzzed 1 h with no crash |
| **M5** | Phase 5.1–5.4 | logging, statusbar, toolbar, shortcuts, hex send |
| **M6** | Phase 5.5–5.7 | XMODEM/YMODEM interop with lrzsz; decoders and dataview live |
| **M7** | Phase 5.8–5.12 + 6 | every module reachable from the UI; zero GTK deprecation warnings; docs updated |
| **M8** | Phase 7 | MCP golden, transport and e2e suites green on both protocol versions; Inspector CLI checks pass; a Claude Code session drives a PTY device end-to-end in both headless and GUI mode |

**Order:** Phase 7.1–7.2 (dispatcher plus stdio, read-only tools) can start right after M3, because it needs only `GtkTermSession`. Each later tool group lands together with the phase that delivers its backend.

One commit per logical fix, using `git commit -m` (each with its regression test), pushed to Forgejo and checked in CI.

## Verification (end-to-end)

1. `meson setup build -Dtests=true && meson test -C build --print-errorlogs` (all suites); `meson test -C build-asan --setup=asan`; `meson test -C build --setup=valgrind --suite unit`.
2. Manual pipeline check:
   - `socat pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB &`
   - `meson devenv -C build ./src/gtkterm-sin-of-pride-version -p /tmp/ttyA -s 115200`
   - `uv run tools/sunshine-gen.py /tmp/ttyB --rate 1000 --split --ids P0,B:P1,P:P2,S:P3`
   - Expect: 4 tabs of the right types, still updating after 20k+ chunks; pause/export work; the log file contains data.
3. TCP: `ncat -lk 5000` ↔ `--tcp 127.0.0.1:5000`, then send packets both ways. UDP likewise with `ncat -u`.
4. XMODEM/YMODEM: send a file to `rx` and receive from `sx` over the socat pair, then `cmp` the files.
5. Fuzz: 1 h on each target with no crash or hang; the coverage report meets the targets.
6. Hardware checklist on a real USB-UART and an RS485 adapter.
7. MCP:
   - Run `uv run tests/mcp/e2e_client.py --transport stdio` and `--transport http`.
   - Run `npx @modelcontextprotocol/inspector@latest --cli … --method tools/list`.
   - Do a live Claude Code session with `claude mcp add gtkterm -- ./build/src/gtkterm-sin-of-pride-version --mcp-stdio`.
8. Install into a staging prefix (`meson install --destdir /tmp/stage`, scratchpad) and run `desktop-file-validate`. Confirm the installed files don't collide with upstream gtkterm package paths (compare against `dpkg -L gtkterm` if available).
