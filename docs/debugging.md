# Debugging

## Logs

All diagnostics go to stderr (stdout carries terminal data or the MCP protocol).

```bash
gtkterm-sunshine -v ...            # info
gtkterm-sunshine -vv ...           # debug
RUST_LOG=gtkterm_core::session=trace,gtkterm_mcp=debug gtkterm-sunshine ...
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

## Fuzzing

```bash
cd fuzz
cargo +nightly fuzz list
cargo +nightly fuzz run sunshine_scanner -- -max_total_time=600
cargo +nightly fuzz tmin sunshine_scanner artifacts/sunshine_scanner/crash-...   # minimise, then add a regression test
```
