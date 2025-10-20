# GTKTerm Language Definitions for GtkSourceView

This directory contains custom language definition files for syntax highlighting
serial communication protocols in GTKTerm.

## Overview

These language definitions enable syntax highlighting for common serial protocols
when GtkSourceView 5 is available. If GtkSourceView is not installed, GTKTerm
will still function normally without syntax highlighting.

## Supported Protocols

### 1. AT Commands (`at-commands.lang`)
Hayes AT command set used by:
- Modems (dial-up, DSL, cable)
- Cellular modules (2G, 3G, 4G, 5G)
- GPS/GNSS modules
- Bluetooth modules
- WiFi modules
- LoRa modules

**Highlights:**
- Standard AT commands (A, D, E, H, etc.)
- Extended commands (+CGMI, +CREG, +CSQ, etc.)
- Response codes (OK, ERROR, CONNECT, etc.)
- Parameters and separators

**Example:**
```
AT+CGMI
Manufacturer
OK

AT+CSQ
+CSQ: 25,0
OK
```

### 2. NMEA 0183 (`nmea.lang`)
Navigation sentences from:
- GPS/GNSS receivers
- Marine electronics
- Navigation systems
- Depth sounders
- Wind sensors

**Highlights:**
- Talker IDs (GP, GL, GA, GN, etc.)
- Sentence types (GGA, RMC, GSA, GSV, etc.)
- Checksums
- Coordinate and time data

**Example:**
```
$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47
$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A
```

### 3. Modbus ASCII (`modbus-ascii.lang`)
Industrial automation protocol for:
- PLCs (Programmable Logic Controllers)
- SCADA systems
- Industrial sensors and actuators
- Building automation
- Process control

**Highlights:**
- Start character (:)
- Slave addresses
- Function codes
- Data fields
- LRC checksums
- Error responses

**Example:**
```
:010300000002FA
:010304000A000147
```

### 4. Hexadecimal Dump (`hexdump.lang`)
Binary data visualization for:
- Debugging serial protocols
- Reverse engineering
- Binary file analysis
- Firmware dumps

**Highlights:**
- Memory offsets
- Hex byte values
- Null bytes
- Control characters
- ASCII representation

**Example:**
```
00000000  48 65 6C 6C 6F 20 57 6F  72 6C 64 21 00 00 00 00 | Hello World!....
00000010  01 02 03 04 05 06 07 08  09 0A 0B 0C 0D 0E 0F 10 | ................
```

## Installation

### System-wide (requires root)
```bash
sudo cp *.lang /usr/share/gtksourceview-5/language-specs/
```

### User-local
```bash
mkdir -p ~/.local/share/gtksourceview-5/language-specs
cp *.lang ~/.local/share/gtksourceview-5/language-specs/
```

### GTKTerm automatic (when built with GtkSourceView)
The language files are automatically installed with GTKTerm.

## Usage in GTKTerm

When GtkSourceView 5 is available:

1. **Automatic Detection:**
   - GTKTerm can auto-detect protocol type from data patterns
   - AT commands: Lines starting with "AT"
   - NMEA: Lines starting with "$GP", "$GL", etc.
   - Modbus ASCII: Lines starting with ":"
   - Hexdump: Lines with hex offsets

2. **Manual Selection:**
   - View → Syntax Highlighting → Select Language
   - Choose from: AT Commands, NMEA, Modbus ASCII, Hexdump, or None

3. **Color Schemes:**
   - Classic (default)
   - Tango
   - Solarized Dark/Light
   - Dracula
   - Oblivion
   - Kate

## Creating Custom Languages

To create a custom language definition:

1. Follow the GtkSourceView language specification:
   https://wiki.gnome.org/Projects/GtkSourceView/LanguageDefinitions

2. Define styles (keyword, string, number, comment, etc.)

3. Create regex patterns or keyword lists

4. Test with sample data

5. Save as `my-protocol.lang`

6. Install to language-specs directory

## Technical Details

**Format:** XML
**Schema:** GtkSourceView Language Definition 2.0
**Version:** Compatible with GtkSourceView 5.x

Each language file defines:
- Metadata (MIME types, file extensions)
- Styles (how elements should be colored)
- Contexts (regex patterns or keyword lists)
- Highlighting rules

## Contributing

To contribute additional protocol definitions:

1. Research the protocol specification
2. Create language definition file
3. Test with real-world data samples
4. Document highlights and examples
5. Submit pull request

Useful protocols to add:
- UBX (u-blox binary protocol)
- SCPI (Standard Commands for Programmable Instruments)
- MQTT (Message Queuing Telemetry Transport)
- JSON (for IoT devices)
- XML (for web services)
- CSV (sensor data logs)

## References

- [GtkSourceView Documentation](https://gnome.pages.gitlab.gnome.org/gtksourceview/)
- [Language Definition Tutorial](https://wiki.gnome.org/Projects/GtkSourceView/LanguageDefinitionv2.0Tutorial)
- [AT Commands Reference](https://www.3gpp.org/ftp/Specs/archive/27_series/27.007/)
- [NMEA 0183 Specification](https://www.nmea.org/content/STANDARDS/NMEA_0183_Standard)
- [Modbus Protocol](https://modbus.org/docs/Modbus_Application_Protocol_V1_1b3.pdf)

## License

These language definitions are part of GTKTerm and are released under the
GNU General Public License v3.0 or later.
