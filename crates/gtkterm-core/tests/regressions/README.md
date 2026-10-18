# C defect regression guards

Every defect of the C version listed in ROADMAP §8 has a test that fails on the C behaviour.
Guards that need no other context live in this directory (`cargo nextest run -p gtkterm-core
--test regressions`); the others sit next to the code they protect. Run one with
`cargo nextest run -E 'test(NAME)'`.

| ID | C defect | Guard |
|---|---|---|
| B1 | repage `memmove` backwards, unbounded `\n` scan | `serial_one_megabyte_each_way_is_exact` (core `tests/ports.rs`) |
| B2 | heap overflow after repage | `caps_hold` proptest (core `src/chunks.rs`) |
| B3 | auto CR/LF never set `cr_received` | `cr_lf_matrix`, `crlf_split_across_chunks_is_not_doubled` (protocols `src/transform.rs`) |
| B4 | buffer dispose freed GObjects | `c_b4_*` (this directory: drop order, device lock released) |
| S1 | Sunshine stopped after 10k chunks | `sunshine_keeps_updating_after_twenty_thousand_chunks` (core `tests/session.rs`), `many_chunks_keep_updating`, `eviction_by_count_keeps_sequence_monotonic` |
| S2 | split packets dropped | `split_anywhere_is_equivalent`, `scanner_reassembles_split_packets` (protocols `src/sunshine.rs`) |
| S3 | re-scan on every panel reopen | `c_s3_panel_toggles_keep_point_counts` (GUI `window.rs`) |
| S4 | `1,5` read as a colour; NaN/inf accepted | `rejects` cases (protocols `src/sunshine.rs`) |
| S5 | only line charts | `routes_by_id_and_prefix` (core `src/sunshine.rs`), `window_charts_actions_and_dialogs` (GUI) |
| S6 | axis drift, flat thresholds, everything blue | `small_signals_are_not_flattened`, `ticks_are_inside_and_ascending` (charts `src/scale.rs`), `packet_colour_wins_and_defaults_differ` (`src/palette.rs`) |
| X1 | first XMODEM block lost on the `C`/NAK handshake | `first_block_survives_a_repeated_start_request` (protocols `src/xmodem.rs`), lrzsz interop (core `tests/transfer.rs`) |
| X2 | EOT/CAN detected inside the payload | `xmodem_sizes_and_special_bytes` (protocols `src/xmodem.rs`) |
| X3 | CTRL-Z stripped from every block | `xmodem_keeps_sub_bytes_inside_the_file` |
| X4 | block-255 wrap duplicate NAK'd forever | `xmodem_sizes_and_special_bytes`, lrzsz `files` (core `tests/transfer.rs`) |
| P1 | RS485 RTS toggled, `usleep` on the UI thread | `c_p1_rs485_*` (core `src/port/serial/mod.rs`); hardware checklist |
| P2 | DTR/RTS toggles did nothing | `c_p2_dtr_and_rts_requests_reach_the_lines` (this directory) |
| P3 | custom baud kept stale `c_cflag` | `reconfigure_clears_stale_bits` (core `src/port/serial/mod.rs`) |
| P4 | partial writes dropped | `c_p4_slow_reader_gets_every_byte` (this directory), `serial_one_megabyte_each_way_is_exact` |
| P5 | Close reported failure on success | `c_p5_close_reports_success` (GUI `window.rs`) |
| T1 | TCP server read only the newest client; blocking connect | `tcp_server_merges_clients_and_survives_disconnects`, `tcp_server_drops_a_client_that_stops_reading`, `tcp_client_connect_timeout_or_unreachable` (core `tests/ports.rs`) |
| U1 | UDP hostname → NULL address | `udp_learns_remote_and_resolves_hostnames` (core `tests/ports.rs`) |
| C1 | CLI config options corrupted the stack | `config_operations` (GUI `tests/binary.rs`) |
| C2 | show_index under the block-cursor key, inverted lock, swapped rows/cols | `c_c2_legacy_keys_land_in_their_own_fields` (this directory), `imports_upstream_1x_keyfile` |
| M1 | macros: missing sentinel, use-after-free, invalid JSON | `json_roundtrip_with_non_ascii_and_controls` (core `src/macros.rs`) |
| L1 | logging never wrote data; no JSON/XML footer | `json_log_roundtrip_with_pause_and_clear`, `raw_log_appends_rx_only` (core `src/logging.rs`) |
| A1 | `exec` with device-controlled arguments | `exec_is_opt_in_and_never_uses_a_shell` (core `tests/script.rs`) |
| K1 | Ctrl+Shift+V bound 4×, Ctrl+letters stolen | `no_duplicates_and_no_plain_ctrl_letters` (GUI `shortcuts.rs`) |
| D1 | decoders read out of bounds | decoder vectors in `protocols/src/decode/*`, fuzz target `decoders` |
| I1 | names collided with upstream GTKTerm | `c_i1_installed_paths_do_not_collide_with_upstream`, `c_i1_desktop_file_names_this_app` (`xtask`) |
