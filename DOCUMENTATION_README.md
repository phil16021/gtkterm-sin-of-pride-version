# GTKTerm Architecture Documentation

This directory contains comprehensive documentation of the GTKTerm codebase and its integrated Sunshine visualization system.

## Documentation Files

### 1. **ARCHITECTURE_OVERVIEW.md** (33 KB, 930 lines)
The complete architectural guide covering:

- **Project Overview**: Statistics, build system, language
- **Overall Architecture**: Visual diagram showing all major components
- **Core GTKTerm Components**: 
  - Application structure (gtkterm.c/h, gtkterm_window.c/h)
  - Terminal display system (gtkterm_terminal.c/h, gtkterm_dataview.c/h)
  - Data management (buffer, chunks, structures)
- **Port Abstraction Layer**: 
  - Abstract interface
  - Serial, TCP, UDP implementations
  - Port control and management
- **Sunshine Visualization System**: 
  - Complete data flow pipeline
  - Manager, Scanner, Parser components
  - Chart widgets (line, bar, pie, scatter)
  - Packet format specification
- **Protocol Decoder System**: AT, NMEA, Modbus, CAN, EtherCAT, RS232
- **Supporting Systems**: Logging, macros, configuration, file transfer, shortcuts
- **Key Patterns**: GObject model, signals, layered architecture, interfaces, data enrichment
- **Design Decisions & Trade-offs**
- **File Organization**: Complete directory structure
- **Build System**: Dependencies and build process
- **Integration Summary**: How components work together

**Best for**: Understanding the complete system architecture, component responsibilities, design patterns, and relationships.

---

### 2. **QUICK_REFERENCE.md** (7.5 KB, 275 lines)
A compact lookup guide featuring:

- **Component Quick Index**: Tables of all major components with files and purposes
- **Key Data Structures**: GtkTermDataChunk, GtkTermSunshineDataPacket, etc.
- **Signal Flow**: Simplified signal chain diagram
- **View Modes**: Terminal display formats (ASCII, HEX, BINARY, DECIMAL, MIXED)
- **Sunshine Packet Format**: With examples
- **Port Types & Features**: Serial (baud, bits, parity, flow control), TCP, UDP
- **Common Operations**: Visualization, toggling, clearing, port selection
- **Key Configuration**: Terminal and port settings
- **Build Dependencies**: All required libraries
- **File Locations**: Where to find source, headers, build output
- **Memory Model**: GObject pattern, buffer limits
- **Testing Virtual Serial Port**: How to test without hardware
- **Key Takeaways**: 8 core architectural principles

**Best for**: Quick lookups, finding specific files, remembering component names, checking configuration options, memory model details.

---

### 3. **DATA_FLOW_DIAGRAMS.md** (19 KB, 578 lines)
Detailed flow diagrams showing:

1. **Application Startup Flow**: Initialization sequence
2. **Serial Data Arrival & Distribution**: How data flows from port to components
3. **Sunshine Visualization Pipeline**: Complete parsing and charting flow
4. **Port Connection Sequence**: User click to data ready
5. **Terminal Display Update**: Data to screen
6. **Logging Pipeline**: Data to log file format
7. **Protocol Decoder Integration**: Auto-detection and decoding
8. **Macro Execution Flow**: Automation system
9. **File Transfer (XModem)**: CRC and block handling
10. **Configuration Save/Load**: Persistence system
11. **Incremental Scanning Strategy**: Optimization technique
12. **Chart Data Structure Evolution**: Raw bytes to visual chart
13. **Memory and Reference Counting**: GObject lifecycle
14. **Signal Connection Map**: Component signal links

Each diagram includes ASCII art and step-by-step explanations.

**Best for**: Understanding data flow, debugging issues, learning how systems interact, tracing execution paths.

---

## Documentation Statistics

| File | Size | Lines | Purpose |
|------|------|-------|---------|
| ARCHITECTURE_OVERVIEW.md | 33 KB | 930 | Complete system design |
| QUICK_REFERENCE.md | 7.5 KB | 275 | Lookup guide |
| DATA_FLOW_DIAGRAMS.md | 19 KB | 578 | Execution flows |
| **Total** | **59.5 KB** | **1,783** | **Comprehensive coverage** |

---

## How to Use This Documentation

### If you want to...

**Understand the overall system architecture**
→ Start with **ARCHITECTURE_OVERVIEW.md**, Section "Overall Architecture" and "Core GTKTerm Components"

**Find a specific component or file**
→ Use **QUICK_REFERENCE.md**, "Component Quick Index" tables

**Debug data flow issues**
→ Look at **DATA_FLOW_DIAGRAMS.md**, especially diagrams 2, 3, and 14

**Add a new feature**
→ Read **ARCHITECTURE_OVERVIEW.md** "Key Architectural Patterns" and relevant section (e.g., "Adding New Chart Types")

**Understand Sunshine visualization**
→ See **ARCHITECTURE_OVERVIEW.md** "Sunshine Visualization System" and **DATA_FLOW_DIAGRAMS.md** diagram 3

**Check memory management**
→ See **DATA_FLOW_DIAGRAMS.md** diagram 13 and **QUICK_REFERENCE.md** "Memory Model"

**Configure the application**
→ Use **QUICK_REFERENCE.md** "Key Configuration" and **ARCHITECTURE_OVERVIEW.md** "Supporting Systems"

**Test without hardware**
→ Follow **QUICK_REFERENCE.md** "Testing Virtual Serial Port"

---

## Key Concepts Quick Reference

### Core Components
- **GtkTermBuffer**: Central data storage with signal broadcasting
- **GtkTermTerminal**: VTE-based terminal display widget
- **GtkTermSerialPort**: Serial port implementation (RS232/RS485)
- **GtkTermSunshineManager**: Visualization coordinator

### Data Flow
Raw bytes → Buffer (chunks) → Terminal (display) + Logging (file) + Sunshine (visualization)

### Sunshine Pipeline
Buffer → Scanner → Parser → Manager → Charts

### Signals
Instead of direct calls, components communicate via GObject signals for loose coupling.

### Port Abstraction
All port types (serial, TCP, UDP) implement the same GtkTermPort interface.

### Design Patterns
- **GObject Model**: Type system, memory management, signals
- **Signal-Based Architecture**: Event-driven, decoupled components
- **Layered Architecture**: UI, Service, Hardware layers
- **Interface-Based Design**: Extensible through interfaces
- **Data Enrichment Pipeline**: Raw → Structured → Parsed → Visualized

---

## Documentation Conventions

### Diagrams
- Text-based ASCII art for universal viewing
- Arrows (↓, →) show flow direction
- Boxes show components or processes
- Indentation shows nesting/hierarchy

### Code References
- File names: `gtkterm_buffer.c/h` (source and header)
- Structures: `GtkTermDataChunk`, `term_config_t`
- Functions: `gtkterm_buffer_new()`
- Macros: `G_DEFINE_TYPE`, `G_DECLARE_FINAL_TYPE`

### Signal Names
- Format: "signal-name"
- Example: "serial-data-received"
- Connected from source to callback in destination

---

## For Developers

### When Making Changes
1. Check **QUICK_REFERENCE.md** for component files
2. Read **ARCHITECTURE_OVERVIEW.md** for affected subsystems
3. Review **DATA_FLOW_DIAGRAMS.md** to understand data flow impact
4. Consider signal connections in diagram 14

### When Adding Features
1. Identify which components are involved
2. Check if existing interfaces can be extended
3. Consider signal-based communication vs. direct calls
4. Verify memory management (GObject cleanup)
5. Update documentation as needed

### When Debugging
1. Use **DATA_FLOW_DIAGRAMS.md** to trace execution
2. Check **ARCHITECTURE_OVERVIEW.md** signal connections
3. Verify buffer management in diagram 13
4. Look for signal disconnection in dispose/finalize

---

## External References

### Related Files in Repository
- **CLAUDE.md**: Project-specific instructions for Claude
- **FIXES_APPLIED.md**: Bug fixes and improvements
- **README.md**: General project overview

### Documentation Tools Used
- Meson build system documentation
- GTK4 API reference
- GObject documentation
- Cairo graphics library

### Key Source Files for Deep Dives
- `src/gtkterm_window.c`: Main window implementation
- `src/gtkterm_buffer.c`: Buffer and signal handling
- `src/sunshine/manager.c`: Sunshine coordinator
- `src/sunshine/data_parser.c`: Packet parsing logic
- `include/sunshine/line_chart.h`: Chart widget interface

---

## Version Information

- **Documentation Date**: 2025-11-10
- **GTKTerm Version**: 1.99.1 (sin-of-pride version)
- **GTK Requirement**: >= 4.6.0
- **VTE Requirement**: >= 0.69.92

---

## Questions or Clarifications?

If you need clarification on any architectural aspect:

1. Check if it's covered in the "Key Takeaways" sections
2. Look for related diagrams in DATA_FLOW_DIAGRAMS.md
3. Search QUICK_REFERENCE.md tables
4. Read the full explanation in ARCHITECTURE_OVERVIEW.md

The documentation is designed to be self-contained and comprehensive for understanding the codebase without needing to read all 40+ source files individually.

---

**Happy exploring!**

