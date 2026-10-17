# Sunshine packet format

GTKTerm Sunshine plots every received line of the form

    {ID|Name|R,G,B|value|Name2|value2 value3|...}

- `ID` names the chart (one tab per ID). An optional prefix chooses the chart type:
  `L:` line (default), `B:` bar, `P:` pie, `S:` scatter — e.g. `{B:levels|...}`.
- Each series is `Name`, an optional colour `R,G,B` (three integers 0-255) and a value field.
- A value field holds one or more numbers separated by spaces, with `.` as decimal separator
  (`1.5`, `-3e2`). One number is a Y sample; two numbers are an X/Y pair (scatter).
- `nan`, `inf`, comma decimals (`1,5`) and text after `}` are rejected.
- A line may contain several packets: `{a|x|1}{b|y|2}`. Lines end with `\n`, `\r` or `\r\n`.
- Names and IDs are at most 64 characters; lines at most 4096 bytes.

Examples:

    {P0|Temperature|255,0,0|23.5}
    {imu|Roll|255,0,0|45.2|Pitch|0,255,0|12.3|Yaw|0,0,255|180.0}
    {S:xy|path|1.0 2.5}

C example (one packet per sample):

    printf("{env|Temp|255,0,0|%.2f|Hum|0,0,255|%.1f}\r\n", temp, hum);
