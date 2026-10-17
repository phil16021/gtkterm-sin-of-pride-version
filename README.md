# GTKTerm Sunshine

A Linux serial / TCP / UDP terminal with **live plotting of device data** ("Sunshine" packets)
and a built-in **MCP server**, so AI agents such as Claude Code can drive your devices.

This is a Rust rewrite of the GTKTerm "Sin of Pride" fork (the C version lives at tag
`legacy-c-v1.99.1`). Status **2.0.0-alpha.2**: GTK 4 / libadwaita application, headless
terminal, MCP server, macros and presets work (milestones R0–R5). Protocol decoders,
XMODEM/YMODEM, scripting and the signal monitor follow — see [docs/ROADMAP.md](docs/ROADMAP.md).

## Sunshine packets

Print lines like these from your firmware and they are plotted, one chart per ID:

```
{P0|Temperature|255,0,0|23.5}
{imu|Roll|255,0,0|45.2|Pitch|0,255,0|12.3|Yaw|0,0,255|180.0}
{B:levels|Red|10|Green|20}        chart prefixes: L: line (default), B: bar, P: pie, S: scatter
{S:xy|path|1.0 2.5}               two values = an X/Y point
```

Colours (`R,G,B`) are optional, decimals use `.`. Full spec: `gtkterm-sunshine --mcp-stdio`
exposes it as the resource `gtkterm://docs/sunshine-protocol`, or read
[crates/gtkterm-mcp/src/sunshine_protocol.md](crates/gtkterm-mcp/src/sunshine_protocol.md).

## Build and install

```bash
# needs: libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libgtksourceview-5-dev libudev-dev
cargo xtask install                      # into ~/.local (binary, desktop entry, icons, man page, completions)
cargo xtask install --prefix /usr --destdir pkg/   # staging for packages
# Headless terminal + MCP server only, without GTK:
cargo build --release -p gtkterm-gui --no-default-features
```

## The window

- Header bar: port (live udev list), baud rate, Connect, view mode (ASCII, hex, binary,
  decimal and mixed dumps), search, chart panel, menu.
- Charts: one tab per packet ID; scroll to zoom, drag to pan, double-click to follow, click a
  legend entry to hide a series, hover for values; pause, time axis, chart type, history
  length, CSV/PNG export.
- Status bar: profiles, endpoint, RX/TX counters, DTR/RTS toggles, CTS/DSR/DCD/RI, log and MCP state.
- Shortcuts use Ctrl+Shift or F-keys (Ctrl+letter goes to the device); press Ctrl+? for the list.

## Macros and presets

Macros (keyboard icon in the header bar) are step lists, edited as text:

```
send: ATZ\r                 # escapes: \r \n \t \0 \e \\ \xHH
expect[2000]: OK\r\n        # wait up to 2 s for a regular expression
wait: 500 ms
send[250]: AT+CSQ\r         # pause 250 ms, then send
```

Give a macro a shortcut (F2–F4, F9–F12 or any combination with a modifier that is not an
application shortcut), show the macro bar, or record what you type (Macros → Record). Macros are
stored in `~/.config/gtkterm-sunshine/macros.json`; old GTKTerm macros are imported once.

Preferences → Connection → *Apply preset* fills in line settings (Arduino, ESP32, GPS, HC-05,
Modbus RTU 8E1, modem, Raspberry Pi, STM32, legacy 7E1, 921600 RTS/CTS); *Save as preset* adds
your own; menu → *Import Presets…* reads the old GTKTerm presets JSON.

## Use

```bash
gtkterm-sunshine -p /dev/ttyUSB0 -s 115200                      # window, opens the port
gtkterm-sunshine --headless -p /dev/ttyUSB0 -s 115200          # terminal on stdin/stdout
gtkterm-sunshine --headless --hexdump --tcp 192.168.1.50:23     # hex dump of a TCP stream
gtkterm-sunshine --headless --udp '9000->10.0.0.7:9001' --log-file run.csv
gtkterm-sunshine -U bench -p /dev/ttyACM0 -s 921600 -S          # save a profile
gtkterm-sunshine -L                                             # list profiles
```

The GTKTerm options keep their names (`--port/-p`, `--baudrate/-s`, `--bits/-b`, `--parity/-a`,
`--stopbits/-t`, `--flow_control/-w`, `--term_echo/-e on|off`, `--show_config/-V`, ...); `-h`
is help, so auto-LF moved from `-h` to `-j`. Settings live in
`~/.config/gtkterm-sunshine/config.toml`; an old `~/.config/.gtktermrc` is imported once.

## MCP (AI agents)

```bash
claude mcp add gtkterm -- gtkterm-sunshine --mcp-stdio            # headless, agent owns the port
```

Or share the open window with an agent: menu → *MCP Server for AI Agents*; the toast copies the
`claude mcp add --transport http …` command.

Then ask e.g. *"open /dev/ttyUSB0 at 115200, wait for a Sunshine packet and summarise the
Temperature series"*. Tools cover ports, send/read/expect, modem lines, Sunshine series and
logging; `--mcp-read-only` hides everything that writes. See [docs/mcp.md](docs/mcp.md).

## Development

```bash
cargo xtask check                          # fmt, clippy, all tests (GUI tests run in Xvfb)
cargo nextest run                          # core, protocols, MCP
cargo nextest run -p gtkterm-gui --no-default-features   # binary end-to-end tests
cargo run -p gtkterm-testkit --bin sunshine-gen -- /tmp/ttyB --rate 50   # test data
cd fuzz && cargo +nightly fuzz run sunshine_scanner
```

Debugging recipes: [docs/debugging.md](docs/debugging.md). Hardware checklist:
[docs/testing/hardware.md](docs/testing/hardware.md).

## License

GPL-3.0-or-later, like GTKTerm.
