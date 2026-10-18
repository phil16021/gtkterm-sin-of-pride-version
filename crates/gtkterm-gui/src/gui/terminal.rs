//! The terminal view: VTE plus formatting of received data.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtkterm_core::chunks::{Chunk, Direction};
use gtkterm_core::config::TerminalConfig;
use gtkterm_core::session::Session;
use gtkterm_protocols::transform::{Transform, TransformOptions};
use gtkterm_protocols::view::{ViewFormatter, ViewMode, ViewSettings};
use vte::prelude::*;

use super::i18n::gettext;

/// PCRE2 compile flags used by VTE searches.
const PCRE2_CASELESS: u32 = 0x0000_0008;
const PCRE2_MULTILINE: u32 = 0x0000_0400;

/// How often queued output is handed to VTE.
const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);
/// Bytes fed to VTE per flush; the rest waits for the next one.
const FEED_BUDGET: usize = 64 * 1024;
/// Queued output beyond this is dropped (oldest first) with a notice: the display cannot keep
/// up, but the session history, logs and MCP still have every byte.
const MAX_PENDING: usize = 4 * 1024 * 1024;

/// Output waiting for the next flush.
#[derive(Default)]
struct Pending {
    bytes: RefCell<Vec<u8>>,
    scheduled: std::cell::Cell<bool>,
    skipped: std::cell::Cell<usize>,
}

impl Pending {
    fn push(&self, data: &[u8]) {
        let mut b = self.bytes.borrow_mut();
        b.extend_from_slice(data);
        if b.len() > MAX_PENDING {
            // Keep the newest half, starting at a line break.
            let cut = b.len() - MAX_PENDING / 2;
            let cut = b[cut..]
                .iter()
                .position(|&c| c == b'\n')
                .map_or(cut, |i| cut + i + 1);
            b.drain(..cut);
            self.skipped.set(self.skipped.get() + cut);
        }
    }

    /// Feed up to [`FEED_BUDGET`] bytes; true if more is waiting.
    fn flush(&self, term: &vte::Terminal) -> bool {
        let skipped = self.skipped.replace(0);
        if skipped > 0 {
            let text = super::i18n::gettext_f(
                "{bytes} bytes not shown: the display could not keep up",
                &[("bytes", &skipped)],
            );
            term.feed(format!("\r\n\x1b[2m[{text}]\x1b[0m\r\n").as_bytes());
        }
        let mut b = self.bytes.borrow_mut();
        let n = b.len().min(FEED_BUDGET);
        term.feed(&b[..n]);
        b.drain(..n);
        !b.is_empty()
    }
}

/// Terminal widget with its formatting state.
pub struct TerminalView {
    /// Top-level widget (search bar + terminal + scrollbar).
    pub widget: gtk::Box,
    /// The VTE widget.
    pub term: vte::Terminal,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_case: gtk::CheckButton,
    search_regex: gtk::CheckButton,
    search_hex: gtk::CheckButton,
    config: RefCell<TerminalConfig>,
    /// Called with every chunk of keyboard/paste input (macro recording).
    pub on_input: RefCell<Option<Box<dyn Fn(&[u8])>>>,
    formatter: RefCell<ViewFormatter>,
    transform: RefCell<Transform>,
    pending: Rc<Pending>,
    /// When the last "input not sent" notice was shown (one per burst of typing).
    refused_at: std::cell::Cell<Option<std::time::Instant>>,
}

/// Why typed input cannot be sent right now, if it cannot.
fn input_refused(session: &Session) -> Option<String> {
    use gtkterm_core::session::PortState;
    use gtkterm_core::transfer::TransferState;
    let st = session.status();
    if !matches!(st.state, PortState::Open) {
        return Some(gettext("Not connected: typed input is not sent (F5 connects)"));
    }
    if session
        .transfer_status()
        .is_some_and(|t| t.state == TransferState::Running)
    {
        return Some(gettext("A file transfer is running: typed input is not sent"));
    }
    None
}

fn rgba(c: gtkterm_core::config::Rgba) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(c.red as f32, c.green as f32, c.blue as f32, c.alpha as f32)
}

fn transform_for(c: &TerminalConfig) -> Transform {
    Transform::new(
        TransformOptions {
            auto_cr: c.auto_cr,
            auto_lf: c.auto_lf,
            timestamps: c.timestamps,
            timestamp_format: c.timestamp_format,
        },
        jiff::tz::TimeZone::system(),
        jiff::Timestamp::now(),
    )
}

impl TerminalView {
    /// Build the view; keystrokes are sent to `session`.
    pub fn new(session: &Session, config: &TerminalConfig) -> Rc<Self> {
        let term = vte::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        term.set_scroll_on_output(true);
        term.set_scroll_on_keystroke(true);
        term.set_mouse_autohide(true);
        term.set_backspace_binding(vte::EraseBinding::AsciiBackspace);
        term.set_enable_bidi(false);

        let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, term.vadjustment().as_ref());
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&term);
        row.append(&scrollbar);

        let search_entry = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Search text, regex or hex bytes"))
            .hexpand(true)
            .build();
        let search_case = gtk::CheckButton::with_label(&gettext("Case"));
        let search_regex = gtk::CheckButton::with_label(&gettext("Regex"));
        let search_hex = gtk::CheckButton::with_label(&gettext("Hex"));
        let prev = gtk::Button::from_icon_name("go-up-symbolic");
        prev.set_tooltip_text(Some(&gettext("Previous match")));
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some(&gettext("Next match (Enter)")));
        search_entry.set_width_chars(8);
        let options = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for w in [&search_case, &search_regex, &search_hex] {
            options.append(w);
        }
        let options_button = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(gettext("Search options"))
            .popover(&gtk::Popover::builder().child(&options).build())
            .build();
        let search_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for w in [
            search_entry.upcast_ref::<gtk::Widget>(),
            options_button.upcast_ref(),
            prev.upcast_ref(),
            next.upcast_ref(),
        ] {
            search_box.append(w);
        }
        let search_bar = gtk::SearchBar::builder()
            .child(&search_box)
            .show_close_button(true)
            .build();
        search_bar.connect_entry(&search_entry);

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("terminal-box");
        widget.append(&search_bar);
        widget.append(&row);

        let view = Rc::new(Self {
            widget,
            term,
            search_bar,
            search_entry,
            search_case,
            search_regex,
            search_hex,
            config: RefCell::new(config.clone()),
            on_input: RefCell::new(None),
            formatter: RefCell::new(ViewFormatter::default()),
            transform: RefCell::new(transform_for(config)),
            pending: Rc::default(),
            refused_at: std::cell::Cell::new(None),
        });
        view.apply_config(config);

        // Visual bell: flash the terminal (VTE only beeps).
        let v = Rc::downgrade(&view);
        view.term.connect_bell(move |t| {
            let Some(v) = v.upgrade() else { return };
            if !v.config.borrow().visual_bell || t.has_css_class("bell-flash") {
                return;
            }
            t.add_css_class("bell-flash");
            let t = t.downgrade();
            glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                if let Some(t) = t.upgrade() {
                    t.remove_css_class("bell-flash");
                }
            });
        });

        let s = session.clone();
        let v = Rc::downgrade(&view);
        view.term.connect_commit(move |_, text, _| {
            let Some(v) = v.upgrade() else { return };
            match input_refused(&s) {
                Some(reason) => v.refused(&reason),
                None => {
                    if let Err(e) = s.send_nowait(text.as_bytes().to_vec()) {
                        v.refused(&e.to_string());
                    }
                }
            }
            if let Some(hook) = v.on_input.borrow().as_ref() {
                hook(text.as_bytes());
            }
        });

        let v = Rc::downgrade(&view);
        view.search_entry.connect_search_changed(move |_| {
            if let Some(v) = v.upgrade() {
                v.find(false);
            }
        });
        let v = Rc::downgrade(&view);
        view.search_entry.connect_activate(move |_| {
            if let Some(v) = v.upgrade() {
                v.find(false);
            }
        });
        for (button, back) in [(&prev, true), (&next, false)] {
            let v = Rc::downgrade(&view);
            button.connect_clicked(move |_| {
                if let Some(v) = v.upgrade() {
                    v.find(back);
                }
            });
        }
        for check in [&view.search_case, &view.search_regex, &view.search_hex] {
            let v = Rc::downgrade(&view);
            check.connect_toggled(move |_| {
                if let Some(v) = v.upgrade() {
                    v.find(false);
                }
            });
        }
        view.install_context_menu();
        view
    }

    fn install_context_menu(&self) {
        let menu = gio::Menu::new();
        menu.append(Some(&gettext("Copy")), Some("win.copy"));
        menu.append(Some(&gettext("Paste")), Some("win.paste"));
        menu.append(Some(&gettext("Select All")), Some("win.select-all"));
        let section = gio::Menu::new();
        section.append(Some(&gettext("Clear Screen")), Some("win.clear-screen"));
        section.append(Some(&gettext("Search…")), Some("win.search"));
        menu.append_section(None, &section);
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&self.term);
        popover.set_has_arrow(false);
        // A popover parented to a widget must be unparented before the widget is finalized.
        let p = popover.clone();
        self.term.connect_destroy(move |_| p.unparent());
        let click = gtk::GestureClick::builder().button(3).build();
        click.connect_pressed(move |_, _, x, y| {
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        });
        self.term.add_controller(click);
    }

    /// Apply terminal settings (font, colours, cursor, transform, view mode).
    pub fn apply_config(&self, c: &TerminalConfig) {
        self.term
            .set_font(Some(&pango::FontDescription::from_string(&c.font)));
        self.term
            .set_colors(Some(&rgba(c.foreground)), Some(&rgba(c.background)), &[]);
        self.term.set_scrollback_lines(i64::from(c.scrollback));
        self.term.set_cursor_shape(if c.block_cursor {
            vte::CursorShape::Block
        } else {
            vte::CursorShape::Ibeam
        });
        self.term.set_cursor_blink_mode(if c.show_cursor {
            vte::CursorBlinkMode::System
        } else {
            vte::CursorBlinkMode::Off
        });
        self.term.set_audible_bell(!c.visual_bell);
        let old = self.config.replace(c.clone());
        if old.auto_cr != c.auto_cr
            || old.auto_lf != c.auto_lf
            || old.timestamps != c.timestamps
            || old.timestamp_format != c.timestamp_format
        {
            self.transform.replace(transform_for(c));
        }
        self.update_view_settings();
    }

    fn update_view_settings(&self) {
        let c = self.config.borrow();
        let columns = usize::try_from(self.term.column_count()).unwrap_or(80).max(10);
        self.formatter.borrow_mut().set_settings(ViewSettings {
            mode: c.view_mode,
            show_index: c.show_index,
            hex_chars: usize::from(c.hex_chars),
            columns,
        });
    }

    /// Current settings.
    pub fn config(&self) -> TerminalConfig {
        self.config.borrow().clone()
    }

    /// Change the view mode.
    pub fn set_view_mode(&self, mode: ViewMode) {
        self.config.borrow_mut().view_mode = mode;
        self.update_view_settings();
        if mode != ViewMode::Ascii {
            self.output(b"\r\n");
        }
    }

    /// Queue `data` for the terminal; it is fed on the next flush (at most every 16 ms), so
    /// fast input costs one VTE update per frame instead of one per chunk.
    fn output(&self, data: &[u8]) {
        self.pending.push(data);
        if self.pending.scheduled.replace(true) {
            return;
        }
        let pending = Rc::clone(&self.pending);
        let term = self.term.downgrade();
        glib::timeout_add_local(FLUSH_INTERVAL, move || {
            let Some(term) = term.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if pending.flush(&term) {
                glib::ControlFlow::Continue
            } else {
                pending.scheduled.set(false);
                glib::ControlFlow::Break
            }
        });
    }

    /// Feed everything queued now.
    #[cfg(test)]
    pub fn flush_output(&self) {
        while self.pending.flush(&self.term) {}
    }

    /// Show a chunk (TX only with local echo).
    pub fn show(&self, chunk: &Chunk) {
        let echo = self.config.borrow().echo;
        if chunk.dir == Direction::Tx && !echo {
            return;
        }
        let mode = self.config.borrow().view_mode;
        let out = if mode == ViewMode::Ascii {
            self.transform.borrow_mut().apply(&chunk.data, chunk.ts)
        } else {
            self.update_view_settings();
            self.formatter.borrow_mut().format(&chunk.data)
        };
        self.output(&out);
    }

    /// A refused-input notice was shown.
    #[cfg(test)]
    pub fn input_was_refused(&self) -> bool {
        self.refused_at.get().is_some()
    }

    /// Typed input was not sent: say so, at most every 3 seconds.
    fn refused(&self, reason: &str) {
        let now = std::time::Instant::now();
        if self
            .refused_at
            .get()
            .is_some_and(|t| now.duration_since(t) < std::time::Duration::from_secs(3))
        {
            return;
        }
        self.refused_at.set(Some(now));
        self.notice(reason);
    }

    /// Write an informational line in a distinct style.
    pub fn notice(&self, text: &str) {
        self.output(format!("\r\n\x1b[2m[{text}]\x1b[0m\r\n").as_bytes());
    }

    /// Clear the screen (and optionally the scrollback).
    pub fn clear(&self, scrollback: bool) {
        self.pending.bytes.borrow_mut().clear();
        self.pending.skipped.set(0);
        self.term.reset(true, scrollback);
        self.formatter.borrow_mut().reset();
        self.transform.borrow_mut().reset();
    }

    /// Copy the selection.
    pub fn copy(&self) {
        self.term.copy_clipboard_format(vte::Format::Text);
    }

    /// Paste into the terminal (sent to the device).
    pub fn paste(&self) {
        self.term.paste_clipboard();
    }

    /// Select everything.
    pub fn select_all(&self) {
        self.term.select_all();
    }

    /// Show the search bar.
    pub fn start_search(&self) {
        self.search_bar.set_search_mode(true);
        self.search_entry.grab_focus();
    }

    fn pattern(&self) -> Option<Result<String, String>> {
        let text = self.search_entry.text().to_string();
        if text.is_empty() {
            return None;
        }
        Some(if self.search_hex.is_active() {
            gtkterm_protocols::hex::parse_hex(&text)
                .map_err(|e| e.to_string())
                .map(|bytes| {
                    let mode = self.config.borrow().view_mode;
                    if mode == ViewMode::Ascii {
                        bytes.iter().map(|b| format!("\\x{b:02x}")).collect()
                    } else {
                        // Dump views show bytes as "AA BB "; search for that text.
                        bytes
                            .iter()
                            .map(|b| format!("{b:02X}"))
                            .collect::<Vec<_>>()
                            .join(r"\s+")
                    }
                })
        } else if self.search_regex.is_active() {
            Ok(text)
        } else {
            Ok(regex::escape(&text))
        })
    }

    fn find(&self, backwards: bool) {
        let Some(pattern) = self.pattern() else {
            self.term.search_set_regex(None, 0);
            self.search_entry.remove_css_class("error");
            return;
        };
        let mut flags = PCRE2_MULTILINE;
        if !self.search_case.is_active() {
            flags |= PCRE2_CASELESS;
        }
        let regex = pattern.ok().and_then(|p| vte::Regex::for_search(&p, flags).ok());
        if let Some(re) = &regex {
            self.search_entry.remove_css_class("error");
            self.term.search_set_regex(Some(re), 0);
            self.term.search_set_wrap_around(true);
            if backwards {
                self.term.search_find_previous();
            } else {
                self.term.search_find_next();
            }
        } else {
            self.search_entry.add_css_class("error");
        }
    }
}
