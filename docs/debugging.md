# Debugging

## Logs

All diagnostics go to stderr (stdout carries terminal data or the MCP protocol).

```bash
gtkterm-sunshine -v ...            # info
gtkterm-sunshine -vv ...           # debug
RUST_LOG=gtkterm_core::session=trace,gtkterm_mcp=debug gtkterm-sunshine ...
gtkterm-sunshine --trace-file /tmp/trace.jsonl ...   # JSON lines at debug level (GTKTERM_TRACE overrides)
jq -c 'select(.spans[0].profile == "lab") | {timestamp, target, msg: .fields.message}' /tmp/trace.jsonl
```

Every event carries its spans: `session` (`id`, `profile`) for the session actor and its port
tasks, and `writer` (`port`) for the transmit task.

**tokio-console** (async tasks, wakeups, busy loops):

```bash
RUSTFLAGS="--cfg tokio_unstable" cargo run -p gtkterm-gui --features console -- --headless -p /tmp/ttyA
tokio-console                      # cargo install tokio-console; connects to 127.0.0.1:6669
```

## Virtual serial ports

```bash
socat -d -d pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB &      # a null-modem pair
socat -d -d -x pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB &   # ... with a hex trace
gtkterm-sunshine --headless -p /tmp/ttyA
cargo run -p gtkterm-testkit --bin sunshine-gen -- /tmp/ttyB --rate 50 --split --garbage-every 7 --chatter
```

Pseudo terminals accept any baud rate but force 8 data bits without parity and have no modem
lines; use real hardware for those (see testing/hardware.md).

## Serial port details

- **Exclusive access** uses `flock(LOCK_EX)` on the device (other programs using flock, such as
  minicom and picocom, see it; *disable_port_lock* turns it off). UUCP lock files in
  `/run/lock` are not written: they need group `lock` (or root) on most systems today, and the
  programs that still honour them also take flock.
- **Reading** is non-blocking through the async reactor with `VMIN = 1`, `VTIME = 0`; every
  read returns what the driver has. The C settings `VMIN`/`VTIME` (read sizes and inter-byte
  timeouts) have no effect on that model and are not offered. Frame gaps (Modbus RTU) are
  measured with timestamps instead.
- **Modem lines** are polled every 50 ms; the kernel's line counters (`TIOCGICOUNT`) catch
  pulses shorter than that, and the signal monitor marks them. `TIOCMIWAIT` is not used: a
  thread waiting in it cannot be woken without hanging up the tty, so it would keep the device
  open (and locked) after Close.
- **TCP server**: data from each client is tagged with its address (the frames pane, MCP
  `read` → `peers`, decoded frames → `peer`).
- **UDP**: longer sends are split at *Largest datagram* (`max_datagram`, default 65 507).

## Inspecting the device side

```bash
strace -f -e trace=ioctl,read,write -p $(pidof gtkterm-sunshine)      # termios / TIOCM* calls
gtkterm-sunshine --headless --hexdump -p /dev/ttyUSB0                  # see exactly what arrives
cargo run -p gtkterm-core --example list_ports                         # what udev reports
udevadm monitor --udev --subsystem-match=tty                            # hotplug events
```

## MCP

```bash
npx @modelcontextprotocol/inspector@latest gtkterm-sunshine --mcp-stdio     # interactive inspector
gtkterm-sunshine --headless -p /tmp/ttyA --mcp-http 8765 -vv                # watch requests
curl -s -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -H 'accept: application/json, text/event-stream' -H 'mcp-protocol-version: 2026-07-28' \
     -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' http://127.0.0.1:8765/mcp
```

## Agents (MCP audit log)

```bash
tail -f ~/.local/share/gtkterm-sunshine/mcp-audit.jsonl | jq -c '{time, tool, outcome, ms}'
jq 'select(.outcome == "denied")' ~/.local/share/gtkterm-sunshine/mcp-audit.jsonl
```

## Translations

```bash
cargo xtask po                                           # refresh po/gtkterm-sunshine.pot and po/*.po
msgfmt -o /tmp/l/de/LC_MESSAGES/gtkterm-sunshine.mo po/de.po
LANGUAGE=de GTKTERM_LOCALEDIR=/tmp/l cargo run -p gtkterm-gui     # try a translation uninstalled
```

## Debuggers, sanitizers, profilers

```bash
rust-gdb --args target/debug/gtkterm-sunshine --headless -p /tmp/ttyA
RUST_BACKTRACE=1 cargo nextest run -p gtkterm-core
cargo +nightly miri test -p gtkterm-protocols                                  # UB check of pure code
RUSTFLAGS=-Zsanitizer=address cargo +nightly test -p gtkterm-core --target x86_64-unknown-linux-gnu
perf record -g target/release/gtkterm-sunshine --headless -p /tmp/ttyA        # then: perf report
GTK_DEBUG=interactive gtkterm-sunshine                                         # GTK Inspector (GUI)
```

## Quality checks

```bash
cargo xtask check                 # what CI runs: fmt, translations, clippy, all tests, deny/audit/machete, MCP Inspector
cargo xtask cov                   # line coverage with per-crate minimums; HTML in target/llvm-cov/html
cargo xtask inspector             # the official MCP Inspector lists the tools and checks schema portability
cargo xtask bench-pipeline        # 50 MiB through a PTY into a session (release)
cargo bench -p gtkterm-protocols --bench pipeline     # also: gtkterm-core --bench store, gtkterm-charts --bench render
UPDATE_REFERENCE=1 cargo test -p gtkterm-charts --test render_reference   # new chart reference images
cargo insta review                # changed formatter / log / schema snapshots
```

The named C-defect guards and where they live: `crates/gtkterm-core/tests/regressions/README.md`.

## Fuzzing

```bash
cd fuzz
cargo +nightly fuzz list
cargo +nightly fuzz run sunshine_scanner -- -max_total_time=600
cargo +nightly fuzz run mcp_arguments -- -max_total_time=60      # agent-supplied tool arguments
cargo +nightly fuzz run presets_import -- -max_total_time=60     # C preset files
cargo +nightly fuzz tmin sunshine_scanner artifacts/sunshine_scanner/crash-...   # minimise, then add a regression test
```
