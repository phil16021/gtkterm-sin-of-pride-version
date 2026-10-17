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
| `sunshine_set_chart_type`, `sunshine_pause`, `sunshine_clear`, `sunshine_export_csv` | chart control |
| `log_start`, `log_stop`, `log_status` | session logs (raw/text/csv/json/xml) inside the log directory |

Write tools are hidden in read-only mode: open/close, send, expect, break, signals, clearing,
chart changes and logging.

**Cursors.** Every received byte has a position in the session's RX stream. `send` returns the
position at send time, so `read`/`expect` from there see exactly the reply. Keep passing
`next_cursor` to read a stream without gaps; `lost` reports bytes evicted from history.

## Resources and prompts

Resources: `gtkterm://ports`, `gtkterm://profiles`, `gtkterm://docs/sunshine-protocol`,
`gtkterm://session/{id}/status`, `.../rx` (last 64 KiB), `.../sunshine`,
`.../sunshine/{packet_id}` (CSV).

Prompts: `debug-serial-device`, `analyze-sunshine-data`, `modbus-scan`, `at-modem-check`,
`write-sunshine-firmware-snippet`.

## Safety

Agents writing to real hardware can have physical effects. Use `--mcp-read-only` when an agent
should only observe. Log files can only be created inside the log directory
(`~/.local/share/gtkterm-sunshine/logs`); file names with path components are rejected.
