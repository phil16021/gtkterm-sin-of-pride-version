//! Macro editor, shortcut validation and keystroke recording.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use adw::prelude::*;
use gtk::glib;
use gtkterm_core::macros::{Macro, MacroCommand, format_text, parse_text};
use gtkterm_core::manager::SessionManager;
use gtkterm_protocols::escape;
use sourceview::prelude::*;

use super::shortcuts::SHORTCUTS;

/// Keystrokes closer together than this are merged into one `send` step.
const MERGE_MS: u64 = 300;

/// Check that `accel` (GTK accelerator syntax) can be used as a macro shortcut.
pub fn validate_shortcut(accel: &str) -> Result<String, String> {
    let Some((key, mods)) = gtk::accelerator_parse(accel) else {
        return Err(format!("{accel:?} is not a valid shortcut"));
    };
    let name = gtk::accelerator_name(key, mods).to_string();
    let mods = mods & gtk::accelerator_get_default_mod_mask();
    let key_name = key.name().map(|n| n.to_string()).unwrap_or_default();
    let is_fkey = key_name.starts_with('F') && key_name[1..].parse::<u8>().is_ok();
    if mods.is_empty() && !is_fkey {
        return Err(format!(
            "{name}: a key without modifier would stop that key from reaching the device"
        ));
    }
    if mods == gtk::gdk::ModifierType::CONTROL_MASK
        && key
            .to_unicode()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '[' || c == ']' || c == '\\')
    {
        return Err(format!(
            "{name}: Ctrl+letter sends a control character to the device"
        ));
    }
    if mods.contains(gtk::gdk::ModifierType::SUPER_MASK) && key_name == "space" {
        return Err("Super+Space switches the keyboard layout".into());
    }
    for (_, title, _, app_accel) in SHORTCUTS {
        if let Some((k, m)) = gtk::accelerator_parse(*app_accel)
            && gtk::accelerator_name(k, m) == name
        {
            return Err(format!("{name} is already used for “{title}”"));
        }
    }
    Ok(name)
}

/// Human readable form of an accelerator.
pub fn shortcut_label(accel: &str) -> String {
    gtk::accelerator_parse(accel).map_or_else(
        || accel.to_owned(),
        |(k, m)| gtk::accelerator_get_label(k, m).to_string(),
    )
}

/// Collects sent data into macro steps.
#[derive(Debug, Default)]
pub struct Recorder {
    last: Option<Instant>,
    commands: Vec<MacroCommand>,
}

impl Recorder {
    /// Record bytes that were just sent.
    pub fn push(&mut self, data: &[u8]) {
        self.push_at(data, Instant::now());
    }

    fn push_at(&mut self, data: &[u8], now: Instant) {
        let delay = self.last.map_or(0, |t| now.duration_since(t).as_millis() as u64);
        self.last = Some(now);
        let text = escape::encode(data);
        match self.commands.last_mut() {
            Some(MacroCommand::Send { data: prev, .. }) if delay < MERGE_MS => prev.push_str(&text),
            _ => self.commands.push(MacroCommand::Send {
                data: text,
                delay_ms: if self.commands.is_empty() { 0 } else { delay },
            }),
        }
    }

    /// Steps recorded so far.
    pub fn finish(self) -> Vec<MacroCommand> {
        self.commands
    }
}

fn language_manager() -> sourceview::LanguageManager {
    let lm = sourceview::LanguageManager::default();
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(|b| b.parent())
    {
        dirs.push(prefix.join("share/gtkterm-sunshine/language-specs"));
    }
    dirs.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/language-specs"));
    let current: Vec<String> = lm.search_path().iter().map(ToString::to_string).collect();
    for d in dirs.into_iter().filter(|d| d.is_dir()) {
        let d = d.to_string_lossy().into_owned();
        if !current.contains(&d) {
            lm.append_search_path(&d);
        }
    }
    lm
}

/// The macro editor dialog.
pub struct MacroEditor {
    /// The dialog.
    pub dialog: adw::Dialog,
    manager: Arc<SessionManager>,
    list: gtk::ListBox,
    name: adw::EntryRow,
    shortcut_button: gtk::Button,
    shortcut: RefCell<Option<String>>,
    capturing: Cell<bool>,
    buffer: sourceview::Buffer,
    error: gtk::Label,
    save: gtk::Button,
    selected: RefCell<Option<String>>,
    run: Box<dyn Fn(&str)>,
}

impl MacroEditor {
    /// Open the editor; `select` chooses a macro, `run` plays a saved macro by name.
    pub fn present(
        parent: &impl IsA<gtk::Widget>,
        manager: Arc<SessionManager>,
        select: Option<String>,
        run: impl Fn(&str) + 'static,
        stop: impl Fn() + 'static,
    ) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .min_content_width(200)
            .build();
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("New macro")
            .build();
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .tooltip_text("Delete macro")
            .build();
        let list_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        list_buttons.append(&add);
        list_buttons.append(&remove);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
        sidebar.append(&scroller);
        sidebar.append(&list_buttons);

        let name = adw::EntryRow::builder().title("Name").build();
        let shortcut_button = gtk::Button::builder()
            .label("None")
            .valign(gtk::Align::Center)
            .tooltip_text("Click, then press the key combination (Backspace clears)")
            .build();
        let shortcut_row = adw::ActionRow::builder().title("Shortcut").build();
        shortcut_row.add_suffix(&shortcut_button);
        let fields = adw::PreferencesGroup::new();
        fields.add(&name);
        fields.add(&shortcut_row);

        let buffer = sourceview::Buffer::new(None);
        if let Some(lang) = language_manager().language("gtkterm-macro") {
            buffer.set_language(Some(&lang));
        }
        let scheme = if adw::StyleManager::default().is_dark() {
            "Adwaita-dark"
        } else {
            "Adwaita"
        };
        if let Some(s) = sourceview::StyleSchemeManager::default().scheme(scheme) {
            buffer.set_style_scheme(Some(&s));
        }
        let view = sourceview::View::with_buffer(&buffer);
        view.set_show_line_numbers(true);
        view.set_monospace(true);
        view.set_highlight_current_line(true);
        view.set_top_margin(6);
        view.set_left_margin(6);
        let editor = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hexpand(true)
            .min_content_height(220)
            .build();
        editor.add_css_class("card");
        let help = gtk::Label::builder()
            .label("send: ATZ\\r   ·   send[250]: text (pause first)   ·   wait: 500 ms   ·   expect[2000]: OK\\r\\n   ·   # comment\nEscapes: \\r \\n \\t \\0 \\e \\\\ \\xHH — expect takes a regular expression")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .build();
        let error = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["error"])
            .build();
        let run_button = gtk::Button::builder()
            .label("Run")
            .tooltip_text("Save and run on the current connection")
            .build();
        let stop_button = gtk::Button::builder().label("Stop").build();
        let save = gtk::Button::builder()
            .label("Save")
            .css_classes(["suggested-action"])
            .build();
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        actions.append(&stop_button);
        actions.append(&run_button);
        actions.append(&save);
        let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
        form.append(&fields);
        form.append(&editor);
        form.append(&help);
        form.append(&error);
        form.append(&actions);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        for m in [&sidebar, &form] {
            m.set_margin_top(12);
            m.set_margin_bottom(12);
        }
        sidebar.set_margin_start(12);
        form.set_margin_end(12);
        form.set_hexpand(true);
        content.append(&sidebar);
        content.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        content.append(&form);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&content));
        let dialog = adw::Dialog::builder()
            .title("Macros")
            .content_width(860)
            .content_height(560)
            .child(&toolbar)
            .build();

        let me = Rc::new(Self {
            dialog,
            manager,
            list,
            name,
            shortcut_button,
            shortcut: RefCell::default(),
            capturing: Cell::new(false),
            buffer,
            error,
            save,
            selected: RefCell::default(),
            run: Box::new(run),
        });
        me.reload_list(select.as_deref());

        let w = Rc::downgrade(&me);
        me.list.connect_row_selected(move |_, row| {
            if let (Some(me), Some(row)) = (w.upgrade(), row) {
                me.load(&row.widget_name());
            }
        });
        let w = Rc::downgrade(&me);
        add.connect_clicked(move |_| {
            let Some(me) = w.upgrade() else { return };
            let name = me.manager.with_macros(|s| s.fresh_name("New macro"));
            let m = Macro {
                name: name.clone(),
                shortcut: None,
                commands: vec![MacroCommand::Send {
                    data: String::new(),
                    delay_ms: 0,
                }],
            };
            match me.manager.update_macros(|s| s.upsert(None, m)) {
                Ok(()) => me.reload_list(Some(&name)),
                Err(e) => me.error.set_text(&e.to_string()),
            }
        });
        let w = Rc::downgrade(&me);
        remove.connect_clicked(move |_| {
            let Some(me) = w.upgrade() else { return };
            let Some(name) = me.selected.borrow().clone() else {
                return;
            };
            let _ = me.manager.update_macros(|s| Ok(s.remove(&name)));
            me.reload_list(None);
        });
        let w = Rc::downgrade(&me);
        me.buffer.connect_changed(move |_| {
            if let Some(me) = w.upgrade() {
                me.check();
            }
        });
        let w = Rc::downgrade(&me);
        me.save.connect_clicked(move |_| {
            if let Some(me) = w.upgrade() {
                me.store();
            }
        });
        let w = Rc::downgrade(&me);
        run_button.connect_clicked(move |_| {
            let Some(me) = w.upgrade() else { return };
            if let Some(name) = me.store() {
                (me.run)(&name);
            }
        });
        stop_button.connect_clicked(move |_| stop());
        let w = Rc::downgrade(&me);
        me.shortcut_button.connect_clicked(move |b| {
            if let Some(me) = w.upgrade() {
                me.capturing.set(true);
                b.set_label("Press keys…");
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&me);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(me) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !me.capturing.get() {
                return glib::Propagation::Proceed;
            }
            let name = key.name().map(|n| n.to_string()).unwrap_or_default();
            if [
                "Shift_L",
                "Shift_R",
                "Control_L",
                "Control_R",
                "Alt_L",
                "Alt_R",
                "Super_L",
                "Super_R",
                "ISO_Level3_Shift",
            ]
            .contains(&name.as_str())
            {
                return glib::Propagation::Stop;
            }
            me.capturing.set(false);
            match name.as_str() {
                "Escape" => {}
                "BackSpace" => *me.shortcut.borrow_mut() = None,
                _ => {
                    let accel = gtk::accelerator_name(key, state & gtk::accelerator_get_default_mod_mask())
                        .to_string();
                    match validate_shortcut(&accel) {
                        Ok(a) => *me.shortcut.borrow_mut() = Some(a),
                        Err(e) => me.error.set_text(&e),
                    }
                }
            }
            me.show_shortcut();
            glib::Propagation::Stop
        });
        me.dialog.add_controller(keys);
        me.dialog.present(Some(parent));
        me
    }

    fn reload_list(&self, select: Option<&str>) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let macros: Vec<(String, Option<String>)> = self.manager.with_macros(|s| {
            s.list()
                .iter()
                .map(|m| (m.name.clone(), m.shortcut.clone()))
                .collect()
        });
        let mut target = None;
        for (i, (name, shortcut)) in macros.iter().enumerate() {
            let label = gtk::Label::builder()
                .label(name)
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .hexpand(true)
                .build();
            let key = gtk::Label::builder()
                .label(shortcut.as_deref().map(shortcut_label).unwrap_or_default())
                .css_classes(["dim-label"])
                .build();
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            b.append(&label);
            b.append(&key);
            let row = gtk::ListBoxRow::builder().child(&b).name(name).build();
            self.list.append(&row);
            if select == Some(name.as_str()) || (select.is_none() && i == 0) {
                target = Some(row);
            }
        }
        match target {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.clear_form(),
        }
    }

    fn clear_form(&self) {
        self.selected.replace(None);
        self.name.set_text("");
        self.buffer.set_text("");
        self.shortcut.replace(None);
        self.show_shortcut();
        self.save.set_sensitive(false);
    }

    fn load(&self, name: &str) {
        let Some(m) = self.manager.with_macros(|s| s.get(name).cloned()) else {
            return;
        };
        self.selected.replace(Some(m.name.clone()));
        self.name.set_text(&m.name);
        self.buffer.set_text(&format_text(&m.commands));
        self.shortcut.replace(m.shortcut.clone());
        self.show_shortcut();
        self.error.set_text("");
    }

    fn show_shortcut(&self) {
        let label = self
            .shortcut
            .borrow()
            .as_deref()
            .map_or_else(|| "None".to_owned(), shortcut_label);
        self.shortcut_button.set_label(&label);
    }

    fn text(&self) -> String {
        let (start, end) = self.buffer.bounds();
        self.buffer.text(&start, &end, false).to_string()
    }

    fn check(&self) -> Option<Vec<MacroCommand>> {
        match parse_text(&self.text()) {
            Ok(c) => {
                self.error.set_text("");
                self.save.set_sensitive(self.selected.borrow().is_some());
                Some(c)
            }
            Err(e) => {
                self.error.set_text(&e.to_string());
                self.save.set_sensitive(false);
                None
            }
        }
    }

    /// Save the form; returns the saved name.
    fn store(&self) -> Option<String> {
        let commands = self.check()?;
        let old = self.selected.borrow().clone()?;
        let name = self.name.text().trim().to_owned();
        let shortcut = self.shortcut.borrow().clone();
        if let Some(s) = &shortcut
            && let Err(e) = validate_shortcut(s)
        {
            self.error.set_text(&e);
            return None;
        }
        let m = Macro {
            name: name.clone(),
            shortcut,
            commands,
        };
        match self.manager.update_macros(|s| s.upsert(Some(&old), m)) {
            Ok(()) => {
                self.reload_list(Some(&name));
                Some(name)
            }
            Err(e) => {
                self.error.set_text(&e.to_string());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[gtk::test]
    fn shortcut_rules() {
        // Needs a display (GTK keyval tables); runs with the other GUI tests.
        if std::env::var_os("GTKTERM_GUI_TESTS").is_none() {
            return;
        }
        assert_eq!(validate_shortcut("F9").as_deref(), Ok("F9"));
        assert_eq!(
            validate_shortcut("<Control><Shift>1").as_deref(),
            Ok("<Shift><Control>1")
        );
        assert_eq!(validate_shortcut("<Alt>m").as_deref(), Ok("<Alt>m"));
        for bad in [
            "a",
            "<Control>c",
            "<Control>bracketleft",
            "F5",
            "<Control><Shift>c",
            "<Super>space",
            "nonsense<",
        ] {
            assert!(validate_shortcut(bad).is_err(), "{bad} must be rejected");
        }
        assert!(validate_shortcut("F5").unwrap_err().contains("Connect"));
    }

    #[test]
    fn recorder_merges_bursts_and_keeps_delays() {
        let mut r = Recorder::default();
        let t0 = Instant::now();
        r.push_at(b"A", t0);
        r.push_at(b"T", t0 + Duration::from_millis(80));
        r.push_at(b"\r", t0 + Duration::from_millis(150));
        r.push_at(b"\x01", t0 + Duration::from_millis(1150));
        assert_eq!(
            r.finish(),
            vec![
                MacroCommand::Send {
                    data: "AT\\r".into(),
                    delay_ms: 0
                },
                MacroCommand::Send {
                    data: "\\x01".into(),
                    delay_ms: 1000
                },
            ]
        );
    }
}
