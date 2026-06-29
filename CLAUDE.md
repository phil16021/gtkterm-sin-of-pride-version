# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a modified version of GTKTerm (a serial terminal application) with a custom "Sunshine" data visualization feature added. The base GTKTerm code handles serial/TCP/UDP communication, while the Sunshine subsystem provides real-time charting capabilities for embedded systems data.

## Build System

This project uses Meson for building:

```bash
# Configure build
meson setup build

# Compile
ninja -C build

# Run without installing
./build/src/gtkterm-sin-of-pride-version

# Install system-wide
sudo ninja -C build install
```

## Dependencies

- GTK4 (>= 4.6.0)
- VTE (>= 0.69.92) - terminal widget
- GLib 2.0 (>= 2.66)
- gudev-1.0 (>= 230)
- json-glib-1.0 (required)
- GtkSourceView 5 (>= 5.0.0, optional - for syntax highlighting)
- libm (math library)

## Architecture

### Core GTKTerm Components

The original GTKTerm application structure:

- **gtkterm_window.c**: Main application window using GtkApplicationWindow
- **gtkterm_terminal.c**: VTE terminal handling for serial I/O display
- **gtkterm_buffer.c**: Ring buffer for serial data with chunk-based storage (`GtkTermDataChunk`)
- **gtkterm_serial_port.c / gtkterm_tcp_port.c / gtkterm_udp_port.c**: Port abstraction layer inheriting from `gtkterm_port.c`
- **gtkterm_protocol_decoder.c**: Base class for protocol decoders (AT, Modbus, NMEA, CAN, EtherCAT, RS232)
- **gtkterm_dataview.c**: Custom widget for hex/ASCII data visualization
- **gtkterm_macro.c**: Macro system for automating commands
- **gtkterm_logging.c**: Session logging functionality
- **gtkterm_file_transfer.c / gtkterm_xmodem.c**: File transfer protocols

### Sunshine Visualization System

Custom addition located in `src/sunshine/` with headers in `include/sunshine/`:

**Data Flow**: Serial data → Buffer → Scanner → Parser → Manager → Charts

1. **data_parser.c**: Parses Sunshine packet format
   - Format: `{PacketID|SeriesName|R,G,B|value|SeriesName2|R,G,B|value2|...}`
   - Color specification (R,G,B) is optional
   - Produces `GtkTermSunshineDataPacket` structures containing multiple `GtkTermSunshineDataSeries`

2. **scanner.c**: Scans `GtkTermBuffer` for Sunshine packets
   - Processes data chunks from the buffer
   - Invokes parser when potential packets are found

3. **manager.c**: Central coordinator for all visualization
   - Connects to `GtkTermBuffer` signals to receive new data
   - Routes parsed packets to appropriate charts by packet ID
   - Manages a GtkNotebook containing chart tabs
   - Uses GHashTable to map packet IDs to chart widgets

4. **Chart widgets** (line_chart.c, bar_chart.c, pie_chart.c, scatter_chart.c):
   - Custom GTK widgets using Cairo for rendering
   - Each implements a "Renew" button to clear data
   - Auto-scaling based on data ranges

**Integration Point**: In `gtkterm_window.c`, the main window contains:
- `main_paned` (GtkPaned): Splits terminal and visualization panel
- `visualization_panel` (GtkNotebook): Container for charts
- `sunshine_manager` (GtkTermSunshineManager): Manages the visualization subsystem
- Keyboard shortcut Ctrl+Shift+V toggles visualization panel visibility

### GObject Patterns

This codebase uses GObject heavily:

- Most components are GObject subclasses (e.g., `G_DEFINE_TYPE` macros)
- Private data stored in separate structs (e.g., `GtkTermBufferPrivate`)
- Signal-based communication between components
- Reference counting for memory management

## Common Operations

### Testing Sunshine Visualization

To test without hardware, use virtual serial ports:
- Supports `/dev/pts/*` virtual terminals
- Send test packets: `{P0|Temperature|255,0,0|23.5}`
- Multiple series in one packet: `{P1|Roll|255,0,0|45.2|Pitch|0,255,0|12.3|Yaw|0,0,255|180.0}`

### Adding New Chart Types

1. Create new files in `src/sunshine/` and `include/sunshine/`
2. Follow existing chart widget patterns (inherit from GtkWidget)
3. Implement Cairo drawing code in draw callback
4. Register in `manager.c`'s `create_chart_for_packet()` function
5. Add source file to `src/meson.build`

### Working with Serial Data

Serial data flows through several layers:
1. Port layer receives raw data (serial_port/tcp_port/udp_port)
2. `GtkTermBuffer` stores data as chunks (`GtkTermDataChunk`)
3. Buffer emits signals that both terminal and Sunshine manager listen to
4. Each chunk includes timestamp, direction (RX/TX), and raw bytes

## Code Style Notes

- Mix of professional GTKTerm code and student project code (Sunshine subsystem)
- Sunshine components include informal comments ("So basically I took GTKTerm...")
- Both use GObject patterns but Sunshine code is less mature
- Header guards follow pattern: `GTKTERM_<COMPONENT>_H`
