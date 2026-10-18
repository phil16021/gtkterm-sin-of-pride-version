# GTKTerm script language

One statement per line. `#` starts a comment line (indentation allowed). Commands are
case-insensitive; `${name}` is replaced by a variable's value everywhere (unknown variables
stay as written). Files written for the C GTKTerm run unchanged.

| Statement | Meaning |
|---|---|
| `send TEXT` | Transmit TEXT. Escapes: `\r \n \t \0 \e \\ \xHH`; any other `\c` is `c`. Everything after the first space is sent, trailing spaces too. |
| `send_hex DE AD 0x01` | Transmit hex bytes. |
| `expect REGEX` | Wait until data received since the last `send` (and after the previous match) matches REGEX (Rust regex syntax, on bytes). Fails the script on timeout. Sets `${MATCH_0}` (whole match), `${MATCH_1}`… (groups). |
| `expect[2000] REGEX` | Same with a one-off timeout (ms; `2s` works). |
| `timeout MS` | Default timeout of later `expect`/`if_match` (initially 5000 ms). |
| `if_match REGEX goto LABEL` | Like `expect`, but on a match jump to LABEL and on timeout continue. `if_match[ms]` sets the timeout. |
| `if A == B goto LABEL` / `if A != B goto LABEL` | Compare after `${}` expansion. |
| `goto LABEL` | Jump. |
| `label NAME` or `NAME:` | Jump target. |
| `set NAME VALUE` | Set a variable. |
| `delay MS` / `wait MS` | Pause (`100`, `100ms`, `1.5s`). |
| `set_dtr 0/1`, `set_rts on/off` | Modem output lines (`true/false`, `high/low` also work). |
| `break [MS]` | Send a break (default 250 ms). |
| `log TEXT` | Report a message. |
| `exec PROGRAM ARGS…` | Run a program (only when exec is allowed). No shell: arguments are split first (quotes `'…'` `"…"` and `\` work), then `${}` is expanded inside each argument, so received data can never add arguments. Sets `${EXEC_OUTPUT}` (stdout without the final newline) and `${EXEC_STATUS}`. 60 s limit. A program given as a path (`./flash.sh`) is relative to the script's folder; plain names are looked up in `PATH`. |
| `send_file PROTOCOL FILE…` | Upload with `xmodem`, `xmodem_1k`, `ymodem`, `ymodem_g` or `zmodem`. Sets `${TRANSFER_BYTES}`. |
| `receive_file PROTOCOL [DIR]` | Download into DIR (default: the script's directory). Sets `${TRANSFER_FILE}`. |
| `include FILE` | Insert another script (paths relative to the including file). |
| `exit` | Stop successfully. |
| `fail MESSAGE` | Stop with an error. |

Relative paths resolve against the script's directory; under MCP only the transfer
directory and `--mcp-allow-dir` directories are usable.

```text
# Probe a modem, retry once, branch on the answer
set tries 0
retry:
send AT\r
if_match[1000] OK goto ok
set tries ${tries}1
if ${tries} != 011 goto retry
fail no answer
ok:
send AT+CSQ\r
expect \+CSQ: (\d+),(\d+)
if ${MATCH_1} == 99 goto no_signal
log signal ${MATCH_1}
exit
no_signal:
fail no signal
```
