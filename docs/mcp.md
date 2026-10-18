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
| `send` | data as `text`, `hex` or `base64`, optional `line_ending` (`cr`, `lf`, `crlf`) and `char_delay_ms` (pause after every byte, for slow bootloaders); returns an `rx_cursor` |
| `read` | received data from a byte `cursor`, optionally waiting `timeout_ms`; returns `next_cursor` and `lost` |
| `expect` | optionally send, then wait for a regex, or for literal bytes with `pattern_encoding` (`hex`: `"AA 55"`); returns match, captures and preceding text |
| `send_break`, `set_signals`, `get_signals` | serial break, DTR/RTS, modem lines |
| `clear_buffer`, `search_buffer` | history management and regex search |
| `sunshine_list_series` | charts and per-series min/max/mean/last, packet/error counters |
| `sunshine_get_series` | samples (LTTB-downsampled to `max_points`), optionally only the `last_n` or the `last_seconds` |
| `sunshine_export_png` | the chart as a PNG image (as the user sees it; GUI builds) |
| `sunshine_set_chart_type`, `sunshine_pause`, `sunshine_clear`, `sunshine_export_csv` | chart control |
| `log_start`, `log_stop`, `log_status` | session logs (raw/text/csv/json/xml) inside the log directory |
| `macro_list`, `macro_save` | macros in the editor syntax (`send:`, `send[ms]:`, `wait:`, `expect[ms]:`) |
| `macro_run` | run a macro and wait for its report (expect steps fail with the text received so far) |
| `preset_list`, `preset_apply` | serial presets; applying reopens the serial port with new line settings |
| `profile_list`, `profile_get`, `profile_save`, `profile_delete` | saved profiles; `profile_save` stores the session's current settings |
| `configure_port` | change baud rate, data bits, parity, stop bits or flow control of an open serial port (it is reopened; history is kept) |
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

Write tools (marked in the [reference](#tool-reference) below) are hidden in read-only mode.
Wrong arguments are tool errors that name the problem, so the agent can correct the call; an
unknown tool is a protocol error (-32602). `gtkterm-sunshine --mcp-dump-schema` prints all tools,
prompts and resources with their JSON schemas.

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
`gtkterm://session/{id}/status`, `.../rx` (last 64 KiB), `.../rx?since=CURSOR&encoding=hex` (JSON
with `data`, `next_cursor`, `lost`; at most 64 KiB), `.../log` (log file status and its last 4 KiB),
`.../frames` (last 200 decoded frames), `.../sunshine`, `.../sunshine/{packet_id}` (CSV).

**Change notifications.** Clients can follow resources instead of polling: with MCP 2026-07-28
through `subscriptions/listen`, with 2025-11-25 through `resources/subscribe`. The server sends
`notifications/resources/updated` for `ports`, `profiles`, `macros` and the per-session `status`,
`rx`, `signals`, `frames`, `sunshine` and `sunshine/{packet_id}` resources, at most 10 times per
second per resource (a burst of data is one update), and `notifications/resources/list_changed`
when sessions appear or go. The documents never change and cannot be subscribed. Read the
resource again after an update; for `rx`, read `rx?since=` with the last `next_cursor`.

**Completions.** `completion/complete` suggests session ids and packet IDs for the resource
templates (`packet_id` uses the `id` already chosen), packet IDs for the focus of
`analyze-sunshine-data`, and ports and profiles for `debug-serial-device`.

The MCP logging capability is not offered: it is deprecated in 2026-07-28 (SEP-2577); device
messages are in `get_status` and the received data.

Prompts: `debug-serial-device`, `analyze-sunshine-data`, `modbus-scan`, `at-modem-check`,
`write-sunshine-firmware-snippet`.

## Safety

Agents writing to real hardware can have physical effects. Use `--mcp-read-only` when an agent
should only observe.

**Approval (window).** The embedded server asks before an agent's write tool runs: *Deny*,
*Allow Once* or *Allow Until the Server Stops* (Preferences → MCP → *Ask before agents change
anything*, on by default). "Until the server stops" applies to the client that was asked,
recognised by the name and version it reports (two instances of the same client share it). A denied call is a tool error that tells the agent to ask you; an
unanswered question is denied after two minutes. Headless servers (`--mcp-stdio`,
`--mcp-http`) do not ask: whoever starts them chooses the access with `--mcp-read-only`.

**Rate limit.** Each client may run 20 write tools per second (bursts of 40); more are refused
with a tool error that suggests batching (one `script_run` instead of many `send`s). Change it
with `--mcp-rate N` or Preferences → MCP → *Write calls per second*; 0 turns the limit off.

**Audit log.** Every tool call is appended to `~/.local/share/gtkterm-sunshine/mcp-audit.jsonl`
(mode 0600; moved to `.1` beyond 8 MiB): time, tool, client name, arguments (cut at 4 KiB),
`outcome` (`ok`, `error`, `denied`, `limited`), error text and duration. For example:
`jq -r 'select(.outcome != "ok") | [.time, .tool, .error] | @tsv' ~/.local/share/gtkterm-sunshine/mcp-audit.jsonl`.

 Log files can only be created inside the log directory
(`~/.local/share/gtkterm-sunshine/logs`); file names with path components are rejected.
`file_send` reads only files inside the transfer directory
(`~/.local/share/gtkterm-sunshine/transfers`) or a directory given with `--mcp-allow-dir DIR`
(repeatable), checked after resolving symlinks; `file_receive` writes only below the transfer
directory, and file names sent by the device are reduced to plain names.

## Tool reference

<!-- tool-reference:begin (generated by crates/gtkterm-mcp/tests/docs.rs) -->
| Tool | Write | Parameters (`*` required) | Description |
|---|---|---|---|
| `clear_buffer` | yes | `session` integer | Clear the received history and all Sunshine charts of a session. |
| `close_port` | yes | `session` integer | Close the connection of a session. |
| `configure_port` | yes | `baudrate` integer, `bits` integer, `flow_control` string, `parity` string, `session` integer, `stopbits` integer | Change the line settings (baud rate, data bits, parity, stop bits, flow control) of an open serial port. |
| `decoder_get_frames` |  | `include_bytes` boolean, `max` integer, `session` integer, `since` integer, `timeout_ms` integer | Decoded protocol frames (Modbus, NMEA, AT, CAN, EtherCAT, ...) with checksum results. |
| `decoder_set` | yes | `clear` boolean, `mode`* string, `session` integer, `signals` boolean | Select the protocol decoder of a session. |
| `expect` | yes | `cursor` integer, `encoding` Encoding, `line_ending` LineEnding, `pattern`* string, `pattern_encoding` Encoding, `send` string, `session` integer, `timeout_ms` integer | Optionally send data, then wait until received data matches a regular expression. |
| `file_cancel` | yes | `session` integer | Cancel the running file transfer of a session (also one started in the window). |
| `file_receive` | yes | `name` string, `overwrite` boolean, `protocol`* TransferProtocolArg, `session` integer, `subdir` string | Download files from the device into the transfer directory (YMODEM/ZMODEM keep the device's file names, XMODEM uses `name`). |
| `file_send` | yes | `paths`* string[], `protocol`* TransferProtocolArg, `session` integer | Upload files to the device with XMODEM, XMODEM-1K, YMODEM, YMODEM-G or ZMODEM. |
| `get_signals` |  | `session` integer | Read the modem lines (DTR, RTS, CTS, DSR, DCD, RI) of a serial port. |
| `get_status` |  | `session` integer | Connection state, endpoint, byte counters, modem lines and logging state. |
| `list_ports` |  |  | List serial ports, sessions and saved profiles. |
| `log_start` | yes | `file_name` string, `format` string, `session` integer | Start logging received (and, except for raw logs, sent) data to a file in the log directory. |
| `log_status` |  | `session` integer | Current log file, format and size. |
| `log_stop` | yes | `session` integer | Stop logging and close the log file. |
| `macro_list` |  |  | List macros with their shortcuts and steps (in the editor syntax accepted by macro_save). |
| `macro_run` | yes | `name`* string, `session` integer | Run a macro in a session and wait until it finishes (expect steps fail on timeout). |
| `macro_save` | yes | `name`* string, `shortcut` string, `text`* string | Create or replace a macro from editor text. |
| `modbus_request` | yes | `framing` ModbusFraming, `request`* ModbusRequestSpec, `session` integer, `timeout_ms` integer, `unit`* integer | Send a Modbus request (RTU, ASCII or TCP framing built with CRC/LRC/MBAP) and wait for the decoded response. |
| `open_port` | yes | `connection` ConnectionSpec, `profile` string, `session` integer | Open a serial device, TCP or UDP connection (reopens if already open). |
| `port_auto_baud` | yes | `candidates` array, `listen_ms` integer, `session` integer | Experimental: find the baud rate of a device that is sending text by trying common rates; the port is reopened at the best one. |
| `port_flush` | yes | `session` integer | Discard data waiting in the serial driver's input and output buffers. |
| `preset_apply` | yes | `name`* string, `session` integer | Apply a preset's line settings to the session's serial port (reopens it). |
| `preset_list` |  |  | List serial presets (built-in and user defined). |
| `profile_delete` | yes | `name`* string | Delete a saved profile. |
| `profile_get` |  | `name`* string | Get a profile's settings (connection and terminal). |
| `profile_list` |  |  | List saved profiles. |
| `profile_save` | yes | `name`* string, `session` integer | Save the session's current connection and terminal settings as a profile. |
| `read` |  | `cursor` integer, `encoding` Encoding, `max_bytes` integer, `session` integer, `timeout_ms` integer | Read received data from a cursor; optionally wait for data to arrive. |
| `script_check` |  | `script`* string | Check a script for syntax errors without running it. |
| `script_run` | yes | `path` string, `script` string, `session` integer, `timeout_ms` integer, `vars` object | Run an automation script (send/expect/delay/set_dtr/if_match/goto/…; read gtkterm://docs/script-language) and wait for it. |
| `script_stop` | yes | `session` integer | Stop the script or macro running in a session. |
| `search_buffer` |  | `max_matches` integer, `pattern`* string, `session` integer | Search the retained received data with a regular expression. |
| `send` | yes | `char_delay_ms` integer, `data`* string, `encoding` Encoding, `line_ending` LineEnding, `session` integer | Send data to the device. |
| `send_break` | yes | `duration_ms` integer, `session` integer | Send a serial break condition. |
| `set_signals` | yes | `dtr` boolean, `rts` boolean, `session` integer | Set the DTR and/or RTS output lines of a serial port. |
| `signal_history` |  | `csv` boolean, `seconds` integer, `session` integer | Modem line history (DTR, RTS, CTS, DSR, DCD, RI) of the last seconds, polled every 50 ms; pulses shorter than that are recovered from the driver's counters. |
| `sunshine_clear` | yes | `id` string, `session` integer | Clear one Sunshine chart's samples, or remove all charts. |
| `sunshine_export_csv` |  | `id`* string, `session` integer | Export a Sunshine chart as CSV (timestamp, elapsed seconds, series, index, values). |
| `sunshine_export_png` |  | `dark` boolean, `height` integer, `id`* string, `session` integer, `width` integer | Render a Sunshine chart as a PNG image, to look at the data the way the user sees it. |
| `sunshine_get_series` |  | `id`* string, `last_n` integer, `last_seconds` number, `max_points` integer, `series` array, `session` integer | Get the samples of a Sunshine chart, downsampled for the context window. |
| `sunshine_list_series` |  | `session` integer | List Sunshine charts with per-series statistics (min/max/mean/last). |
| `sunshine_pause` | yes | `id`* string, `paused`* boolean, `session` integer | Pause or resume a Sunshine chart. |
| `sunshine_set_chart_type` | yes | `id`* string, `kind`* string, `session` integer | Choose the chart type (line, bar, pie, scatter) for a packet ID. |
<!-- tool-reference:end -->
