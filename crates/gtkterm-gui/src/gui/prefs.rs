//! Preferences dialog: connection, terminal and MCP settings of the current profile.

use std::rc::Rc;

use adw::prelude::*;
use gtkterm_core::config::{Connection, FlowControl, Parity, Profile, Rgba, SerialConfig, TerminalConfig};
use gtkterm_protocols::transform::TimestampFormat;
use gtkterm_protocols::view::{HEX_WIDTHS, ViewMode};

use super::state::McpSettings;

fn combo(title: &str, items: &[&str], selected: usize) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(items))
        .build();
    row.set_selected(selected as u32);
    row
}

fn spin(title: &str, min: f64, max: f64, value: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, 1.0);
    row.set_title(title);
    row.set_value(value);
    row
}

fn switch(title: &str, subtitle: &str, active: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).active(active).build();
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row
}

fn entry(title: &str, text: &str) -> adw::EntryRow {
    let row = adw::EntryRow::builder().title(title).build();
    row.set_text(text);
    row
}

fn group(title: &str, rows: &[&gtk::Widget]) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(title).build();
    for r in rows {
        g.add(*r);
    }
    g
}

fn to_rgba(c: Rgba) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(c.red as f32, c.green as f32, c.blue as f32, c.alpha as f32)
}

fn from_rgba(c: &gtk::gdk::RGBA) -> Rgba {
    Rgba::new(
        f64::from(c.red()),
        f64::from(c.green()),
        f64::from(c.blue()),
        f64::from(c.alpha()),
    )
}

const PARITIES: [Parity; 5] = [
    Parity::None,
    Parity::Odd,
    Parity::Even,
    Parity::Mark,
    Parity::Space,
];
const FLOWS: [FlowControl; 4] = [
    FlowControl::None,
    FlowControl::XonXoff,
    FlowControl::RtsCts,
    FlowControl::Rs485,
];
const STAMPS: [TimestampFormat; 3] = [
    TimestampFormat::LocalTime,
    TimestampFormat::Relative,
    TimestampFormat::Iso8601,
];

/// Parse the "wait for character" entry: empty, a decimal byte, `0x..`, `\r`/`\n`, or one character.
fn parse_wait_char(s: &str) -> Result<Option<u8>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let byte = match s {
        "\\r" | "CR" | "cr" => Some(b'\r'),
        "\\n" | "LF" | "lf" => Some(b'\n'),
        _ => s
            .strip_prefix("0x")
            .map_or_else(|| s.parse::<u8>().ok(), |h| u8::from_str_radix(h, 16).ok())
            .or_else(|| (s.len() == 1).then(|| s.as_bytes()[0])),
    };
    byte.map(Some)
        .ok_or_else(|| format!("{s:?} is not a byte, 0xNN, \\r, \\n or a single character"))
}

fn show_wait_char(c: Option<u8>) -> String {
    match c {
        None => String::new(),
        Some(b'\r') => "\\r".into(),
        Some(b'\n') => "\\n".into(),
        Some(b) if b.is_ascii_graphic() => (b as char).to_string(),
        Some(b) => b.to_string(),
    }
}

/// Show the dialog; `apply` gets the edited profile and MCP settings when it is closed.
pub fn show(
    parent: &impl IsA<gtk::Widget>,
    profile_name: &str,
    profile: &Profile,
    mcp: &McpSettings,
    mcp_url: Option<String>,
    apply: impl Fn(Profile, McpSettings) + 'static,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title(format!("Preferences — [{profile_name}]"))
        .build();
    dialog.set_search_enabled(true);

    // ---------- Connection ----------
    let serial = profile.connection.serial().cloned().unwrap_or_default();
    let (kind_idx, host, tcp_port, bind, udp_local, udp_host, udp_port) = match &profile.connection {
        Connection::Serial(_) => (0, String::new(), 23, "0.0.0.0".to_owned(), 0, String::new(), 0),
        Connection::TcpClient { host, port, .. } => {
            (1, host.clone(), *port, "0.0.0.0".into(), 0, String::new(), 0)
        }
        Connection::TcpServer { bind, port } => (2, String::new(), *port, bind.clone(), 0, String::new(), 0),
        Connection::Udp {
            bind,
            local_port,
            remote_host,
            remote_port,
        } => (
            3,
            String::new(),
            0,
            bind.clone(),
            *local_port,
            remote_host.clone().unwrap_or_default(),
            *remote_port,
        ),
    };
    let kind = combo(
        "Connection type",
        &["Serial port", "TCP client", "TCP server", "UDP"],
        kind_idx,
    );
    let port = entry("Device", &serial.port);
    let baud = spin("Baud rate", 50.0, 12_000_000.0, f64::from(serial.baudrate));
    baud.set_subtitle("Any value; non-standard rates use termios2");
    let bits = combo(
        "Data bits",
        &["5", "6", "7", "8"],
        usize::from(serial.bits.clamp(5, 8) - 5),
    );
    let parity = combo(
        "Parity",
        &["None", "Odd", "Even", "Mark", "Space"],
        PARITIES.iter().position(|p| *p == serial.parity).unwrap_or(0),
    );
    let stop = combo("Stop bits", &["1", "2"], usize::from(serial.stopbits == 2));
    let flow = combo(
        "Flow control",
        &["None", "XON/XOFF", "RTS/CTS", "RS485 (RTS while sending)"],
        FLOWS.iter().position(|f| *f == serial.flow_control).unwrap_or(0),
    );
    let before = spin(
        "RS485 RTS lead time (ms)",
        0.0,
        10_000.0,
        f64::from(serial.rs485_rts_time_before_tx),
    );
    let after = spin(
        "RS485 RTS tail time (ms)",
        0.0,
        10_000.0,
        f64::from(serial.rs485_rts_time_after_tx),
    );
    let nolock = switch(
        "Do not lock the device",
        "Allow other programs to open it at the same time",
        serial.disable_port_lock,
    );
    let serial_group = group(
        "Serial port",
        &[
            port.upcast_ref(),
            baud.upcast_ref(),
            bits.upcast_ref(),
            parity.upcast_ref(),
            stop.upcast_ref(),
            flow.upcast_ref(),
            before.upcast_ref(),
            after.upcast_ref(),
            nolock.upcast_ref(),
        ],
    );

    let host_row = entry("Host", &host);
    let tcp_port_row = spin("Port", 1.0, 65_535.0, f64::from(tcp_port.max(1)));
    let tcp_group = group("TCP client", &[host_row.upcast_ref(), tcp_port_row.upcast_ref()]);
    let bind_row = entry("Bind address", &bind);
    let listen_row = spin("Listen port", 1.0, 65_535.0, f64::from(tcp_port.max(1)));
    let server_group = group("TCP server", &[bind_row.upcast_ref(), listen_row.upcast_ref()]);
    let udp_local_row = spin("Local port (0 = any)", 0.0, 65_535.0, f64::from(udp_local));
    let udp_host_row = entry("Remote host (empty: first sender)", &udp_host);
    let udp_port_row = spin("Remote port", 0.0, 65_535.0, f64::from(udp_port));
    let udp_group = group(
        "UDP",
        &[
            udp_local_row.upcast_ref(),
            udp_host_row.upcast_ref(),
            udp_port_row.upcast_ref(),
        ],
    );

    let groups = [
        serial_group.clone(),
        tcp_group.clone(),
        server_group.clone(),
        udp_group.clone(),
    ];
    let show_groups = {
        let groups = groups.clone();
        move |i: u32| {
            for (n, g) in groups.iter().enumerate() {
                g.set_visible(n as u32 == i);
            }
        }
    };
    show_groups(kind.selected());
    kind.connect_selected_notify(move |c| show_groups(c.selected()));

    let conn_page = adw::PreferencesPage::builder()
        .title("Connection")
        .icon_name("network-wired-symbolic")
        .build();
    conn_page.add(&group("", &[kind.upcast_ref()]));
    for g in &groups {
        conn_page.add(g);
    }
    let note = adw::PreferencesGroup::builder()
        .description("Changes apply when the dialog is closed; an open connection is reopened.")
        .build();
    conn_page.add(&note);
    dialog.add(&conn_page);

    // ---------- Terminal ----------
    let t = &profile.terminal;
    let font_button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    font_button.set_font_desc(&pango::FontDescription::from_string(&t.font));
    font_button.set_valign(gtk::Align::Center);
    let font_row = adw::ActionRow::builder().title("Font").build();
    font_row.add_suffix(&font_button);
    let fg_button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    fg_button.set_rgba(&to_rgba(t.foreground));
    fg_button.set_valign(gtk::Align::Center);
    let bg_button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    bg_button.set_rgba(&to_rgba(t.background));
    bg_button.set_valign(gtk::Align::Center);
    let colors_row = adw::ActionRow::builder()
        .title("Text / background colour")
        .build();
    colors_row.add_suffix(&fg_button);
    colors_row.add_suffix(&bg_button);
    let scrollback = spin("Scrollback lines", 0.0, 1_000_000.0, f64::from(t.scrollback));
    let block = switch("Block cursor", "", t.block_cursor);
    let blink = switch("Show cursor", "", t.show_cursor);
    let bell = switch("Visual bell", "Flash instead of beeping", t.visual_bell);
    let echo = switch("Local echo", "Show sent data", t.echo);
    let auto_cr = switch("Auto CR", "Insert CR before a bare LF", t.auto_cr);
    let auto_lf = switch("Auto LF", "Insert LF after a bare CR", t.auto_lf);
    let stamps = switch("Timestamps", "Prefix every received line", t.timestamps);
    let stamp_fmt = combo(
        "Timestamp format",
        &["Local time HH:MM:SS.mmm", "Relative +S.mmm", "ISO 8601"],
        STAMPS.iter().position(|f| *f == t.timestamp_format).unwrap_or(0),
    );
    let delay = spin(
        "Delay after each line (ms)",
        0.0,
        500.0,
        f64::from(t.wait_delay_ms),
    );
    let wait_char = entry(
        "Wait for character after each line (empty: off)",
        &show_wait_char(t.wait_char),
    );
    let mode_labels: Vec<&str> = ViewMode::ALL.iter().map(|m| m.label()).collect();
    let mode = combo(
        "View mode",
        &mode_labels,
        ViewMode::ALL.iter().position(|m| *m == t.view_mode).unwrap_or(0),
    );
    let index = switch(
        "Show byte index",
        "In hex, binary and decimal views",
        t.show_index,
    );
    let hex_labels: Vec<String> = HEX_WIDTHS.iter().map(ToString::to_string).collect();
    let hex_refs: Vec<&str> = hex_labels.iter().map(String::as_str).collect();
    let hex = combo(
        "Bytes per line in hex view",
        &hex_refs,
        HEX_WIDTHS
            .iter()
            .position(|w| *w == usize::from(t.hex_chars))
            .unwrap_or(2),
    );

    let term_page = adw::PreferencesPage::builder()
        .title("Terminal")
        .icon_name("utilities-terminal-symbolic")
        .build();
    term_page.add(&group(
        "Appearance",
        &[
            font_row.upcast_ref(),
            colors_row.upcast_ref(),
            scrollback.upcast_ref(),
            block.upcast_ref(),
            blink.upcast_ref(),
            bell.upcast_ref(),
        ],
    ));
    term_page.add(&group(
        "Received data",
        &[
            mode.upcast_ref(),
            index.upcast_ref(),
            hex.upcast_ref(),
            auto_cr.upcast_ref(),
            auto_lf.upcast_ref(),
            stamps.upcast_ref(),
            stamp_fmt.upcast_ref(),
        ],
    ));
    term_page.add(&group(
        "Sent data",
        &[echo.upcast_ref(), delay.upcast_ref(), wait_char.upcast_ref()],
    ));
    dialog.add(&term_page);

    // ---------- MCP ----------
    let mcp_enabled = switch(
        "MCP server",
        "Let AI agents use this window's connection over HTTP",
        mcp.enabled,
    );
    let mcp_port = spin("Port on 127.0.0.1", 1024.0, 65_535.0, f64::from(mcp.port));
    let mcp_ro = switch(
        "Read-only",
        "Agents may read data but not send, open or close",
        mcp.read_only,
    );
    let token = mcp.token.clone().unwrap_or_default();
    let token_row = adw::ActionRow::builder()
        .title("Bearer token")
        .subtitle(if token.is_empty() {
            "generated when the server starts"
        } else {
            &token
        })
        .subtitle_selectable(true)
        .build();
    let command = mcp_url.as_ref().map(|u| {
        format!("claude mcp add --transport http gtkterm {u} --header \"Authorization: Bearer {token}\"")
    });
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy the claude mcp add command")
        .valign(gtk::Align::Center)
        .sensitive(command.is_some())
        .build();
    copy.add_css_class("flat");
    let cmd = command.clone();
    copy.connect_clicked(move |b| {
        if let Some(c) = &cmd {
            b.clipboard().set_text(c);
        }
    });
    token_row.add_suffix(&copy);
    let mcp_page = adw::PreferencesPage::builder()
        .title("MCP")
        .icon_name("network-server-symbolic")
        .build();
    let mcp_group = group(
        "Model Context Protocol",
        &[
            mcp_enabled.upcast_ref(),
            mcp_port.upcast_ref(),
            mcp_ro.upcast_ref(),
            token_row.upcast_ref(),
        ],
    );
    mcp_group.set_description(Some(
        "Headless alternative for Claude Code: claude mcp add gtkterm -- gtkterm-sunshine --mcp-stdio",
    ));
    mcp_page.add(&mcp_group);
    dialog.add(&mcp_page);

    let original = profile.clone();
    let original_mcp = mcp.clone();
    let apply = Rc::new(apply);
    dialog.connect_closed(move |_| {
        let mut p = original.clone();
        p.connection = match kind.selected() {
            1 => Connection::TcpClient {
                host: host_row.text().trim().to_owned(),
                port: tcp_port_row.value() as u16,
                connect_timeout_ms: 5_000,
            },
            2 => Connection::TcpServer {
                bind: bind_row.text().trim().to_owned(),
                port: listen_row.value() as u16,
            },
            3 => {
                let h = udp_host_row.text().trim().to_owned();
                Connection::Udp {
                    bind: if bind.is_empty() {
                        "0.0.0.0".into()
                    } else {
                        bind.clone()
                    },
                    local_port: udp_local_row.value() as u16,
                    remote_host: (!h.is_empty()).then_some(h),
                    remote_port: udp_port_row.value() as u16,
                }
            }
            _ => Connection::Serial(SerialConfig {
                port: port.text().trim().to_owned(),
                baudrate: baud.value() as u32,
                bits: 5 + bits.selected() as u8,
                stopbits: 1 + stop.selected() as u8,
                parity: PARITIES[(parity.selected() as usize).min(4)],
                flow_control: FLOWS[(flow.selected() as usize).min(3)],
                rs485_rts_time_before_tx: before.value() as u32,
                rs485_rts_time_after_tx: after.value() as u32,
                disable_port_lock: nolock.is_active(),
            }),
        };
        p.terminal = TerminalConfig {
            font: font_button
                .font_desc()
                .map_or(p.terminal.font.clone(), |f| f.to_string()),
            foreground: from_rgba(&fg_button.rgba()),
            background: from_rgba(&bg_button.rgba()),
            scrollback: scrollback.value() as u32,
            block_cursor: block.is_active(),
            show_cursor: blink.is_active(),
            visual_bell: bell.is_active(),
            echo: echo.is_active(),
            auto_cr: auto_cr.is_active(),
            auto_lf: auto_lf.is_active(),
            timestamps: stamps.is_active(),
            timestamp_format: STAMPS[(stamp_fmt.selected() as usize).min(2)],
            wait_delay_ms: delay.value() as u32,
            wait_char: parse_wait_char(&wait_char.text()).unwrap_or(p.terminal.wait_char),
            view_mode: ViewMode::ALL[(mode.selected() as usize).min(ViewMode::ALL.len() - 1)],
            show_index: index.is_active(),
            hex_chars: HEX_WIDTHS[(hex.selected() as usize).min(HEX_WIDTHS.len() - 1)] as u8,
            ..p.terminal.clone()
        };
        let m = McpSettings {
            enabled: mcp_enabled.is_active(),
            port: mcp_port.value() as u16,
            read_only: mcp_ro.is_active(),
            token: original_mcp.token.clone(),
        };
        apply(p, m);
    });
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_char_parsing() {
        assert_eq!(parse_wait_char(""), Ok(None));
        assert_eq!(parse_wait_char("62"), Ok(Some(62)));
        assert_eq!(parse_wait_char("0x3E"), Ok(Some(0x3E)));
        assert_eq!(parse_wait_char(">"), Ok(Some(b'>')));
        assert_eq!(parse_wait_char("\\r"), Ok(Some(b'\r')));
        assert!(parse_wait_char("abc").is_err());
        for c in [None, Some(b'\r'), Some(b'\n'), Some(b'>'), Some(7)] {
            assert_eq!(parse_wait_char(&show_wait_char(c)), Ok(c));
        }
    }
}
