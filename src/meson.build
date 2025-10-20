gtkterm_sources = [
	'gtkterm_cmdline.c',
	'gtkterm.c',
	'gtkterm_window.c',
	'macros.c',
	'gtkterm_configuration.c',
	'gtkterm_serial_port.c',
	'gtkterm_terminal.c',
	'gtkterm_buffer.c',
	'gtkterm_logging.c',
	'gtkterm_data_chunk.c',
	'gtkterm_macro.c',
	'gtkterm_dataview.c',
	'gtkterm_protocol_decoder.c',
	'gtkterm_at_decoder.c',
	'gtkterm_modbus_decoder.c',
	'gtkterm_nmea_decoder.c',
	'gtkterm_can_decoder.c',
	'gtkterm_ethercat_decoder.c',
	'gtkterm_modbus_tcp_decoder.c',
	'gtkterm_rs232_decoder.c',
	'gtkterm_file_transfer.c',
	'gtkterm_xmodem.c',
	'gtkterm_automation.c',
	'gtkterm_port.c',
	'gtkterm_tcp_port.c',
	'gtkterm_udp_port.c',
	'gtkterm_port_control.c',
	'gtkterm_statusbar.c',
	'gtkterm_toolbar.c',
	'gtkterm_shortcuts.c',
	'gtkterm_presets.c',
	'gtkterm_signal_monitor.c',
	# Sunshine visualization components
	'sunshine/data_parser.c',
	'sunshine/scanner.c',
	'sunshine/manager.c',
	'sunshine/line_chart.c',
	'sunshine/bar_chart.c',
	'sunshine/pie_chart.c',
	'sunshine/scatter_chart.c',
	gresources
]

# Build dependency list
cc = meson.get_compiler('c')
math_dep = cc.find_library('m', required: false)

gtkterm_deps = [
	gtk_deps,
	vte_deps,
	config,
	gudev_deps,
	json_glib_deps,
	math_dep
]

# Add GtkSourceView if available
if gtksourceview_deps.found()
	gtkterm_deps += gtksourceview_deps
endif

executable(
	'gtkterm-sin-of-pride-version',
	[gtkterm_sources],
	export_dynamic : true,
	dependencies : gtkterm_deps,
	include_directories : include_directories('../include/sunshine'),
	install : true
)
