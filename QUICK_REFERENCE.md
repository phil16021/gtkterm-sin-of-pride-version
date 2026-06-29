# GTKTerm Architecture - Quick Reference Guide

## Component Quick Index

### Core GTKTerm Components

| Component | File | Purpose | Type |
|-----------|------|---------|------|
| **Application** | `gtkterm.c/h` | App lifecycle, CLI args | GtkApplication |
| **Main Window** | `gtkterm_window.c/h` | UI container, menus | GtkApplicationWindow |
| **Terminal** | `gtkterm_terminal.c/h` | VTE display widget | VteTerminal subclass |
| **Buffer** | `gtkterm_buffer.c/h` | Data storage, signals | GObject |
| **Data Chunk** | `gtkterm_data_chunk.c/h` | Metadata container | Struct |

### Port Layer (Communication)

| Component | File | Protocol |
|-----------|------|----------|
| **Port Interface** | `gtkterm_port.c/h` | Abstract (all types) |
| **Serial Port** | `gtkterm_serial_port.c/h` | RS232/RS485 |
| **TCP Port** | `gtkterm_tcp_port.c/h` | TCP client/server |
| **UDP Port** | `gtkterm_udp_port.c/h` | UDP sockets |
| **Port Control** | `gtkterm_port_control.c/h` | Management |

### Protocol Decoders

| Component | File | Protocol |
|-----------|------|----------|
| **Decoder Interface** | `gtkterm_protocol_decoder.c/h` | Abstract |
| **AT Decoder** | `gtkterm_at_decoder.c/h` | AT commands |
| **NMEA Decoder** | `gtkterm_nmea_decoder.c/h` | GPS (NMEA 0183) |
| **Modbus Decoder** | `gtkterm_modbus_decoder.c/h` | Modbus RTU |
| **Modbus TCP Decoder** | `gtkterm_modbus_tcp_decoder.c/h` | Modbus TCP |
| **CAN Decoder** | `gtkterm_can_decoder.c/h` | CAN bus |
| **EtherCAT Decoder** | `gtkterm_ethercat_decoder.c/h` | EtherCAT |
| **RS232 Decoder** | `gtkterm_rs232_decoder.c/h` | Generic serial |

### Sunshine Visualization System

| Component | File | Purpose |
|-----------|------|---------|
| **Manager** | `manager.c/h` | Coordinates visualization |
| **Scanner** | `scanner.c/h` | Finds packets in buffer |
| **Parser** | `data_parser.c/h` | Parses packet format |
| **Line Chart** | `line_chart.c/h` | Time-series visualization |
| **Bar Chart** | `bar_chart.c/h` | Category comparison |
| **Pie Chart** | `pie_chart.c/h` | Proportion visualization |
| **Scatter Chart** | `scatter_chart.c/h` | Relationship plots |

### Supporting Systems

| Component | File | Purpose |
|-----------|------|---------|
| **Logging** | `gtkterm_logging.c/h` | File persistence |
| **Macros** | `gtkterm_macro.c/h` | Command automation |
| **Configuration** | `gtkterm_configuration.c/h` | Settings storage |
| **File Transfer** | `gtkterm_file_transfer.c/h` | XModem protocol |
| **Data View** | `gtkterm_dataview.c/h` | Hex/ASCII display |
| **Shortcuts** | `gtkterm_shortcuts.c/h` | Keyboard bindings |
| **Toolbar** | `gtkterm_toolbar.c/h` | UI controls |
| **Statusbar** | `gtkterm_statusbar.c/h` | Status display |

---

## Key Data Structures

### GtkTermDataChunk (Metadata Container)
```c
struct {
  gint64 timestamp_us;        // When received (microseconds)
  GtkTermDataDirection dir;   // RX or TX
  guint8 *data;               // Raw bytes
  gsize length;               // Data length
}
```

### GtkTermSunshineDataPacket (Parsed Visualization Data)
```c
struct {
  gchar *id;                  // Packet ID ("P0", "P1", etc.)
  GList *series;              // List of data series
  guint64 timestamp_ms;       // When received
}
```

### GtkTermSunshineDataSeries (Single Series in Packet)
```c
struct {
  gchar *name;                // Series name
  GtkTermSunshineColor color; // RGB color
  GArray *values;             // Array of doubles
}
```

---

## Signal Flow

### Simplified Signal Chain

```
Serial Port
  ↓ "serial-data-received"
GtkTermBuffer
  ↓ "data-chunks-updated"
  ├─→ GtkTermTerminal (display)
  ├─→ GtkTermLogging (log file)
  └─→ GtkTermSunshineManager (visualization)
        ↓
      Scanner → Parser → Manager → Charts
```

---

## View Modes (Terminal Display)

- `ASCII` - Plain text
- `HEX` - Hexadecimal bytes
- `BINARY` - Binary digits
- `DECIMAL` - Decimal values
- `MIXED` - Hex + ASCII side-by-side
- `BINARY_MIXED` - Binary + ASCII
- `DECIMAL_MIXED` - Decimal + ASCII

---

## Sunshine Packet Format

```
{PacketID|Series1Name|R,G,B|Value1|Series2Name|R,G,B|Value2|...}

Example: {P0|Temperature|255,0,0|23.5|Humidity|0,255,0|45.2}
```

- First field: Packet ID
- Odd fields: Series name
- Even fields (optional): R,G,B color
- Last field: Numeric value

---

## Port Types & Features

### Serial Port
- Baud: 300-2,000,000 bps
- Bits: 5-8
- Stop bits: 1-2
- Parity: None, Odd, Even
- Flow control: None, Xon/Xoff, RTS/CTS, RS485
- Features: DTR/RTS control, break signal

### TCP Port
- Mode: Client or Server
- Client: Connect to remote host:port
- Server: Listen for incoming connections

### UDP Port
- Connectionless
- Send/receive datagrams

---

## Common Operations

### Starting Visualization
1. Data arrives at buffer
2. Manager listens to "data-chunks-updated" signal
3. Scanner scans for `{...}` patterns
4. Parser validates and extracts packet
5. Manager creates chart (if new ID) or updates existing
6. Chart renders with Cairo

### Toggling Visualization
- **Keyboard**: Ctrl+Shift+V
- **Menu**: View → Visualization
- **Action**: `on_gtkterm_toggle_visualization()`

### Clearing Charts
- Renew button on chart tab
- `gtkterm_sunshine_manager_clear_all()`

### Port Selection
1. Dropdown in header bar
2. Auto-scan for serial ports (gudev)
3. Manual TCP/UDP entry
4. Click "Connect" to open port

---

## Key Configuration

### Terminal Config (`term_config_t`)
- Echo, auto-CR, auto-LF
- Timestamp display
- View mode, hex display width
- Font, colors, cursor style
- Scrollback, rows, columns

### Port Config (`port_config_t`)
- Port name/address
- Baud rate, bits, stop bits
- Parity, flow control
- RS485 timing (if applicable)

---

## Build Dependencies

```
GTK4 >= 4.6.0          # GUI framework
VTE >= 0.69.92         # Terminal emulator
GLib >= 2.66           # Core utilities
gudev >= 230           # USB device enumeration
json-glib-1.0          # JSON parsing
GtkSourceView 5        # Syntax highlighting (optional)
libm                   # Math library
```

Build with: `meson setup build && ninja -C build`

---

## File Locations

- **Headers**: `include/` + `src/`
- **Sunshine headers**: `include/sunshine/`
- **Source files**: `src/`
- **Sunshine source**: `src/sunshine/`
- **Build output**: `build/`
- **Data resources**: `data/`

---

## Memory Model

### GObject Pattern
- **Type system**: G_DEFINE_TYPE, G_DECLARE_FINAL_TYPE
- **Encapsulation**: Private structs in .c files
- **Cleanup**: g_object_unref() with reference counting
- **Signal safety**: Auto-disconnects on finalize

### Buffer Limits
- **Max data chunks**: 10,000
- **Max line length**: Configurable (GString grows as needed)
- **Ring buffer**: Circular, configurable size

---

## Testing Virtual Serial Port

```bash
# Create virtual port pair
socat -d -d pty,raw,echo=0 pty,raw,echo=0
# Returns: /dev/pts/N and /dev/pts/M

# In one terminal (run gtkterm, connect to /dev/pts/N)
./build/src/gtkterm-sin-of-pride-version

# In another terminal, send test data
echo "{P0|Temperature|255,0,0|23.5}" > /dev/pts/M
```

---

## Key Takeaways

1. **Modular architecture** - Components loosely coupled via signals
2. **Dual data storage** - Ring buffer for display, chunks for metadata
3. **Optional Sunshine** - Visualization doesn't interfere with core terminal
4. **Abstracted ports** - Serial/TCP/UDP use same interface
5. **Extensible decoders** - Plugin architecture for protocols
6. **GObject foundation** - Type-safe, memory-managed, signal-based
7. **Data enrichment** - Each layer adds structure/metadata
8. **Incremental scanning** - Sunshine doesn't re-parse old data

