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
| 10 | RS485 adapter + real Modbus RTU slave, 19200 8E1 | `modbus_request` reads registers; frames pane shows TX/RX with CRC ✓; unplugging the slave gives timeouts, no false frames | ☐ |
| 11 | GPS module on a UART | *Auto-detect* picks NMEA within a second; positions match the module's datasheet format | ☐ |
| 12 | RTS↔CTS jumper, *Record Modem Line Changes* | toggling RTS (F8) produces RS232 frames with `Changed: RTS,CTS` | ☐ |
| 13 | Board with an XMODEM/YMODEM bootloader (e.g. U-Boot `loady`, STM32 IAP) at 115200 | firmware image uploads, bootloader reports the right size/CRC; cancel mid-transfer leaves the bootloader responsive | ☐ |
| 14 | Linux target with lrzsz over a USB-UART, 9600 and 921600 baud | ZMODEM `sz`/`rz` both ways, 10 MB file, `cmp` clean; unplugging mid-transfer ends with an error, not a hang | ☐ |
