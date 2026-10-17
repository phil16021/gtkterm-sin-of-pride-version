# Hardware test checklist

Automated tests run on pseudo terminals, which cannot check line levels, parity, modem
lines or RS485 direction control. Run this list on real hardware before a release.

| # | Setup | Check | Pass |
|---|---|---|---|
| 1 | USB-UART with TX↔RX loopback | `--headless -p /dev/ttyUSB0`: typed text comes back; 1 MB file via stdin is identical (`cmp`) | ☐ |
| 2 | same, 921600 and 250000 baud (`-s`) | no corruption; `stty -F /dev/ttyUSB0 -a` shows the speed | ☐ |
| 3 | two adapters cross-wired, 7E1 vs 8N1 | mismatch shows garbage, matching settings are clean (parity really applied) | ☐ |
| 4 | RTS↔CTS, DTR↔DSR jumpers | MCP `set_signals` dtr/rts toggles show up in `get_signals` cts/dsr | ☐ |
| 5 | `-w rts` with CTS held low | sending blocks without freezing reception; resumes when CTS goes high | ☐ |
| 6 | RS485 adapter, `-w rs485 -x 1 -y 1` | logic analyser: RTS asserted only while transmitting, delays respected | ☐ |
| 7 | unplug the adapter while streaming | port state becomes error/closed, no crash; replug + SIGUSR1 reopens | ☐ |
| 8 | Sunshine firmware at 1 kHz | charts stay live > 10 minutes, CPU < 20 % | ☐ |
| 9 | Claude Code via `--mcp-stdio` | agent opens the port, reads a packet, summarises a series | ☐ |
