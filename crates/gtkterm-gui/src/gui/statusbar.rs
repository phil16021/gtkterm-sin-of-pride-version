//! Bottom status bar.

use gtk::prelude::*;
use gtkterm_core::port::ModemLines;
use gtkterm_core::session::{PortState, SessionStatus};

/// Status bar widgets.
pub struct StatusBar {
    /// Top-level widget.
    pub widget: gtk::Box,
    /// Profile menu button (its popover is filled by the window).
    pub profile: gtk::MenuButton,
    endpoint: gtk::Label,
    state: gtk::Label,
    counters: gtk::Label,
    /// DTR toggle.
    pub dtr: gtk::ToggleButton,
    /// RTS toggle.
    pub rts: gtk::ToggleButton,
    leds: [gtk::Label; 4],
    log: gtk::Label,
    mcp: gtk::Label,
    activity: gtk::Label,
}

fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 10 * 1024 * 1024 => format!("{:.1} MiB", b as f64 / 1_048_576.0),
        b if b >= 10 * 1024 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

impl StatusBar {
    /// Build an idle status bar.
    pub fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        widget.add_css_class("statusbar");
        let profile = gtk::MenuButton::builder()
            .label("[default]")
            .tooltip_text("Profiles")
            .build();
        profile.add_css_class("flat");
        let endpoint = gtk::Label::builder()
            .label("No open port")
            .ellipsize(pango::EllipsizeMode::Middle)
            .max_width_chars(48)
            .build();
        let state = gtk::Label::builder()
            .label("Disconnected")
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(40)
            .build();
        let counters = gtk::Label::new(Some("RX 0 B · TX 0 B"));
        counters.add_css_class("numeric");
        let spacer = gtk::Box::builder().hexpand(true).build();
        let dtr = gtk::ToggleButton::builder()
            .label("DTR")
            .tooltip_text("Data Terminal Ready (F7)")
            .sensitive(false)
            .build();
        let rts = gtk::ToggleButton::builder()
            .label("RTS")
            .tooltip_text("Request To Send (F8)")
            .sensitive(false)
            .build();
        for b in [&dtr, &rts] {
            b.add_css_class("flat");
        }
        let leds = ["CTS", "DSR", "DCD", "RI"].map(|n| {
            let l = gtk::Label::new(Some(n));
            l.add_css_class("led");
            l
        });
        let log = gtk::Label::new(None);
        let mcp = gtk::Label::new(None);
        let activity = gtk::Label::builder()
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(30)
            .build();
        widget.append(&profile);
        widget.append(&endpoint);
        widget.append(&state);
        widget.append(&counters);
        widget.append(&spacer);
        widget.append(&activity);
        widget.append(&log);
        widget.append(&mcp);
        widget.append(&dtr);
        widget.append(&rts);
        for l in &leds {
            widget.append(l);
        }
        Self {
            widget,
            profile,
            endpoint,
            state,
            counters,
            dtr,
            rts,
            leds,
            log,
            mcp,
            activity,
        }
    }

    /// Show a session status.
    pub fn update(&self, st: &SessionStatus) {
        self.profile.set_label(&format!("[{}]", st.name));
        self.endpoint.set_text(if st.description.is_empty() {
            "No open port"
        } else {
            &st.description
        });
        self.endpoint.set_tooltip_text(Some(&st.description));
        let (text, class) = match &st.state {
            PortState::Open => ("Connected".to_owned(), Some("state-open")),
            PortState::Opening => ("Connecting…".to_owned(), None),
            PortState::Closed => ("Disconnected".to_owned(), None),
            PortState::Error(e) => (format!("Error: {e}"), Some("state-error")),
        };
        self.state.set_text(&text);
        self.state.set_tooltip_text(Some(&text));
        for c in ["state-open", "state-error"] {
            self.state.remove_css_class(c);
        }
        if let Some(c) = class {
            self.state.add_css_class(c);
        }
        let queue = if st.tx_queue > 0 {
            format!(" · queue {}", st.tx_queue)
        } else {
            String::new()
        };
        self.counters.set_text(&format!(
            "RX {} · TX {}{queue}",
            human(st.rx_bytes),
            human(st.tx_bytes)
        ));
        match &st.log {
            Some(l) => {
                let paused = matches!(l.state, gtkterm_core::logging::LogState::Paused);
                self.log.set_text(&format!(
                    "● LOG{} {}",
                    if paused { " (paused)" } else { "" },
                    human(l.bytes)
                ));
                self.log.set_tooltip_text(Some(&l.path.display().to_string()));
            }
            None => self.log.set_text(""),
        }
        self.set_modem(st.modem);
    }

    /// Show modem lines (`None`: unavailable).
    pub fn set_modem(&self, m: Option<ModemLines>) {
        let available = m.is_some();
        let m = m.unwrap_or_default();
        for (b, on) in [(&self.dtr, m.dtr), (&self.rts, m.rts)] {
            b.set_sensitive(available);
            if b.is_active() != on {
                b.set_active(on);
            }
        }
        for (l, on) in self.leds.iter().zip([m.cts, m.dsr, m.dcd, m.ri]) {
            l.set_visible(available);
            if on {
                l.add_css_class("on");
            } else {
                l.remove_css_class("on");
            }
        }
    }

    /// Show macro activity: recording, a running macro, or nothing.
    pub fn set_activity(&self, recording: bool, running: Option<&str>) {
        let text = match (recording, running) {
            (true, _) => "● REC".to_owned(),
            (false, Some(name)) => format!("▶ {name}"),
            (false, None) => String::new(),
        };
        self.activity.set_text(&text);
        self.activity
            .set_tooltip_text(running.map(|n| format!("Macro {n} is running")).as_deref());
    }

    /// Show the embedded MCP endpoint state.
    pub fn set_mcp(&self, url: Option<&str>) {
        match url {
            Some(u) => {
                self.mcp.set_text("◆ MCP");
                self.mcp
                    .set_tooltip_text(Some(&format!("MCP server listening on {u}")));
            }
            None => self.mcp.set_text(""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::human;

    #[test]
    fn byte_formatting() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(20 * 1024), "20.0 KiB");
        assert_eq!(human(30 * 1024 * 1024), "30.0 MiB");
    }
}
