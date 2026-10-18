//! The send bar under the terminal: a line of text (escapes) or hex bytes with a line
//! ending, a history, and optional repeating.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtkterm_core::session::Session;

use super::i18n::{gettext, gettext_f};
use super::rt::spawn;
use super::state::UiState;

/// Line endings in the order of the dropdown.
const ENDINGS: [&[u8]; 4] = [b"", b"\r", b"\n", b"\r\n"];
/// History entries kept.
const HISTORY: usize = 50;

/// The send bar of one tab.
pub struct SendBar {
    /// Top-level widget.
    pub widget: gtk::Box,
    entry: gtk::Entry,
    ending: gtk::DropDown,
    hex: gtk::ToggleButton,
    every: gtk::SpinButton,
    repeat: gtk::ToggleButton,
    session: Session,
    state: Rc<RefCell<UiState>>,
    /// Position while browsing the history with Up/Down (None: editing a new line).
    browsing: Cell<Option<usize>>,
    repeating: RefCell<Option<glib::SourceId>>,
    /// Shows errors (invalid escapes, send failures).
    on_error: Rc<dyn Fn(&str)>,
}

impl SendBar {
    /// Build the bar for `session`; `on_error` shows a message.
    pub fn new(
        session: &Session,
        state: &Rc<RefCell<UiState>>,
        on_error: impl Fn(&str) + 'static,
    ) -> Rc<Self> {
        let entry = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text(gettext(
                "Text to send (\\r \\n \\t \\xHH work), Enter sends, Up/Down: history",
            ))
            .build();
        let ending = gtk::DropDown::from_strings(&[&gettext("No line ending"), "CR", "LF", "CR LF"]);
        ending.set_tooltip_text(Some(&gettext("Appended to every send")));
        ending.set_selected(state.borrow().send_ending.min(3));
        let hex = gtk::ToggleButton::builder()
            .label("Hex")
            .tooltip_text(gettext("The text is hex bytes (DE AD, 0x01, \\x41)"))
            .build();
        let every = gtk::SpinButton::with_range(10.0, 3_600_000.0, 10.0);
        every.set_value(f64::from(state.borrow().send_every_ms.max(10)));
        every.set_tooltip_text(Some(&gettext("Repeat interval (ms)")));
        let repeat = gtk::ToggleButton::builder()
            .icon_name("media-playlist-repeat-symbolic")
            .tooltip_text(gettext("Send repeatedly at this interval"))
            .build();
        let send = gtk::Button::builder()
            .label(gettext("Send"))
            .css_classes(["suggested-action"])
            .build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        widget.add_css_class("send-bar");
        for w in [
            entry.upcast_ref::<gtk::Widget>(),
            hex.upcast_ref(),
            ending.upcast_ref(),
            every.upcast_ref(),
            repeat.upcast_ref(),
            send.upcast_ref(),
        ] {
            widget.append(w);
        }
        let bar = Rc::new(Self {
            widget,
            entry,
            ending,
            hex,
            every,
            repeat,
            session: session.clone(),
            state: Rc::clone(state),
            browsing: Cell::new(None),
            repeating: RefCell::new(None),
            on_error: Rc::new(on_error),
        });

        let b = Rc::downgrade(&bar);
        bar.entry.connect_activate(move |_| {
            if let Some(b) = b.upgrade() {
                b.send_now(true);
            }
        });
        let b = Rc::downgrade(&bar);
        send.connect_clicked(move |_| {
            if let Some(b) = b.upgrade() {
                b.send_now(true);
            }
        });
        let keys = gtk::EventControllerKey::new();
        let b = Rc::downgrade(&bar);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(b) = b.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gtk::gdk::Key::Up => b.browse(1),
                gtk::gdk::Key::Down => b.browse(-1),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        bar.entry.add_controller(keys);
        let b = Rc::downgrade(&bar);
        bar.repeat.connect_toggled(move |t| {
            if let Some(b) = b.upgrade() {
                b.set_repeating(t.is_active());
            }
        });
        let st = Rc::clone(state);
        bar.ending
            .connect_selected_notify(move |d| st.borrow_mut().send_ending = d.selected());
        let st = Rc::clone(state);
        bar.every
            .connect_value_changed(move |s| st.borrow_mut().send_every_ms = s.value() as u32);
        bar
    }

    /// The bytes the bar would send now.
    fn bytes(&self) -> Result<Vec<u8>, String> {
        let text = self.entry.text();
        let mut data = if self.hex.is_active() {
            gtkterm_protocols::hex::parse_hex(&text)
                .map_err(|e| gettext_f("Invalid hex: {error}", &[("error", &e)]))?
        } else {
            gtkterm_protocols::escape::decode(&text).map_err(|e| e.to_string())?
        };
        data.extend_from_slice(ENDINGS[(self.ending.selected() as usize).min(3)]);
        Ok(data)
    }

    /// Send the current line; `remember` puts it into the history.
    fn send_now(&self, remember: bool) {
        let data = match self.bytes() {
            Ok(d) => d,
            Err(e) => {
                (self.on_error)(&e);
                self.stop_repeating();
                return;
            }
        };
        if data.is_empty() {
            return;
        }
        if remember {
            let text = self.entry.text().to_string();
            if !text.is_empty() {
                let mut st = self.state.borrow_mut();
                st.send_history.retain(|h| *h != text);
                st.send_history.insert(0, text);
                st.send_history.truncate(HISTORY);
            }
            self.browsing.set(None);
        }
        let s = self.session.clone();
        let on_error = Rc::clone(&self.on_error);
        spawn(
            async move { s.send(data).await.map_err(|e| e.to_string()) },
            move |r| {
                if let Err(e) = r {
                    on_error(&e);
                }
            },
        );
    }

    /// Up (`1`) / Down (`-1`) through the history.
    fn browse(&self, step: i32) {
        let history = self.state.borrow().send_history.clone();
        if history.is_empty() {
            return;
        }
        let next = match (self.browsing.get(), step > 0) {
            (None, true) => Some(0),
            (Some(i), true) => Some((i + 1).min(history.len() - 1)),
            (None | Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
        };
        self.browsing.set(next);
        self.entry.set_text(next.map_or("", |i| history[i].as_str()));
        self.entry.set_position(-1);
    }

    fn set_repeating(self: &Rc<Self>, on: bool) {
        self.stop_timer();
        if !on {
            return;
        }
        if let Err(e) = self.bytes() {
            (self.on_error)(&e);
            self.repeat.set_active(false);
            return;
        }
        self.send_now(true);
        let ms = self.every.value().max(10.0) as u64;
        let b = Rc::downgrade(self);
        let id = glib::timeout_add_local(Duration::from_millis(ms), move || {
            let Some(b) = b.upgrade() else {
                return glib::ControlFlow::Break;
            };
            b.send_now(false);
            glib::ControlFlow::Continue
        });
        *self.repeating.borrow_mut() = Some(id);
    }

    fn stop_timer(&self) {
        if let Some(id) = self.repeating.borrow_mut().take() {
            id.remove();
        }
    }

    /// Stop a running repeat (e.g. when the connection closes).
    pub fn stop_repeating(&self) {
        self.stop_timer();
        if self.repeat.is_active() {
            self.repeat.set_active(false);
        }
    }

    /// Focus the text field.
    pub fn focus(&self) {
        self.entry.grab_focus();
    }

    /// Fill in a line (tests).
    #[cfg(test)]
    pub fn set_line(&self, text: &str, hex: bool, ending: u32) {
        self.entry.set_text(text);
        self.hex.set_active(hex);
        self.ending.set_selected(ending);
    }

    /// Press Enter (tests).
    #[cfg(test)]
    pub fn activate(&self) {
        self.send_now(true);
    }

    /// Start or stop repeating (tests).
    #[cfg(test)]
    pub fn set_repeat(self: &Rc<Self>, every_ms: f64, on: bool) {
        self.every.set_value(every_ms);
        self.repeat.set_active(on);
    }
}

impl Drop for SendBar {
    fn drop(&mut self) {
        self.stop_timer();
    }
}
