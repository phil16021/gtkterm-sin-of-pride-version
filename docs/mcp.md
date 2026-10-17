# MCP server

`gtkterm-sunshine` implements the [Model Context Protocol](https://modelcontextprotocol.io)
with the official Rust SDK (rmcp 3.5). Protocol revision **2026-07-28** (stateless) is the
default; clients that still use the `initialize` handshake (2025-11-25 and older) are served
as well.

## Setup

| Client | Command |
|---|---|
| Claude Code, stdio (headless) | `claude mcp add gtkterm -- gtkterm-sunshine --mcp-stdio` |
| Claude Code, with a port opened at start | `claude mcp add gtkterm -- gtkterm-sunshine --mcp-stdio -p /dev/ttyUSB0 -s 115200` |
| Read-only access | add `--mcp-read-only` |
| GUI window | menu → *MCP Server for AI Agents* (or Preferences → MCP); a toast offers the `claude mcp add` command |
| HTTP (shared with a running instance) | `gtkterm-sunshine --headless -p /dev/ttyUSB0 --mcp-http 8765` prints the URL and token, then `claude mcp add --transport http gtkterm http://127.0.0.1:8765/mcp --header "Authorization: Bearer <token>"` |
| MCP Inspector | `npx @modelcontextprotocol/inspector@latest gtkterm-sunshine --mcp-stdio` |

The HTTP endpoint binds to `127.0.0.1` by default, requires the bearer token (`--mcp-token` or
`GTKTERM_MCP_TOKEN`, otherwise random) and rejects browser requests from foreign origins.

## Tools

| Tool | Purpose |
|---|---|
| `list_ports` | serial devices (udev: vendor, model, serial number), sessions, saved profiles |
| `open_port` | open `{"type":"serial","port":...,"baudrate":...}`, `tcp_client`, `tcp_server`, `udp`, or a saved `profile` |
| `close_port`, `get_status` | connection state, endpoint, byte counters, modem lines, log state |
| `send` | data as `text`, `hex` or `base64`, optional `line_ending` (`cr`, `lf`, `crlf`); returns an `rx_cursor` |
| `read` | received data from a byte `cursor`, optionally waiting `timeout_ms`; returns `next_cursor` and `lost` |
| `expect` | optionally send, then wait for a regex; returns match, captures and preceding text |
| `send_break`, `set_signals`, `get_signals` | serial break, DTR/RTS, modem lines |
| `clear_buffer`, `search_buffer` | history management and regex search |
| `sunshine_list_series` | charts and per-series min/max/mean/last, packet/error counters |
| `sunshine_get_series` | samples (LTTB-downsampled to `max_points`) |
| `sunshine_export_png` | the chart as a PNG image (as the user sees it; GUI builds) |
| `sunshine_set_chart_type`, `sunshine_pause`, `sunshine_clear`, `sunshine_export_csv` | chart control |
| `log_start`, `log_stop`, `log_status` | session logs (raw/text/csv/json/xml) inside the log directory |
| `macro_list`, `macro_save` | macros in the editor syntax (`send:`, `send[ms]:`, `wait:`, `expect[ms]:`) |
| `macro_run` | run a macro and wait for its report (expect steps fail with the text received so far) |
| `preset_list`, `preset_apply` | serial presets; applying reopens the serial port with new line settings |
| `profile_list`, `profile_get`, `profile_save` | saved profiles; `profile_save` stores the session's current settings |
| `decoder_set` | protocol decoder: `off`, `auto`, `raw`, `at`, `nmea`, `modbus_rtu`, `modbus_ascii`, `modbus_tcp`, `can`, `slcan`, `ethercat`, `rs232`; `signals` adds modem line changes |
| `decoder_get_frames` | decoded frames since a sequence number (`next_seq`), with source (rx/tx/lines), summary, fields, checksum result and bytes; can wait `timeout_ms` |
| `modbus_request` | build a Modbus request (RTU/ASCII/TCP framing incl. CRC/LRC/MBAP), send it, wait for the matching response and return registers, bits, the write echo or the exception |
| `file_send` | upload files with `xmodem`, `xmodem_1k`, `ymodem`, `ymodem_g` or `zmodem` (lrzsz); reports progress notifications, cancelling the call aborts the transfer |
| `file_receive` | download into the transfer directory (optional `subdir`; XMODEM takes a `name`) |
| `file_cancel` | cancel a running transfer, including one started in the window |
| `script_run` | run an automation script (`script` text or a `path` in the transfer directory) with initial `vars`; `log` lines arrive as progress notifications; failures name the line and include the log and received data |
| `script_check`, `script_stop` | validate a script without running it; stop the running script or macro |
| `signal_history` | modem line samples of the last `seconds` (≤ 300), or CSV |
| `port_flush`, `port_auto_baud` | discard driver buffers; try baud rates and reopen at the one where the device's output is readable |

Write tools are hidden in read-only mode: open/close, send, expect, break, signals, clearing,
chart changes, logging, macro_run/macro_save, preset_apply, profile_save, decoder_set,
modbus_request, the file tools, script_run/script_stop, port_flush and port_auto_baud.

**Scripts.** Read `gtkterm://docs/script-language` (or call `script_check`) before writing one.
`exec` is refused unless the server runs with `--mcp-allow-exec` (in the window: Preferences →
MCP → *Agents may use exec in scripts*); even then programs are started without a shell.
`include`, `send_file` and `receive_file` paths must stay inside the transfer directory or an
`--mcp-allow-dir` directory.

**File transfers.** The device must be ready first (its receiver started, or about to send).
`file_send` and `file_receive` return when the transfer is over, with the files, byte count,
duration and retries; a failed transfer is a tool error with the reason. Clients that send a
`progressToken` get `notifications/progress` with bytes so far (and the total when sending).
`get_status` shows the running or last transfer under `transfer`.

**Frames.** Decoded frames get increasing sequence numbers; the last 10 000 are kept. Poll
`decoder_get_frames` with `since` = the previous `next_seq`; `dropped` counts frames that were
evicted before you read them. `modbus_request` switches the decoder to its framing if needed
(`decoder_changed` in the result) and matches the response by unit, function and, for Modbus
TCP, transaction ID. Example:

```json
{"unit": 1, "request": {"function": "read_holding_registers", "address": 0, "count": 2}}
→ {"response": {"kind": "registers", "values": [42, 7]}, "request": "01 03 00 00 00 02 C4 0B", ...}
```

**Cursors.** Every received byte has a position in the session's RX stream. `send` returns the
position at send time, so `read`/`expect` from there see exactly the reply. Keep passing
`next_cursor` to read a stream without gaps; `lost` reports bytes evicted from history.

## Resources and prompts

Resources: `gtkterm://ports`, `gtkterm://profiles`, `gtkterm://macros`, `gtkterm://docs/sunshine-protocol`,
`gtkterm://docs/script-language`, `gtkterm://session/{id}/signals` (CSV),
`gtkterm://session/{id}/status`, `.../rx` (last 64 KiB), `.../frames` (last 200 decoded frames), `.../sunshine`,
`.../sunshine/{packet_id}` (CSV).

Prompts: `debug-serial-device`, `analyze-sunshine-data`, `modbus-scan`, `at-modem-check`,
`write-sunshine-firmware-snippet`.

## Safety

Agents writing to real hardware can have physical effects. Use `--mcp-read-only` when an agent
should only observe. Log files can only be created inside the log directory
(`~/.local/share/gtkterm-sunshine/logs`); file names with path components are rejected.
`file_send` reads only files inside the transfer directory
(`~/.local/share/gtkterm-sunshine/transfers`) or a directory given with `--mcp-allow-dir DIR`
(repeatable), checked after resolving symlinks; `file_receive` writes only below the transfer
directory, and file names sent by the device are reduced to plain names.
