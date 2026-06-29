# GTKTerm Data Flow Diagrams

## 1. Application Startup Flow

```
main()
  ↓
g_application_run(GTK_APPLICATION(app), argc, argv)
  ↓
gtkterm_startup()
  ├─ Load UI from GResource
  ├─ Parse command-line options
  └─ Create option groups
  ↓
gtkterm_activate()
  ↓
create_window(app, window_data)
  ├─ Create GtkTermWindow
  ├─ Build header bar (port, baud, connect button)
  ├─ Create main paned split
  │  ├─ Left: Terminal widget (GtkTermTerminal)
  │  └─ Right: Visualization notebook (GtkNotebook)
  ├─ Create status bar
  ├─ Create GtkTermBuffer
  ├─ Create port object (serial/TCP/UDP)
  ├─ Connect signals
  └─ Show window
  ↓
gtk_window_present(window)
```

---

## 2. Serial Data Arrival & Distribution

```
┌─────────────────────────────────────────────────────────────┐
│ Serial Port Device (/dev/ttyUSB0)                           │
└────────────────────────┬────────────────────────────────────┘
                         │ Raw bytes arrive
                         ↓
┌─────────────────────────────────────────────────────────────┐
│ GtkTermSerialPort (async read thread)                       │
│ - Read from file descriptor                                 │
│ - Emit "serial-data-received" signal                        │
└────────────────────────┬────────────────────────────────────┘
                         │ Signal with raw bytes
                         ↓
┌─────────────────────────────────────────────────────────────┐
│ GtkTermBuffer::on_add_data() callback                       │
│ - Append to ring buffer                                     │
│ - Create GtkTermDataChunk                                   │
│ - Add chunk to GPtrArray                                    │
│ - Emit "data-chunks-updated" signal                         │
└─────────────────────────────────────────────────────────────┘
            │                      │                    │
            ↓                      ↓                    ↓
    ┌───────────────┐    ┌──────────────┐    ┌─────────────────┐
    │  Terminal     │    │   Logging    │    │  Sunshine       │
    │  Widget       │    │  Manager     │    │  Manager        │
    ├───────────────┤    ├──────────────┤    ├─────────────────┤
    │ - Format text │    │ - Write to   │    │ - Scan for      │
    │ - Apply view  │    │   log file   │    │   packets       │
    │   mode (hex/  │    │ - Add        │    │ - Parse         │
    │   ASCII)      │    │   timestamps │    │   visualization │
    │ - Display on  │    │ - Mark RX/TX │    │   data          │
    │   screen      │    │              │    │ - Update charts │
    └───────────────┘    └──────────────┘    └─────────────────┘
```

---

## 3. Sunshine Visualization Data Pipeline

```
┌──────────────────────────────────┐
│ GtkTermSunshineManager           │
│ Listens to buffer signals        │
└────────────┬─────────────────────┘
             │
             ↓
┌──────────────────────────────────────────────────────────┐
│ on_buffer_data_received()                                │
│ Triggered by GtkTermBuffer::data-chunks-updated signal  │
└────────────┬─────────────────────────────────────────────┘
             │
             ↓
┌──────────────────────────────────────────────────────────┐
│ Call gtkterm_sunshine_scanner_scan_from_index()         │
│ Starting from last_scanned_chunk_index                  │
└────────────┬─────────────────────────────────────────────┘
             │
             ↓
    ┌────────────────────┐
    │ For each chunk:    │
    │                    │
    │ 1. Convert bytes   │
    │    to text         │
    │ 2. Accumulate in   │
    │    line_buffer     │
    │ 3. Check for \n    │
    └─────────┬──────────┘
              │
              ↓ Line complete
    ┌─────────────────────────────────────────┐
    │ Call gtkterm_sunshine_parser_parse_line()│
    │ for {ID|Series|Color|Value|...}        │
    └──────────────┬────────────────────────┘
                   │
         ┌─────────┴─────────┐
         ↓                   ↓
    ┌─────────┐         ┌──────────┐
    │ Valid?  │         │ Invalid  │
    │         │         │          │
    │ YES     │         │ Discard  │
    └────┬────┘         └──────────┘
         │
         ↓
    ┌──────────────────────────────┐
    │ Create GtkTermSunshineData   │
    │ Packet structure:            │
    │ - id (e.g., "P0")            │
    │ - series list (names,        │
    │   colors, values)            │
    │ - timestamp_ms               │
    └──────────┬───────────────────┘
               │
               ↓
    ┌──────────────────────────────┐
    │ Emit callback:               │
    │ on_packet_found(packet)      │
    └──────────┬───────────────────┘
               │
               ↓
    ┌───────────────────────────────────────┐
    │ route_packet_to_chart()               │
    │                                       │
    │ 1. Extract packet ID from first field│
    │ 2. Look up in charts hash table      │
    └─────────────┬───────────────────────┘
                  │
        ┌─────────┴─────────┐
        ↓                   ↓
    ┌─────────┐        ┌──────────────┐
    │ Found?  │        │ Create new   │
    │ YES     │        │ chart widget │
    │ Update  │        │ (line chart) │
    │ existing│        └────┬─────────┘
    └────┬────┘             │
         │                  ↓
         │         ┌─────────────────────┐
         │         │ Add to notebook     │
         │         │ Add to hash table   │
         │         │ Store reference    │
         │         └────┬────────────────┘
         │              │
         └──────┬───────┘
                │
                ↓
        ┌────────────────────────────────┐
        │ For each series in packet:     │
        │                                │
        │ - Find/create series in chart │
        │ - Add data points (x,y)        │
        │ - Set color                    │
        └────────────┬───────────────────┘
                     │
                     ↓
            ┌───────────────────┐
            │ gtk_widget_queue  │
            │ _draw()           │
            │ (trigger redraw)  │
            └────────┬──────────┘
                     │
                     ↓
            ┌───────────────────────────────┐
            │ GtkTermSunshineLineChart      │
            │ ::snapshot()                  │
            │                               │
            │ Render with Cairo:            │
            │ - Draw axes & grid            │
            │ - Plot data points            │
            │ - Draw connecting lines       │
            │ - Color by series             │
            │ - Draw legend                 │
            │ - Update on-screen            │
            └───────────────────────────────┘
```

---

## 4. Port Connection Sequence

```
User clicks "Connect" button
     ↓
on_connect_button_clicked()
     ↓
Get selected port (serial/TCP/UDP)
     ↓
Get configuration (baud, bits, etc.)
     ↓
Create appropriate port object:
├─ GtkTermSerialPort (serial)
├─ GtkTermTCPPort (TCP client/server)
└─ GtkTermUDPPort (UDP)
     ↓
gtkterm_port_open(port, error)
     ↓
┌─ Serial: open /dev/ttyUSB0, set termios
├─ TCP: create socket, bind/listen or connect
└─ UDP: create socket, bind
     ↓
Set port state: OPENING → OPEN
     ↓
Emit "state_changed" signal
     ↓
UI updates (button text: "Connected")
     ↓
Start async read thread
     ↓
Ready to receive data
```

---

## 5. Terminal Display Update

```
GtkTermBuffer receives data
     ↓
Append to ring buffer
     ↓
Emit "data-chunks-updated"
     ↓
GtkTermTerminal callback
     ↓
Get view mode setting:
├─ ASCII: Pass through as-is
├─ HEX: Convert bytes to "AA BB CC"
├─ BINARY: Convert to "10101010 10111011 ..."
├─ DECIMAL: Convert to "170 187 204 ..."
└─ MIXED: Hex on left, ASCII on right
     ↓
Apply formatting:
├─ Add timestamp (if enabled)
├─ Add RX/TX marker (if enabled)
└─ Add index number (if enabled)
     ↓
Write to VTE terminal widget
     ↓
VTE scrolls and displays text
     ↓
User sees formatted data on screen
```

---

## 6. Logging Pipeline

```
GtkTermBuffer emits "data-chunks-updated"
     ↓
GtkTermLogging callback
     ↓
Check if logging is active
     ↓
For each chunk:
├─ Get timestamp
├─ Get direction (RX/TX)
├─ Convert data to displayable format
└─ Write to log file with metadata
     ↓
File format example:
┌────────────────────────────────────────┐
│ [RX] 2025-11-10 13:45:23.456           │
│ 48 65 6C 6C 6F 20 57 6F 72 6C 64       │
│ Hello World                             │
│                                        │
│ [TX] 2025-11-10 13:45:24.789           │
│ 4F 4B                                  │
│ OK                                     │
└────────────────────────────────────────┘
```

---

## 7. Protocol Decoder Integration

```
GtkTermBuffer receives data
     ↓
For each protocol decoder registered:
     ↓
Call: gtkterm_protocol_decoder_detect(data)
Returns: Confidence score (0.0 - 1.0)
     ↓
Select decoder with highest confidence
     ↓
Call: gtkterm_protocol_decoder_decode(chunk)
Returns: GtkTermDecodedPacket
     ├─ protocol name
     ├─ summary string
     ├─ detailed breakdown
     ├─ validity flag
     ├─ error message (if invalid)
     └─ fields hash table
     ↓
Display decoded information
(e.g., in side panel or popover)
```

---

## 8. Macro Execution Flow

```
User triggers macro (via menu or hotkey)
     ↓
GtkTermMacroManager finds macro definition
     ↓
For each command in macro:
     ├─ Substitute variables
     ├─ Apply delays
     └─ Send via serial port
         ↓
         Emit "serial-data-received" (with TX data)
         ↓
         Buffer processes (same as incoming)
         ↓
         Terminal displays with [TX] marker
         ↓
         Logging records to file
         ↓
         Sunshine scans (if valid format)
```

---

## 9. File Transfer (XModem) Sequence

```
User initiates XModem transfer
     ↓
GtkTermFileTransfer manager starts
     ↓
Send initial handshake (NAK)
     ↓
For each block:
├─ Send SOH + block number + data (128 bytes)
├─ Calculate CRC
├─ Wait for ACK/NAK response
├─ Retransmit on NAK
└─ Timeout handling
     ↓
Receive ACK for last block
     ↓
Send EOT (end of transmission)
     ↓
Transfer complete
     ↓
Update progress dialog
```

---

## 10. Configuration Save/Load

```
User selects "Save Config"
     ↓
GtkTermConfiguration manager
     ↓
Collect settings:
├─ Port configuration (serial, TCP, UDP)
├─ Terminal settings (echo, auto-LF, view mode)
├─ Font and colors
├─ Window size and position
└─ Macro definitions
     ↓
Serialize to JSON or config format
     ↓
Write to ~/.config/gtkterm/ or similar
     ↓
User selects "Load Config"
     ↓
Read config file
     ↓
Parse settings
     ↓
Apply to current session:
├─ Update port settings
├─ Update terminal view
├─ Restore window geometry
└─ Load macros
```

---

## 11. Incremental Scanning Strategy

```
Sunshine Manager initialized
     ↓
last_scanned_chunk_index = 0
     ↓
Buffer grows with new data:
├─ Chunk 0: bytes 0-99
├─ Chunk 1: bytes 100-199
├─ Chunk 2: bytes 200-299
└─ ...
     ↓
On signal "data-chunks-updated":
     ↓
Call: scanner_scan_from_index(buffer, last_scanned_chunk_index)
     ↓
Loop from chunk index to current end:
├─ Process chunk N
├─ Reconstruct lines
├─ Parse packets
└─ Update charts
     ↓
last_scanned_chunk_index = current_end
     ↓
Next signal only processes new chunks
(avoids re-parsing old data)
```

---

## 12. Chart Data Structure Evolution

```
Raw serial bytes:
{P0|Temperature|255,0,0|23.5}

     ↓ Parser processes

GtkTermSunshineDataPacket:
{
  id: "P0",
  series: [
    {
      name: "Temperature",
      color: {r: 255, g: 0, b: 0},
      values: [23.5]
    }
  ],
  timestamp_ms: 1234567890
}

     ↓ Manager routes to chart

GtkTermSunshineLineChart:
{
  widget_id: "P0",
  title: "P0",
  series_list: [
    {
      name: "Temperature",
      color: {r: 255, g: 0, b: 0},
      x_values: [0],
      y_values: [23.5],
      visible: TRUE
    }
  ],
  x_min: 0, x_max: 1,
  y_min: 23.5, y_max: 23.5
}

     ↓ Next packet: {P0|Temperature|255,0,0|24.1}

Updated GtkTermSunshineLineChart:
{
  series_list: [
    {
      name: "Temperature",
      color: {r: 255, g: 0, b: 0},
      x_values: [0, 1],
      y_values: [23.5, 24.1],
      visible: TRUE
    }
  ],
  x_min: 0, x_max: 2,
  y_min: 23.5, y_max: 24.1
}

     ↓ Render with Cairo

Visual line chart with two points and connecting line
```

---

## 13. Memory and Reference Counting

```
GObject Reference Counting Model:

gtkterm_buffer_new()
g_object_ref_count = 1

gtkterm_terminal_new(port, buffer)
g_object_ref(buffer)  ← Terminal holds reference
g_object_ref_count = 2

gtkterm_sunshine_manager_new(notebook, buffer)
g_object_ref(buffer)  ← Manager holds reference
g_object_ref_count = 3

When terminal destroys:
g_object_unref(buffer)
g_object_ref_count = 2

When manager disposes:
g_signal_handler_disconnect(buffer, handler_id)
g_object_unref(buffer)
g_object_ref_count = 1

When window destroys:
g_object_unref(buffer)  ← Final reference
g_object_ref_count = 0
     ↓
Object finalized:
├─ Free private data
├─ Disconnect all signals
├─ Free allocated memory
└─ Destroy object
```

---

## 14. Signal Connection Map

```
┌──────────────────────────────────┐
│ GtkTermSerialPort                │
├──────────────────────────────────┤
│ Signals:                         │
│ - serial-data-received(data, len)│
│ - state-changed(old, new)        │
│ - error-occurred(message)        │
└────────┬─────────────────────────┘
         │
         ├──→ GtkTermBuffer
         │    on_add_data()
         │
         └──→ Window
              on_port_status_changed()

┌──────────────────────────────────┐
│ GtkTermBuffer                    │
├──────────────────────────────────┤
│ Signals:                         │
│ - data-chunks-updated()          │
└────────┬────────────────────────┘
         │
         ├──→ GtkTermTerminal
         │    display_data()
         │
         ├──→ GtkTermLogging
         │    write_to_log()
         │
         └──→ GtkTermSunshineManager
              on_buffer_data_received()

┌──────────────────────────────────┐
│ GtkTermSunshineScanner           │
├──────────────────────────────────┤
│ Callback:                        │
│ on_packet_found(packet)          │
└────────┬─────────────────────────┘
         │
         └──→ GtkTermSunshineManager
              on_packet_found()
              route_packet_to_chart()
```

