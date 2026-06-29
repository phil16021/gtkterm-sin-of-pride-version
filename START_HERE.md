# GTKTerm Architecture Documentation - START HERE

Welcome! This guide will help you navigate the comprehensive architecture documentation for the GTKTerm "Sin of Pride" version with integrated Sunshine visualization system.

## What You'll Find

This repository contains a full GTKTerm implementation (40+ source files) with:
- **Terminal Emulator**: Serial/TCP/UDP communication with VTE
- **Data Visualization**: Real-time charting system (Sunshine)
- **Protocol Support**: AT, NMEA, Modbus, CAN, EtherCAT decoders
- **Advanced Features**: Logging, macros, file transfer, configuration

## Documentation Files

### 1. **DOCUMENTATION_README.md** (Start here for overview)
The master index explaining:
- What's in each document
- How to navigate the docs
- When to use which file
- Key concepts summary

**Best for**: Understanding what documentation exists and which file to read

### 2. **ARCHITECTURE_OVERVIEW.md** (Complete reference)
The comprehensive guide (930 lines) covering:
- System architecture and component design
- 20+ major components with responsibilities
- Data structures and interfaces
- Port abstraction (Serial/TCP/UDP)
- Sunshine visualization system
- Protocol decoders
- Design patterns and principles

**Best for**: Deep understanding of system design

### 3. **QUICK_REFERENCE.md** (Fast lookup)
Quick index and tables (275 lines) for:
- All 30+ components with file names
- Data structures (C structs)
- Configuration options
- Build dependencies
- File locations
- Testing without hardware

**Best for**: Finding specific components, filenames, or quick facts

### 4. **DATA_FLOW_DIAGRAMS.md** (Execution flows)
Detailed diagrams (578 lines) showing:
- 14 different data flows and sequences
- Application startup
- Serial data processing
- Sunshine visualization pipeline
- Signal connections
- Memory management

**Best for**: Understanding how data moves through the system

---

## Quick Navigation

### If you want to understand...

**The overall system**
```
1. Read: DOCUMENTATION_README.md (5 min)
2. Look at: ARCHITECTURE_OVERVIEW.md → "Overall Architecture" diagram (10 min)
3. Review: Key Concepts section in DOCUMENTATION_README.md (5 min)
```

**How data flows**
```
1. Check: DATA_FLOW_DIAGRAMS.md → Diagram 2 "Serial Data Arrival" (10 min)
2. Follow: Diagram 3 "Sunshine Pipeline" (10 min)
3. Review: Signal map in Diagram 14 (5 min)
```

**Where to find a specific component**
```
1. Open: QUICK_REFERENCE.md
2. Search: Component Quick Index tables
3. Get: File name, purpose, type (1 min)
```

**How to add a new chart type**
```
1. Read: ARCHITECTURE_OVERVIEW.md → "Adding New Chart Types" (10 min)
2. Study: QUICK_REFERENCE.md → Line chart interface (5 min)
3. Check: DATA_FLOW_DIAGRAMS.md → Diagram 3 (10 min)
4. Examine: src/sunshine/line_chart.c (implementation) (20 min)
```

**Memory management**
```
1. See: DATA_FLOW_DIAGRAMS.md → Diagram 13 "Memory & Reference Counting" (10 min)
2. Read: QUICK_REFERENCE.md → Memory Model (5 min)
3. Check: ARCHITECTURE_OVERVIEW.md → GObject section (10 min)
```

---

## Document Statistics

| File | Size | Content | Time |
|------|------|---------|------|
| DOCUMENTATION_README.md | 8.6 KB | Overview + navigation | 5 min |
| ARCHITECTURE_OVERVIEW.md | 33 KB | Complete system design | 30 min |
| QUICK_REFERENCE.md | 7.5 KB | Lookup tables + quick facts | 5 min |
| DATA_FLOW_DIAGRAMS.md | 19 KB | 14 detailed diagrams | 20 min |
| **Total** | **68 KB** | **Comprehensive coverage** | **1 hour** |

---

## Five Core Concepts

### 1. Data Flow
```
Serial Port → Buffer → Terminal Display
                    → Logging File
                    → Sunshine Charts
```

### 2. Signal-Based Architecture
Components communicate via signals instead of direct calls:
- Loose coupling
- Multiple listeners
- Event-driven design

### 3. Port Abstraction
All port types (Serial, TCP, UDP) implement same interface:
- Unified API
- Runtime selection
- Easy to extend

### 4. Sunshine Visualization
Custom packet format: `{ID|Series|Color|Value|...}`
- Real-time parsing
- Incremental scanning
- Multiple chart types

### 5. GObject Pattern
C-based OOP using GObject:
- Type system
- Memory management (reference counting)
- Signals
- Properties

---

## Component Overview

### Core (7 components)
- GtkTermApplication - Entry point
- GtkTermWindow - Main UI
- GtkTermTerminal - Display
- GtkTermBuffer - Data storage
- GtkTermSerialPort - Serial I/O
- GtkTermTCPPort - Network I/O
- GtkTermUDPPort - UDP I/O

### Sunshine (4 components)
- GtkTermSunshineManager - Coordinator
- GtkTermSunshineScanner - Packet finder
- GtkTermSunshineParser - Format parser
- Chart widgets (line, bar, pie, scatter)

### Decoders (8 components)
- AT, NMEA, Modbus, Modbus TCP, CAN, EtherCAT, RS232

### Supporting (5+ components)
- Logging, Macros, Configuration, File Transfer, etc.

---

## Recommended Reading Order

### For Newcomers (1 hour)
1. This file (10 min)
2. DOCUMENTATION_README.md (10 min)
3. ARCHITECTURE_OVERVIEW.md → "Overall Architecture" (20 min)
4. QUICK_REFERENCE.md → "Key Takeaways" (10 min)
5. DATA_FLOW_DIAGRAMS.md → Diagrams 2 & 3 (10 min)

### For Developers (2 hours)
1. ARCHITECTURE_OVERVIEW.md → Full read (45 min)
2. DATA_FLOW_DIAGRAMS.md → All diagrams (45 min)
3. QUICK_REFERENCE.md → All sections (20 min)
4. Source code review (as needed) (10 min)

### For Code Review (30 min)
1. QUICK_REFERENCE.md → Component tables (5 min)
2. DATA_FLOW_DIAGRAMS.md → Relevant diagrams (15 min)
3. ARCHITECTURE_OVERVIEW.md → Affected sections (10 min)

---

## File Locations

```
gtkterm-sin-of-pride-version/
├── START_HERE.md                    (This file)
├── DOCUMENTATION_README.md          (Master index)
├── ARCHITECTURE_OVERVIEW.md         (Complete design)
├── QUICK_REFERENCE.md              (Lookup guide)
├── DATA_FLOW_DIAGRAMS.md           (Flow diagrams)
├── src/                            (Source code)
│   ├── gtkterm.c/h                (Entry point)
│   ├── gtkterm_window.c/h          (Main window)
│   ├── gtkterm_buffer.c/h          (Data storage)
│   ├── gtkterm_*_port.c/h          (Port implementations)
│   ├── gtkterm_*_decoder.c/h       (Protocol decoders)
│   └── sunshine/                   (Visualization)
│       ├── manager.c/h
│       ├── scanner.c/h
│       ├── data_parser.c/h
│       └── *_chart.c/h
├── include/
│   └── sunshine/                   (Sunshine headers)
├── meson.build                     (Build config)
└── build/                          (Compiled output)
```

---

## Key Features at a Glance

### Terminal Capabilities
- 7 display modes (ASCII, hex, binary, decimal, mixed)
- Configurable font, colors, cursor
- RX/TX marking
- Microsecond timestamps
- Search functionality

### Port Support
- Serial (RS232/RS485) at 300-2Mbps
- TCP client/server
- UDP
- Port statistics
- Error handling

### Sunshine Visualization
- Real-time parsing from custom packet format
- 4 chart types with auto-detection
- Color-coded data series
- Efficient incremental scanning
- Toggleable (Ctrl+Shift+V)

### Data Management
- Ring buffer (legacy)
- Typed data chunks (new)
- 10,000 chunk capacity
- Timestamp tracking
- Direction metadata

---

## Common Tasks

### Find a specific component
→ QUICK_REFERENCE.md tables (1 min)

### Understand Sunshine system
→ ARCHITECTURE_OVERVIEW.md section "Sunshine Visualization System" (15 min)

### Debug data flow
→ DATA_FLOW_DIAGRAMS.md diagrams 2-3 (10 min)

### Check memory management
→ DATA_FLOW_DIAGRAMS.md diagram 13 (5 min)

### Add protocol decoder
→ ARCHITECTURE_OVERVIEW.md "Protocol Decoder System" (10 min)

### Add chart type
→ ARCHITECTURE_OVERVIEW.md "Adding New Chart Types" (10 min)

### Configure terminal
→ QUICK_REFERENCE.md "Key Configuration" (5 min)

### Test without hardware
→ QUICK_REFERENCE.md "Testing Virtual Serial Port" (5 min)

---

## Build & Run

```bash
# Build
meson setup build
ninja -C build

# Run
./build/src/gtkterm-sin-of-pride-version

# Test (virtual serial port)
socat -d -d pty,raw,echo=0 pty,raw,echo=0
# Returns: /dev/pts/X and /dev/pts/Y
# Connect to one, send test data to other

# Example: Send Sunshine packet
echo "{P0|Temperature|255,0,0|23.5}" > /dev/pts/Y
```

---

## Questions?

1. **Quick answer?** → QUICK_REFERENCE.md
2. **Need full context?** → ARCHITECTURE_OVERVIEW.md
3. **How does it flow?** → DATA_FLOW_DIAGRAMS.md
4. **Don't know where to start?** → DOCUMENTATION_README.md

---

## Document Quality

- 1,783 lines of documentation
- 14 detailed diagrams
- 30+ component references
- 100% coverage of major systems
- Cross-referenced topics
- Navigation guides

---

**Ready? Start with DOCUMENTATION_README.md or jump to a specific file above!**

