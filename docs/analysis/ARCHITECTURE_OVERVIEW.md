# GTKTerm "Sin of Pride" Version - Architecture & Organization Overview

## Project Overview

This is a modified version of **GTKTerm** (a serial terminal application) with an integrated **Sunshine** real-time data visualization subsystem. GTKTerm handles serial/TCP/UDP communication and terminal display, while Sunshine provides dynamic charting of embedded systems data.

### Key Statistics
- **Build System**: Meson
- **Language**: C
- **GUI Framework**: GTK4 (with VTE for terminal widget)
- **Total Components**: ~40+ source files organized in modular architecture
- **Core GTKTerm Files**: src/*.c (original GTKTerm components)
- **Sunshine Subsystem**: src/sunshine/*.c (custom data visualization)

---

## Overall Architecture

```
┌─────────────────────────────────────────────────────────────┐
│              GTKTerm Application (GtkApplication)           │
└──────────┬────────────────────────────────────────────────┬─┘
           │
           ├─────────────────────────────────────────────────────┐
           │                                                     │
           v                                                     v
    ┌─────────────────┐                           ┌──────────────────────┐
    │ GtkTermWindow   │                           │ GtkTermTerminal      │
    │ (Main Window)   │                           │ (VTE Widget)         │
    ├─────────────────┤                           ├──────────────────────┤
    │ - Header bar    │<──────────Signals─────>   │ - Input/Output       │
    │ - Terminal area │                           │ - Display formatting │
    │ - Toolbar       │                           │ - View modes (hex/   │
    │ - Paned split   │                           │   ASCII/binary)      │
    │   (terminal +   │                           └──────────────────────┘
    │    visualization)                                       │
    └─────────────────┘                                       │
           │                                                   │
           └──────────────────────┬──────────────────────────┬─┘
                                  │                          │
                    ┌─────────────v────────────┐  ┌──────────v─────────┐
                    │   GtkTermBuffer          │  │ Serial/TCP/UDP Port│
                    │ (Ring Buffer with Chunks)│  │                    │
                    ├──────────────────────────┤  ├────────────────────┤
                    │ - Stores GtkTermData     │  │ - Abstract Port    │
                    │   Chunk array (GPtrArray)│  │   Interface        │
                    │ - Emits signals on data  │  │ - Implements: RX/TX│
                    │   received               │  │ - Handles serial,  │
                    │ - Manages timestamp,     │  │   TCP, UDP         │
                    │   direction metadata     │  │ - Statistics       │
                    └──────────┬───────────────┘  └────────────────────┘
                               │
                               │ Signals
                               │ (serial-data-received)
                               │
                    ┌──────────v──────────────────────┐
                    │  Sunshine Visualization System  │
                    │  (Optional - can be toggled)    │
                    └──────────┬───────────────────────┘
                               │
                ┌──────────────┼──────────────┐
                │              │              │
         ┌──────v──────┐ ┌────v──────┐ ┌────v──────┐
         │ Scanner     │ │ Manager   │ │ Notebook  │
         │             │ │           │ │ Widget    │
         │ - Watches   │ │ - Routes  │ │           │
         │   buffer    │ │   packets │ │ - Contains│
         │ - Finds     │ │ - Creates │ │   chart   │
         │   packets   │ │   charts  │ │   tabs    │
         │ - Calls     │ │ - Manages │ └───────────┘
         │   callbacks │ │   chart   │
         └──────┬──────┘ │   hash    │
                │        │   table   │
                │        └─────┬─────┘
         ┌──────v──────────────┘
         │
      ┌──v───────────────────────────────┐
      │  Data Parser                      │
      ├───────────────────────────────────┤
      │ - Parses Sunshine packet format   │
      │   {ID|SeriesName|R,G,B|value|..} │
      │ - Produces GtkTermSunshine       │
      │   DataPacket structures          │
      │ - Handles color specs & values   │
      └───────────────────────────────────┘
         │
         └──> Chart Widgets
              ├─ Line Chart (gtkterm_sunshine_line_chart)
              ├─ Bar Chart (gtkterm_sunshine_bar_chart)
              ├─ Pie Chart (gtkterm_sunshine_pie_chart)
              └─ Scatter Chart (gtkterm_sunshine_scatter_chart)
```

---

## Core GTKTerm Components

### 1. **Main Application Structure**

#### **gtkterm.c / gtkterm.h**
- **Role**: Application entry point and GtkApplication object
- **Key Structs**: 
  - `GtkTerm` - Inherits from GTK_TYPE_APPLICATION
  - Manages command-line arguments
  - Handles application lifecycle (startup, activate, quit)
- **Signals**: `gtkterm_signals[]` - Custom application signals
- **Responsibilities**:
  - Create main application window
  - Parse command-line options
  - Handle application quit action
  - Memory cleanup on exit

#### **gtkterm_window.c / gtkterm_window.h**
- **Role**: Main application window (GtkApplicationWindow)
- **Key Structs**:
  - `GtkTermWindow` - Main window container
  - Contains all UI elements and managers
- **Key Components**:
  - **Header bar**: Port selection, baud rate, connect button, status
  - **Main paned**: Splits terminal and visualization areas
  - **Terminal area**: VTE terminal widget + scrollbar
  - **Visualization panel**: GtkNotebook with chart tabs
- **Signal Handlers**: 
  - Menu actions (File, Edit, View, Configure, Log, Control Signals)
  - Port connect/disconnect
  - View mode switching
  - Sunshine toggle (Ctrl+Shift+V)
- **Key Methods**:
  - `create_window()` - Constructs UI
  - `gtkterm_show_infobar()` - Displays messages
  - `gtkterm_window_update_statusbar()` - Updates status display
  - `on_gtkterm_toggle_visualization()` - Shows/hides Sunshine panel

### 2. **Terminal Display System**

#### **gtkterm_terminal.c / gtkterm_terminal.h**
- **Role**: VTE terminal widget for serial I/O display
- **Inherits from**: VteTerminal (GTK widget)
- **Configuration**:
  ```c
  typedef struct {
    bool block_cursor, show_cursor, echo, auto_lf, auto_cr;
    int delay, rows, columns, scrollback;
    GtkTermTerminalView view_mode;  // ASCII, HEX, BINARY, MIXED, etc.
    int hex_chars;
    bool visual_bell;
    GdkRGBA foreground_color, background_color;
    PangoFontDescription *font;
  } term_config_t;
  ```
- **View Modes**:
  - ASCII - Plain text
  - HEX - Hexadecimal bytes
  - BINARY - Binary representation
  - DECIMAL - Decimal values
  - MIXED - Hex + ASCII side-by-side
  - BINARY_MIXED - Binary + ASCII
  - DECIMAL_MIXED - Decimal + ASCII
- **Responsibilities**:
  - Display incoming serial data in selected format
  - Emit `vte-data-received` signals (connected to buffer)
  - Handle keyboard input and send to serial port
  - Apply terminal settings (echo, auto-CR/LF, timestamps)
  - Manage font, colors, and cursor

#### **gtkterm_dataview.c / gtkterm_dataview.h**
- **Role**: Custom data visualization widget (alternative to VTE)
- **Purpose**: Hex/ASCII data visualization for detailed inspection
- **Features**:
  - Hex/ASCII side-by-side display
  - Highlighting and search capabilities
  - Data offset indication
  - Custom rendering

### 3. **Data Management: Buffer & Chunks**

#### **gtkterm_buffer.c / gtkterm_buffer.h**
- **Role**: Central ring buffer for serial data storage and signal distribution
- **Inherits from**: G_TYPE_OBJECT (GObject)
- **Key Structure**:
  ```c
  struct _GtkTermBufferPrivate {
    char *buffer;                    // Ring buffer storage
    size_t tail;                     // Ring buffer position
    term_config_t *term_conf;        // Terminal config
    
    // Enhanced chunk-based storage
    GPtrArray *data_chunks;          // Array of GtkTermDataChunk
    gint64 session_start_time;       // Session timestamp (microseconds)
    
    GtkTermSerialPort *serial_port;
    GtkTermTerminal *terminal;
    GError *error;
  };
  ```
- **Design**: Hybrid approach
  - **Ring buffer**: Legacy text buffer for terminal display
  - **Data chunks**: New metadata-enriched storage for advanced features
- **Signals**:
  - Connected to `serial-data-received` (from port)
  - Connected to `vte-data-received` (from terminal)
  - Emits signals to Sunshine manager
- **Max Storage**: 10,000 data chunks
- **Methods**:
  - `gtkterm_buffer_get_data_chunks()` - Get all chunks
  - `gtkterm_buffer_clear_data_chunks()` - Clear storage
  - `gtkterm_buffer_get_session_start_time()` - Get session timestamp

#### **gtkterm_data_chunk.c / gtkterm_data_chunk.h**
- **Role**: Metadata-enriched data container
- **Structure**:
  ```c
  typedef struct {
    gint64 timestamp_us;              // Microsecond timestamp
    GtkTermDataDirection direction;   // RX or TX
    guint8 *data;                     // Raw bytes
    gsize length;                     // Data length
  } GtkTermDataChunk;
  
  typedef enum {
    GTKTERM_DATA_DIRECTION_RX = 0,    // Received
    GTKTERM_DATA_DIRECTION_TX = 1     // Transmitted
  } GtkTermDataDirection;
  ```
- **Methods**:
  - `gtkterm_data_chunk_new()` - Create with auto-timestamp
  - `gtkterm_data_chunk_free()` - Cleanup
  - `gtkterm_data_chunk_get_timestamp_string()` - Format timestamp
    - Supports: "absolute", "relative", "delta" formats
  - `gtkterm_data_chunk_get_current_time_us()` - Get current time

---

## Port Abstraction Layer

### **gtkterm_port.c / gtkterm_port.h** (Abstract Interface)
- **Pattern**: GObject interface (not implementation)
- **Types**:
  ```c
  typedef enum {
    GTKTERM_PORT_TYPE_SERIAL,      // RS232/RS485
    GTKTERM_PORT_TYPE_TCP_CLIENT,  // TCP client
    GTKTERM_PORT_TYPE_TCP_SERVER,  // TCP server
    GTKTERM_PORT_TYPE_UDP          // UDP socket
  } GtkTermPortType;
  
  typedef enum {
    GTKTERM_PORT_STATE_CLOSED,
    GTKTERM_PORT_STATE_OPENING,
    GTKTERM_PORT_STATE_OPEN,
    GTKTERM_PORT_STATE_CLOSING,
    GTKTERM_PORT_STATE_ERROR
  } GtkTermPortState;
  ```
- **Statistics**:
  ```c
  typedef struct {
    guint64 bytes_sent, bytes_received;
    guint64 frames_sent, frames_received;
    guint64 errors;
    GDateTime *connected_time;
  } GtkTermPortStats;
  ```
- **Virtual Methods**:
  - `get_name()` - Port identifier
  - `get_port_type()` - Port type
  - `get_state()` - Current state
  - `open()` / `close()` - Lifecycle
  - `write()` / `read()` - I/O operations
  - `get_stats()` / `reset_stats()` - Statistics
- **Signals**:
  - `data_received()` - New data available
  - `state_changed()` - Port state transition
  - `error_occurred()` - Error event

### **gtkterm_serial_port.c / gtkterm_serial_port.h**
- **Role**: Serial port implementation (RS232/RS485)
- **Implements**: GtkTermPort interface
- **Configuration**:
  ```c
  typedef struct {
    char *port;                    // /dev/ttyUSB0, etc.
    long int baudrate;             // 300-2000000 bps
    int bits;                      // 5-8 bits
    int stopbits;                  // 1-2
    GtkTermSerialPortParity parity;
    GtkTermSerialPortFlowControl flow_control;
    int rs485_rts_time_before_transmit;
    int rs485_rts_time_after_transmit;
    bool disable_port_lock;
  } port_config_t;
  ```
- **Features**:
  - RS485 half-duplex support
  - Flow control (none, Xon/Xoff, RTS/CTS)
  - Serial signal control (DTR, RTS toggle)
  - Break signal transmission
  - Baud rate up to 2Mbps

### **gtkterm_tcp_port.c / gtkterm_tcp_port.h**
- **Role**: TCP client/server implementation
- **Implements**: GtkTermPort interface
- **Modes**:
  ```c
  typedef enum {
    GTKTERM_TCP_MODE_CLIENT,       // Connect to server
    GTKTERM_TCP_MODE_SERVER        // Listen for connections
  } GtkTermTCPMode;
  ```
- **Methods**:
  - `gtkterm_tcp_port_new(mode, host, port)` - Create TCP port
  - `gtkterm_tcp_port_get_remote_address()` - Get remote info
  - `gtkterm_tcp_port_get_client_count()` - Server mode only

### **gtkterm_udp_port.c / gtkterm_udp_port.h**
- **Role**: UDP socket implementation
- **Implements**: GtkTermPort interface
- **Features**: Connectionless UDP communication

### **gtkterm_port_control.c / gtkterm_port_control.h**
- **Role**: High-level port management
- **Responsibilities**:
  - Enumerate available ports
  - Create appropriate port objects (serial/TCP/UDP)
  - Handle port lifecycle
  - Manage port configuration UI

---

## Sunshine Visualization System

### **Data Flow Pipeline**

```
Serial Data Stream
       ↓
   GtkTermBuffer (stores raw data + metadata in chunks)
       ↓
   Signals: "data-received"
       ↓
GtkTermSunshineManager (listens to signals)
       ↓
GtkTermSunshineScanner (scans for packets)
       ↓
Data line reconstruction (handles fragmented chunks)
       ↓
GtkTermSunshineParser (parses packet format)
       ↓
Validation & packet creation
       ↓
on_packet_found() callback
       ↓
GtkTermSunshineManager::route_packet_to_chart()
       ↓
Create chart (if new packet ID) or
Update existing chart
       ↓
Chart visualization (Cairo rendering)
```

### **Packet Format**

```
{PacketID|SeriesName|R,G,B|value|SeriesName2|R,G,B|value2|...}

Examples:
{P0|Temperature|255,0,0|23.5}
{P1|Roll|255,0,0|45.2|Pitch|0,255,0|12.3|Yaw|0,0,255|180.0}
{P2|Speed|0,255,0|87.3}  // Color optional
{P3|Altitude|value_only}
```

### **1. GtkTermSunshineManager (manager.c/h)**
- **Role**: Central coordinator for visualization
- **Inherits from**: G_TYPE_OBJECT
- **Structure**:
  ```c
  struct {
    GtkWidget *notebook;            // Tab container for charts
    GtkTermBuffer *buffer;          // Data source
    GtkTermSunshineScanner *scanner;
    GHashTable *charts;             // Maps packet ID -> chart widget
    guint last_scanned_chunk_index; // Track scan progress
    guint total_packets_processed;  // Statistics
    gulong buffer_signal_handler;   // Signal connection
  };
  ```
- **Key Methods**:
  - `gtkterm_sunshine_manager_new(notebook, buffer)` - Create manager
  - `gtkterm_sunshine_manager_scan_history()` - Scan existing data
  - `gtkterm_sunshine_manager_clear_all()` - Reset all charts
  - `gtkterm_sunshine_manager_reset()` - Reset from current point
  - `gtkterm_sunshine_manager_get_stats()` - Get packet/chart counts
- **Callbacks**:
  - `on_packet_found()` - Triggered by scanner
  - `on_buffer_data_received()` - Triggered by buffer signals
  - `on_renew_button_clicked()` - Clear chart data
- **Chart Creation Logic**:
  - Detects packet ID (first field)
  - Creates appropriate chart (default: line chart)
  - Adds tab to notebook
  - Stores in hash table for future references

### **2. GtkTermSunshineScanner (scanner.c/h)**
- **Role**: Scans buffer for Sunshine packets
- **Inherits from**: G_TYPE_OBJECT
- **Structure**:
  ```c
  struct {
    GtkTermSunshineParser *parser;
    GString *line_buffer;           // Reconstructs lines from chunks
    GtkTermSunshinePacketCallback packet_callback;
    gpointer callback_user_data;
    guint packets_found;
    guint lines_scanned;
  };
  ```
- **Key Methods**:
  - `gtkterm_sunshine_scanner_new()` - Create scanner
  - `gtkterm_sunshine_scanner_set_callback(callback, data)` - Set packet handler
  - `gtkterm_sunshine_scanner_scan_buffer(buffer)` - Scan all data
  - `gtkterm_sunshine_scanner_scan_from_index(buffer, index)` - Incremental scan
  - `gtkterm_sunshine_scanner_reset()` - Reset state
  - `gtkterm_sunshine_scanner_get_stats()` - Get counts
- **Scanning Strategy**:
  1. Iterates through all data chunks in buffer
  2. Reconstructs complete lines from chunk data
  3. Handles line breaks and fragment boundaries
  4. Passes each complete line to parser
  5. Invokes callback for each found packet

### **3. GtkTermSunshineParser (data_parser.c/h)**
- **Role**: Parses Sunshine packet format
- **Inherits from**: G_TYPE_OBJECT
- **Data Structures**:
  ```c
  typedef struct {
    guint8 r, g, b;
    gboolean has_color;
  } GtkTermSunshineColor;
  
  typedef struct {
    gchar *name;
    GtkTermSunshineColor color;
    GArray *values;  // Array of gdouble
  } GtkTermSunshineDataSeries;
  
  typedef struct {
    gchar *id;              // Packet ID like "P0"
    GList *series;          // List of GtkTermSunshineDataSeries*
    guint64 timestamp_ms;   // When received
  } GtkTermSunshineDataPacket;
  ```
- **Parser Structure**:
  ```c
  struct {
    guint64 packets_received;
    guint64 packets_dropped;
    GtkTermSunshineDataCallback data_callback;
    gpointer callback_user_data;
    GString *line_buffer;
  };
  ```
- **Key Methods**:
  - `gtkterm_sunshine_parser_new()` - Create parser
  - `gtkterm_sunshine_parser_parse_line(line, packet, error)` - Parse single line
  - `gtkterm_sunshine_parser_set_callback(callback, data)` - Set callback
  - `gtkterm_sunshine_parser_get_packets_received/dropped()` - Statistics
  - `gtkterm_sunshine_parser_reset_stats()` - Reset counters
- **Memory Management**:
  - `gtkterm_sunshine_packet_new/free()` - Packet lifecycle
  - `gtkterm_sunshine_data_series_new/free()` - Series lifecycle
- **Parsing Logic**:
  1. Check for `{` and `}` delimiters
  2. Extract packet ID (first field)
  3. Parse series entries: Name | Color (optional) | Value
  4. Validate numeric values
  5. Create packet structure with series list

### **4. Chart Widgets**

#### **Common Interface**
All charts inherit from `GtkWidget` and implement:
- `handle_packet()` - Accept new data
- `clear_all()` - Clear data
- `set_title()` - Set display title
- `add_series()` - Register new data series

#### **Line Chart (line_chart.c/h)**
- **Primary chart type for time-series data**
- **Rendering**: Cairo 2D graphics
- **Structure**:
  ```c
  struct {
    gchar *widget_id;               // Packet ID
    gchar *title;                   // Display title
    GList *series_list;             // Data series
    GtkTermSunshineChartSeries {
      gchar *name;
      GtkTermSunshineColor color;
      GArray *x_values, *y_values;
      gboolean visible;
    };
    gdouble x_min, x_max;
    gdouble y_min, y_max;
    guint64 time_counter;           // Auto-incrementing X for time series
  };
  ```
- **Capabilities**:
  - Multiple data series per chart
  - Auto-scaling axes
  - Configurable X/Y limits
  - Legend display
  - Grid and axes
  - Time-series support (auto-incrementing X)
  - Individual X,Y point plotting
- **Methods**:
  - `gtkterm_sunshine_line_chart_add_series(name, color)`
  - `gtkterm_sunshine_line_chart_add_data_point(series_name, x, y)`
  - `gtkterm_sunshine_line_chart_add_data_time_series(series_name, y)` - X auto-increment
  - `gtkterm_sunshine_line_chart_clear_series(series_name)`
  - `gtkterm_sunshine_line_chart_handle_packet(packet)` - Accept packet data
- **Drawing**:
  - Draws grid with 10 divisions
  - Plots data points and connects with lines
  - Color-coded by series
  - Includes legend

#### **Bar Chart (bar_chart.c/h)**
- **Purpose**: Category/comparison visualization
- **Similar interface** to line chart
- **Rendering**: Horizontal/vertical bars for each series

#### **Pie Chart (pie_chart.c/h)**
- **Purpose**: Proportion visualization
- **Shows**: Relative values as pie segments
- **Colors**: Series-specific colors

#### **Scatter Chart (scatter_chart.c/h)**
- **Purpose**: Relationship visualization
- **Renders**: Individual points without connecting lines
- **Useful for**: Discrete measurements, non-sequential data

---

## Protocol Decoder System

### **gtkterm_protocol_decoder.c/h** (Abstract Interface)
- **Pattern**: GObject interface for extensible protocol support
- **Supported Protocols**:
  - AT Commands (modem control)
  - NMEA 0183 (GPS/navigation)
  - Modbus RTU/TCP (industrial)
  - CAN bus
  - EtherCAT
  - RS232 (generic serial)
- **Virtual Methods**:
  - `get_name()` / `get_id()` - Protocol identification
  - `detect()` - Auto-detect protocol (confidence score 0-1)
  - `decode()` - Decode data chunk to human-readable format
  - `validate()` - Check data integrity
  - `get_stats()` - Return statistics string
  - `reset()` - Clear decoder state
- **Output Structure**:
  ```c
  typedef struct {
    char *protocol;                 // Protocol name
    char *summary;                  // One-line summary
    char *details;                  // Detailed breakdown
    gboolean valid;                 // Valid data?
    char *error;                    // Error message
    GHashTable *fields;             // Decoded fields (name->value)
  } GtkTermDecodedPacket;
  ```

### **Individual Decoders**
- **AT Decoder**: Command/response parsing
- **NMEA Decoder**: GPS sentence parsing ($GPGGA, $GPRMC, etc.)
- **Modbus Decoder**: Function codes, registers, checksums
- **Modbus TCP Decoder**: Network variant with TCP headers
- **CAN Decoder**: Frame IDs, DLC, payload analysis
- **EtherCAT Decoder**: Real-time industrial protocol
- **RS232 Decoder**: Generic serial analysis

---

## Supporting Systems

### **Logging System (gtkterm_logging.c/h)**
- **Role**: Session logging to file
- **Features**:
  - Timestamp inclusion
  - Direction marking (RX/TX)
  - Pause/resume capability
  - File rotation support
- **Connection**: Listens to buffer data-received signals

### **Macro System (gtkterm_macro.c/h)**
- **Role**: Automation through command macros
- **Features**:
  - Macro definition/editing
  - Scheduled execution
  - Variable substitution
  - Trigger conditions

### **File Transfer (gtkterm_file_transfer.c/h)**
- **Protocols**: XModem support
- **Features**:
  - CRC validation
  - Block retransmission
  - Progress tracking

### **Configuration System (gtkterm_configuration.c/h)**
- **Persistence**: Save/load port & terminal settings
- **Format**: Configuration files
- **Scope**: Per-session profiles

### **Shortcuts (gtkterm_shortcuts.c/h)**
- **Keyboard bindings**: Customizable shortcuts
- **Ctrl+Shift+V**: Toggle visualization
- **Ctrl+Q**: Quit
- (Others defined in menu)

### **Statusbar & Toolbar (gtkterm_statusbar.c/h, gtkterm_toolbar.c/h)**
- **Display**: Connection status, statistics
- **Controls**: Port selection, baud rate, connect button

---

## Data Flow: Complete Picture

### **1. Incoming Serial Data**

```
Serial Port Device (/dev/ttyUSB0, etc.)
        ↓
GtkTermSerialPort::read() in async handler
        ↓
Emit: "serial-data-received" signal
        ↓
GtkTermBuffer::on_add_data() callback
        ↓
├─→ Append to ring buffer (for terminal display)
├─→ Create GtkTermDataChunk with metadata
├─→ Add chunk to GPtrArray
├─→ Emit: "data-chunks-updated" signal
│
└─→ Connected listeners:
    ├─ GtkTermTerminal: Display formatted text
    ├─ GtkTermLogging: Write to log file
    └─ GtkTermSunshineManager: Scan for visualization packets
```

### **2. Sunshine Visualization Processing**

```
GtkTermSunshineManager::on_buffer_data_received()
        ↓
Call: gtkterm_sunshine_scanner_scan_from_index()
        ↓
Iterate chunks starting from last_scanned_chunk_index
        ↓
For each chunk:
  ├─ Convert raw bytes to text
  ├─ Accumulate in line_buffer
  ├─ When line complete (newline):
  │   └─ Call: gtkterm_sunshine_parser_parse_line()
  │       ├─ Check for { } delimiters
  │       ├─ Extract packet ID
  │       ├─ Parse series (Name|Color|Value)
  │       ├─ Validate data
  │       └─ Emit callback: on_packet_found()
  │
  └─ Update last_scanned_chunk_index
        ↓
on_packet_found() invoked for each packet
        ↓
route_packet_to_chart():
  ├─ Look up packet ID in charts hash table
  ├─ If not found:
  │   └─ Create new chart widget
  │       └─ Add tab to notebook
  │       └─ Store in hash table
  │
  └─ Call: chart_widget->handle_packet(packet)
      ├─ For each series:
      │   ├─ Find or create series in chart
      │   └─ Add data points
      │
      └─ Trigger redraw (queue_draw)
            ↓
      GtkSnapshot / cairo_t draw callback
            ↓
      Render chart (axes, grid, lines, legend)
```

### **3. Outgoing Data**

```
User presses key / sends command
        ↓
GtkTermTerminal::on_key_press()
        ↓
Call: gtkterm_serial_port_write()
        ↓
Write bytes to serial port
        ↓
(If echo enabled)
Emit: "serial-data-received" with TX data
        ↓
Same flow as incoming data but marked as TX
```

---

## Key Architectural Patterns

### **1. GObject Model**
- **Type System**: Uses G_DECLARE_FINAL_TYPE / G_DEFINE_TYPE macros
- **Encapsulation**: Private data in separate structs
- **Memory Management**: Reference counting, g_object_unref()
- **Signals**: Type-safe event notification
- **Properties**: Formal property system with GParamSpec

### **2. Signal-Based Architecture**
Instead of direct function calls, components communicate via signals:
- **Serial Port** → (signal) → **Buffer** → (signal) → **Terminal + Manager**
- **Decouples** dependencies
- **Allows** multiple listeners (logging, visualization, decoders)
- **Event-driven** design

### **3. Layered Architecture**
```
UI Layer
├─ GtkTermWindow
├─ GtkTermTerminal
├─ Chart Widgets
└─ Status/Toolbar

Service Layer
├─ GtkTermBuffer (data aggregation)
├─ GtkTermSunshineManager (coordination)
├─ GtkTermLogging (persistence)
└─ Protocol Decoders

Port/Hardware Layer
├─ GtkTermSerialPort
├─ GtkTermTCPPort
├─ GtkTermUDPPort
└─ GtkTermPort (interface)
```

### **4. Interface-Based Design**
- **GtkTermPort**: Abstract port interface with multiple implementations
- **GtkTermProtocolDecoder**: Abstract decoder interface with plugins
- **Allows** runtime selection and swapping

### **5. Data Enrichment Pipeline**
- **Raw bytes** → **Ring buffer** → **Typed data chunks** → **Parsed packets**
- **Each stage** adds metadata/structure
- **Enables** advanced features (logging with timestamps, visualization with direction)

---

## Key Design Decisions

### **1. Dual-Buffer System**
- **Ring buffer**: Fast display, legacy compatibility
- **Chunk array**: Metadata storage, advanced features
- **Trade-off**: Extra memory for better functionality

### **2. Sunshine Optional**
- Can be toggled on/off (Ctrl+Shift+V)
- Doesn't interfere with terminal operation
- Graceful degradation if no valid packets

### **3. Stateless Scanning**
- Scanner maintains `last_scanned_chunk_index`
- Avoids re-processing old data
- Supports incremental updates

### **4. Chart Type Auto-Detection**
- Default to line chart for all packets
- Could extend with heuristics (e.g., detect pie chart if values are proportions)

### **5. Port Abstraction**
- Support multiple protocols (serial/TCP/UDP)
- Unified interface
- Easy to add new types

---

## File Organization

### **Core Structure**
```
src/
├─ gtkterm.c/h                      # Application entry point
├─ gtkterm_window.c/h               # Main window
├─ gtkterm_terminal.c/h             # Terminal widget
├─ gtkterm_buffer.c/h               # Data buffer
├─ gtkterm_data_chunk.c/h           # Chunk structure
├─ gtkterm_port.c/h                 # Port interface
├─ gtkterm_serial_port.c/h          # Serial implementation
├─ gtkterm_tcp_port.c/h             # TCP implementation
├─ gtkterm_udp_port.c/h             # UDP implementation
├─ gtkterm_port_control.c/h         # Port management
├─ gtkterm_protocol_decoder.c/h     # Decoder interface
├─ gtkterm_*_decoder.c/h            # Protocol decoders (AT, NMEA, Modbus, CAN, EtherCAT)
├─ gtkterm_logging.c/h              # File logging
├─ gtkterm_macro.c/h                # Command macros
├─ gtkterm_file_transfer.c/h        # File transfer (XModem)
├─ gtkterm_configuration.c/h        # Settings persistence
├─ gtkterm_dataview.c/h             # Hex/ASCII viewer
├─ gtkterm_shortcuts.c/h            # Keyboard shortcuts
├─ gtkterm_toolbar.c/h              # Toolbar UI
├─ gtkterm_statusbar.c/h            # Status display
│
└─ sunshine/
   ├─ manager.c                     # Visualization coordinator
   ├─ scanner.c                     # Packet finder
   ├─ data_parser.c                 # Packet parser
   ├─ line_chart.c                  # Line chart widget
   ├─ bar_chart.c                   # Bar chart widget
   ├─ pie_chart.c                   # Pie chart widget
   └─ scatter_chart.c               # Scatter plot widget

include/
└─ sunshine/                         # Sunshine headers
   ├─ manager.h
   ├─ scanner.h
   ├─ data_parser.h
   ├─ line_chart.h
   ├─ bar_chart.h
   ├─ pie_chart.h
   └─ scatter_chart.h
```

---

## Build System (Meson)

### **Key Dependencies**
- **gtk4** (>= 4.6.0) - GUI framework
- **vte-2.91-gtk4** (>= 0.69.92) - Terminal emulator
- **glib-2.0** (>= 2.66) - Utilities, event system
- **gudev-1.0** (>= 230) - USB device enumeration
- **json-glib-1.0** - JSON parsing
- **gtksourceview-5** (>= 5.0.0, optional) - Syntax highlighting
- **libm** - Math library (for chart calculations)

### **Build Process**
```bash
meson setup build          # Configure
ninja -C build            # Compile
./build/src/gtkterm-sin-of-pride-version  # Run
```

---

## Integration Summary

### **How Components Work Together**

1. **User connects serial port** via UI
   - Port object created (serial/TCP/UDP)
   - Data signals connected

2. **Serial data arrives**
   - Port emits signal with raw bytes
   - Buffer receives and stores (ring buffer + chunks)
   - Terminal receives copy and displays formatted
   - Logging receives copy and writes to file
   - Sunshine manager receives signal

3. **Sunshine processes incoming data**
   - Scanner incremental scan from last position
   - Parser identifies valid packets
   - Manager routes to appropriate chart
   - Chart updates and redraws
   - User sees real-time visualization

4. **User can switch views**
   - Terminal display formats (ASCII/hex/binary/mixed)
   - Sunshine panel visibility (Ctrl+Shift+V)
   - Protocol decoder selection

5. **Session management**
   - Settings saved to configuration
   - Session logged if enabled
   - Charts and data persisted in memory

---

## Notable Features

### **Sunshine Innovation**
- Custom packet format `{ID|Series|Color|Value}`
- Real-time parsing and visualization
- Multiple chart types
- Color customization per series

### **Port Flexibility**
- Same UI for serial/TCP/UDP
- Seamless protocol switching
- Statistics tracking

### **Advanced Data View**
- Multiple display formats
- Timestamp tracking
- RX/TX direction marking
- Protocol-specific decoding

### **Extensibility**
- Protocol decoder plugin interface
- New chart types easily added
- Custom macro system

---

## Conclusion

GTKTerm "Sin of Pride" is a well-architected embedded systems serial tool with a custom visualization layer. The combination of:

- **Modular component design** (buffer, ports, terminal, logging)
- **Signal-based communication** (loose coupling)
- **Interface abstraction** (ports, decoders, charts)
- **Metadata enrichment** (timestamps, direction, structured packets)

...creates a flexible, extensible platform for serial communication and real-time data visualization. The Sunshine subsystem demonstrates thoughtful integration without disrupting core GTKTerm functionality.
