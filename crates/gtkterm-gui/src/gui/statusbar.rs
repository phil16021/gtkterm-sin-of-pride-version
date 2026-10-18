//! Bottom status bar.

use gtk::prelude::*;
use gtkterm_core::port::ModemLines;
use gtkterm_core::session::{PortState, SessionStatus};
use gtkterm_core::transfer::{TransferState, TransferStatus};

use super::i18n::{gettext, gettext_f};

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
    transfer: gtk::Box,
    transfer_label: gtk::Label,
    transfer_bar: gtk::ProgressBar,
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
            .tooltip_text(gettext("Profiles"))
            .build();
        profile.add_css_class("flat");
        let endpoint = gtk::Label::builder()
            .label(gettext("No open port"))
            .ellipsize(pango::EllipsizeMode::Middle)
            .max_width_chars(48)
            .build();
        let state = gtk::Label::builder()
            .label(gettext("Disconnected"))
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(40)
            .build();
        let counters = gtk::Label::new(Some("RX 0 B · TX 0 B"));
        counters.add_css_class("numeric");
        let spacer = gtk::Box::builder().hexpand(true).build();
        let dtr = gtk::ToggleButton::builder()
            .label("DTR")
            .tooltip_text(gettext("Data Terminal Ready (F7)"))
            .sensitive(false)
            .build();
        let rts = gtk::ToggleButton::builder()
            .label("RTS")
            .tooltip_text(gettext("Request To Send (F8)"))
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
        let transfer_label = gtk::Label::builder()
            .ellipsize(pango::EllipsizeMode::Middle)
            .max_width_chars(40)
            .build();
        transfer_label.add_css_class("numeric");
        let transfer_bar = gtk::ProgressBar::builder()
            .width_request(90)
            .valign(gtk::Align::Center)
            .build();
        let cancel = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text(gettext("Cancel the file transfer"))
            .action_name("win.cancel-transfer")
            .build();
        cancel.add_css_class("flat");
        let transfer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        transfer.append(&transfer_label);
        transfer.append(&transfer_bar);
        transfer.append(&cancel);
        transfer.set_visible(false);
        widget.append(&profile);
        widget.append(&endpoint);
        widget.append(&state);
        widget.append(&counters);
        widget.append(&spacer);
        widget.append(&transfer);
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
            transfer,
            transfer_label,
            transfer_bar,
        }
    }

    /// Show a running transfer; hidden when it is over.
    pub fn set_transfer(&self, st: Option<&TransferStatus>) {
        let Some(st) = st.filter(|s| s.state == TransferState::Running) else {
            self.transfer.set_visible(false);
            return;
        };
        self.transfer.set_visible(true);
        self.transfer_label.set_text(&st.summary());
        self.transfer_label.set_tooltip_text(Some(&gettext_f(
            "{summary} · {retries} retries",
            &[("summary", &st.summary()), ("retries", &st.retries)],
        )));
        match st.fraction() {
            Some(f) => self.transfer_bar.set_fraction(f),
            None => self.transfer_bar.pulse(),
        }
    }

    /// Show progress of something that is not a protocol transfer (a raw file send), or hide it.
    pub fn set_progress(&self, progress: Option<(&str, f64)>) {
        let Some((text, fraction)) = progress else {
            self.transfer.set_visible(false);
            return;
        };
        self.transfer.set_visible(true);
        self.transfer_label.set_text(text);
        self.transfer_label.set_tooltip_text(Some(text));
        self.transfer_bar.set_fraction(fraction.clamp(0.0, 1.0));
    }

    /// The transfer indicator is shown.
    #[cfg(test)]
    pub fn transfer_visible(&self) -> bool {
        self.transfer.is_visible()
    }

    /// Show a session status.
    pub fn update(&self, st: &SessionStatus) {
        self.profile.set_label(&format!("[{}]", st.name));
        if st.description.is_empty() {
            self.endpoint.set_text(&gettext("No open port"));
        } else {
            self.endpoint.set_text(&st.description);
        }
        self.endpoint.set_tooltip_text(Some(&st.description));
        let (text, class) = match &st.state {
            PortState::Open => (gettext("Connected"), Some("state-open")),
            PortState::Opening => (gettext("Connecting…"), None),
            PortState::Closed => (gettext("Disconnected"), None),
            PortState::Error(e) => (gettext_f("Error: {error}", &[("error", e)]), Some("state-error")),
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
            gettext_f(" · queue {count}", &[("count", &st.tx_queue)])
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
                let paused = if paused {
                    gettext(" (paused)")
                } else {
                    String::new()
                };
                self.log.set_text(&format!("● LOG{paused} {}", human(l.bytes)));
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
            // The level is shown by opacity only; tell assistive technology (and the tooltip).
            let state = if on { gettext("high") } else { gettext("low") };
            let text = format!("{}: {state}", l.label());
            l.set_tooltip_text(Some(&text));
            l.update_property(&[gtk::accessible::Property::Description(&text)]);
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
        self.activity.set_tooltip_text(
            running
                .map(|n| gettext_f("Macro {name} is running", &[("name", &n)]))
                .as_deref(),
        );
    }

    /// Show the embedded MCP endpoint state.
    pub fn set_mcp(&self, url: Option<&str>) {
        match url {
            Some(u) => {
                self.mcp.set_text("◆ MCP");
                self.mcp
                    .set_tooltip_text(Some(&gettext_f("MCP server listening on {url}", &[("url", &u)])));
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
