//! Script editor: edit, open and save `.script` files, run them on the window's session and
//! watch the running line and the `log` output.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use gtkterm_core::manager::{ManagerEvent, SessionManager};
use gtkterm_core::script::{Script, ScriptError, ScriptOptions};
use gtkterm_core::session::Session;
use sourceview::prelude::*;

use super::i18n::{gettext, gettext_f};
use super::rt::spawn;

const TEMPLATE: &str = "# GTKTerm script — see the help button for all commands\n\
                        log Probing the device...\n\
                        send AT\\r\n\
                        expect OK\n\
                        log Device answered\n";

/// `path` with the home directory shown as `~`.
fn home_relative(path: &std::path::Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    match std::env::var_os("HOME").map(std::path::PathBuf::from) {
        Some(home) if !home.as_os_str().is_empty() => match path.strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        },
        _ => path.display().to_string(),
    }
}

/// The script editor window.
pub struct ScriptWindow {
    /// The window.
    pub window: adw::Window,
    manager: Arc<SessionManager>,
    session: Session,
    buffer: sourceview::Buffer,
    view: sourceview::View,
    log: gtk::TextBuffer,
    log_view: gtk::TextView,
    status: gtk::Label,
    run: gtk::Button,
    stop: gtk::Button,
    title: adw::WindowTitle,
    path: RefCell<Option<PathBuf>>,
    running_tag: gtk::TextTag,
    error_tag: gtk::TextTag,
    running: Cell<bool>,
    allow_exec: Box<dyn Fn() -> bool>,
    remember: Box<dyn Fn(PathBuf)>,
    /// The text differs from the file (or from the template).
    dirty: Cell<bool>,
    /// Text is being replaced by the program, not edited.
    loading: Cell<bool>,
}

impl ScriptWindow {
    /// Build the (hidden) window.
    pub fn new(
        parent: &impl IsA<gtk::Window>,
        manager: Arc<SessionManager>,
        session: Session,
        allow_exec: impl Fn() -> bool + 'static,
        remember: impl Fn(PathBuf) + 'static,
    ) -> Rc<Self> {
        let buffer = sourceview::Buffer::new(None);
        if let Some(lang) = super::macros::language_manager().language("gtkterm-script") {
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
        buffer.set_text(TEMPLATE);
        let running_tag = buffer
            .create_tag(
                Some("running"),
                &[("paragraph-background", &"rgba(53,132,228,0.25)")],
            )
            .expect("new tag");
        let error_tag = buffer
            .create_tag(
                Some("error"),
                &[("paragraph-background", &"rgba(224,27,36,0.25)")],
            )
            .expect("new tag");
        let view = sourceview::View::with_buffer(&buffer);
        view.set_show_line_numbers(true);
        view.set_monospace(true);
        view.set_highlight_current_line(true);
        view.set_top_margin(6);
        view.set_left_margin(6);
        view.set_tab_width(4);
        let editor = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .min_content_height(260)
            .build();

        let log = gtk::TextBuffer::new(None);
        let log_view = gtk::TextView::builder()
            .buffer(&log)
            .editable(false)
            .monospace(true)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(4)
            .left_margin(6)
            .build();
        let log_scroll = gtk::ScrolledWindow::builder()
            .child(&log_view)
            .min_content_height(110)
            .build();
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&editor)
            .end_child(&log_scroll)
            .resize_end_child(false)
            .shrink_end_child(false)
            .build();
        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(4)
            .label(gettext("Ready"))
            .build();

        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new(&gettext("Script"), &gettext("unsaved"));
        header.set_title_widget(Some(&title));
        let new = gtk::Button::builder()
            .icon_name("document-new-symbolic")
            .tooltip_text(gettext("New script"))
            .build();
        let open = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(gettext("Open script…"))
            .build();
        let save = gtk::Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text(gettext("Save (Ctrl+S)"))
            .build();
        let save_as = gtk::Button::builder()
            .icon_name("document-save-as-symbolic")
            .tooltip_text(gettext("Save as…"))
            .build();
        let run = gtk::Button::builder()
            .label(gettext("Run"))
            .tooltip_text(gettext("Run on this window's port (Ctrl+Return)"))
            .build();
        run.add_css_class("suggested-action");
        let stop = gtk::Button::builder()
            .label(gettext("Stop"))
            .sensitive(false)
            .build();
        stop.add_css_class("destructive-action");
        let help_text = gtk::Label::builder()
            .label(gtkterm_mcp::SCRIPT_LANGUAGE)
            .selectable(true)
            .wrap(true)
            .xalign(0.0)
            .max_width_chars(90)
            .build();
        help_text.add_css_class("monospace");
        let help_scroll = gtk::ScrolledWindow::builder()
            .child(&help_text)
            .min_content_width(620)
            .min_content_height(460)
            .build();
        let help = gtk::MenuButton::builder()
            .icon_name("help-about-symbolic")
            .tooltip_text(gettext("Language reference"))
            .popover(&gtk::Popover::builder().child(&help_scroll).build())
            .build();
        header.pack_start(&new);
        header.pack_start(&open);
        header.pack_start(&save);
        header.pack_start(&save_as);
        header.pack_end(&help);
        header.pack_end(&stop);
        header.pack_end(&run);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&paned);
        content.append(&status);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));
        let window = adw::Window::builder()
            .title(gettext("Script"))
            .transient_for(parent)
            .default_width(760)
            .default_height(620)
            .hide_on_close(true)
            .content(&toolbar)
            .build();

        let this = Rc::new(Self {
            window,
            manager,
            session,
            buffer,
            view,
            log,
            log_view,
            status,
            run,
            stop,
            title,
            path: RefCell::default(),
            running_tag,
            error_tag,
            running: Cell::new(false),
            allow_exec: Box::new(allow_exec),
            remember: Box::new(remember),
            dirty: Cell::new(false),
            loading: Cell::new(false),
        });
        let w = Rc::downgrade(&this);
        this.run.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.start();
            }
        });
        let w = Rc::downgrade(&this);
        this.stop.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.manager.stop_macro(w.session.id());
            }
        });
        let w = Rc::downgrade(&this);
        open.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.unless_unsaved(Self::choose_open);
            }
        });
        let w = Rc::downgrade(&this);
        new.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.unless_unsaved(Self::new_script);
            }
        });
        let w = Rc::downgrade(&this);
        this.window.connect_close_request(move |win| {
            let Some(w) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !w.dirty.get() {
                return glib::Propagation::Proceed;
            }
            let win = win.clone();
            w.unless_unsaved(move |w: &Rc<Self>| {
                w.mark_dirty(false);
                // After the question has gone: closing while it is still shown would only
                // close the question.
                let win = win.clone();
                glib::idle_add_local_once(move || {
                    for _ in 0..4 {
                        let Some(d) = win.visible_dialog() else { break };
                        d.force_close();
                    }
                    win.close();
                });
            });
            glib::Propagation::Stop
        });
        let w = Rc::downgrade(&this);
        save.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.save(false);
            }
        });
        let w = Rc::downgrade(&this);
        save_as.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.save(true);
            }
        });
        let keys = gtk::ShortcutController::new();
        let w = Rc::downgrade(&this);
        keys.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>Return"),
            Some(gtk::CallbackAction::new(move |_, _| {
                if let Some(w) = w.upgrade() {
                    w.start();
                }
                glib::Propagation::Stop
            })),
        ));
        let w = Rc::downgrade(&this);
        keys.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>s"),
            Some(gtk::CallbackAction::new(move |_, _| {
                if let Some(w) = w.upgrade() {
                    w.save(false);
                }
                glib::Propagation::Stop
            })),
        ));
        this.window.add_controller(keys);
        let w = Rc::downgrade(&this);
        this.buffer.connect_changed(move |b| {
            if let Some(w) = w.upgrade() {
                b.remove_tag(&w.error_tag, &b.start_iter(), &b.end_iter());
                if !w.loading.get() {
                    w.mark_dirty(true);
                }
            }
        });
        this
    }

    /// Show the window.
    pub fn present(&self) {
        self.window.present();
    }

    /// Script text.
    pub fn text(&self) -> String {
        self.buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), false)
            .into()
    }

    /// Replace the text (tests).
    #[cfg(test)]
    pub fn set_text(&self, text: &str) {
        self.buffer.set_text(text);
    }

    /// Open `path` (from the command line or the file manager), asking first if the current
    /// script has unsaved changes.
    pub fn open_path(self: &Rc<Self>, path: PathBuf) {
        self.unless_unsaved(move |w: &Rc<Self>| {
            if let Err(e) = w.load(&path) {
                w.set_status(&e, true);
            }
            w.present();
        });
    }

    /// Load a file into the editor.
    pub fn load(&self, path: &std::path::Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.replace_text(&text);
        self.set_path(path.to_owned());
        self.mark_dirty(false);
        let path_name = path.display();
        self.set_status(&gettext_f("Opened {path}", &[("path", &path_name)]), false);
        Ok(())
    }

    /// Replace the text without marking it as edited.
    fn replace_text(&self, text: &str) {
        self.loading.set(true);
        self.buffer.set_text(text);
        self.loading.set(false);
    }

    /// Start over with the template.
    fn new_script(self: &Rc<Self>) {
        self.replace_text(TEMPLATE);
        self.path.borrow_mut().take();
        self.title.set_subtitle(&gettext("unsaved"));
        self.mark_dirty(false);
    }

    /// The text has (or no longer has) changes that are not saved: shown as "• name".
    fn mark_dirty(&self, dirty: bool) {
        self.dirty.set(dirty);
        let name = self
            .path
            .borrow()
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| gettext("Script"), |n| n.to_string_lossy().into_owned());
        self.title
            .set_title(&if dirty { format!("• {name}") } else { name });
    }

    /// Run `then` now, or after the user agreed to discard unsaved changes.
    fn unless_unsaved(self: &Rc<Self>, then: impl Fn(&Rc<Self>) + 'static) {
        if !self.dirty.get() {
            then(self);
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Discard unsaved changes?"))
            .body(gettext("The script has changes that are not saved."))
            .close_response("cancel")
            .default_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", gettext("Cancel").as_str()),
            ("discard", gettext("Discard").as_str()),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        let w = Rc::downgrade(self);
        dialog.connect_response(None, move |_, resp| {
            if let (Some(w), "discard") = (w.upgrade(), resp) {
                then(&w);
            }
        });
        dialog.present(Some(&self.window));
    }

    /// Unsaved changes are kept in a recovery file when the window goes away (application
    /// quit); returns where.
    pub fn keep_unsaved(&self) -> Option<PathBuf> {
        if !self.dirty.get() {
            return None;
        }
        let path = gtkterm_core::config::config_dir().join("unsaved.script");
        match gtkterm_core::fsutil::write_atomic(&path, self.text().as_bytes(), Some(0o600)) {
            Ok(()) => Some(path),
            Err(e) => {
                tracing::warn!(error = %e, "cannot keep the unsaved script");
                None
            }
        }
    }

    /// The text has unsaved changes.
    #[cfg(test)]
    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn set_path(&self, path: PathBuf) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.title.set_title(&name);
        self.title
            .set_subtitle(&path.parent().map(home_relative).unwrap_or_default());
        (self.remember)(path.clone());
        *self.path.borrow_mut() = Some(path);
    }

    fn set_status(&self, text: &str, error: bool) {
        self.status.set_text(text);
        self.status.set_tooltip_text(Some(text));
        if error {
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
        }
    }

    fn choose_open(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.script");
        filter.set_name(Some(&gettext("GTKTerm scripts")));
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open script"))
            .filters(&filters)
            .modal(true)
            .build();
        if let Some(dir) = self.path.borrow().as_ref().and_then(|p| p.parent()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let w = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let (Some(w), Some(path)) = (w.upgrade(), res.ok().and_then(|f| f.path())) else {
                return;
            };
            if let Err(e) = w.load(&path) {
                w.set_status(&e, true);
            }
        });
    }

    fn save(self: &Rc<Self>, choose: bool) {
        let current = self.path.borrow().clone();
        if let (false, Some(path)) = (choose, current) {
            let shown = path.display();
            match std::fs::write(&path, self.text()) {
                Ok(()) => {
                    self.mark_dirty(false);
                    self.set_status(&gettext_f("Saved {path}", &[("path", &shown)]), false);
                }
                Err(e) => self.set_status(
                    &gettext_f("Cannot save {path}: {error}", &[("path", &shown), ("error", &e)]),
                    true,
                ),
            }
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Save script"))
            .initial_name("untitled.script")
            .modal(true)
            .build();
        let w = Rc::downgrade(self);
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let (Some(w), Some(path)) = (w.upgrade(), res.ok().and_then(|f| f.path())) else {
                return;
            };
            w.set_path(path);
            w.save(false);
        });
    }

    fn mark_line(&self, tag: &gtk::TextTag, line: usize) {
        let b = &self.buffer;
        b.remove_tag(tag, &b.start_iter(), &b.end_iter());
        let Some(start) = b.iter_at_line(i32::try_from(line.saturating_sub(1)).unwrap_or(0)) else {
            return;
        };
        let mut end = start;
        end.forward_to_line_end();
        b.apply_tag(tag, &start, &end);
        let mut it = start;
        self.view.scroll_to_iter(&mut it, 0.1, false, 0.0, 0.0);
    }

    fn append_log(&self, line: &str) {
        let mut end = self.log.end_iter();
        self.log.insert(&mut end, line);
        self.log.insert(&mut end, "\n");
        let mut end = self.log.end_iter();
        self.log_view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
    }

    fn name(&self) -> String {
        self.path
            .borrow()
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "<editor>".to_owned(), |n| n.to_string_lossy().into_owned())
    }

    /// Parse and run the editor's text.
    pub fn start(self: &Rc<Self>) {
        if self.running.get() {
            return;
        }
        let name = self.name();
        let base = self
            .path
            .borrow()
            .as_ref()
            .and_then(|p| p.parent().map(ToOwned::to_owned));
        let script = match Script::parse(&name, &self.text(), base.as_deref()) {
            Ok(s) => s,
            Err(e) => {
                if let ScriptError::Parse { at, .. } = &e
                    && at.file == name
                {
                    self.mark_line(&self.error_tag, at.line);
                }
                self.set_status(&e.to_string(), true);
                return;
            }
        };
        let opts = ScriptOptions {
            allow_exec: (self.allow_exec)(),
            ..ScriptOptions::default()
        };
        self.log.set_text("");
        self.set_running(true);
        self.set_status(&gettext_f("Running {name}…", &[("name", &name)]), false);
        let (m, s) = (Arc::clone(&self.manager), self.session.clone());
        let w = Rc::downgrade(self);
        spawn(async move { m.run_script(&s, &script, &opts).await }, move |r| {
            let Some(w) = w.upgrade() else { return };
            w.set_running(false);
            let b = &w.buffer;
            b.remove_tag(&w.running_tag, &b.start_iter(), &b.end_iter());
            match r {
                Ok(report) => {
                    let seconds = format!("{:.1}", report.duration_ms as f64 / 1000.0);
                    w.set_status(
                        &gettext_f(
                            "Finished: {steps} steps in {seconds} s",
                            &[("steps", &report.steps), ("seconds", &seconds)],
                        ),
                        false,
                    );
                }
                Err(ScriptError::Failed { at, message, .. }) => {
                    if at.file == w.name() {
                        w.mark_line(&w.error_tag, at.line);
                    }
                    w.append_log(&format!("✗ {at}: {message}"));
                    w.set_status(
                        &gettext_f("Failed at {at}: {message}", &[("at", &at), ("message", &message)]),
                        true,
                    );
                }
                Err(e) => w.set_status(&e.to_string(), !matches!(e, ScriptError::Cancelled)),
            }
        });
    }

    fn set_running(&self, on: bool) {
        self.running.set(on);
        self.run.set_sensitive(!on);
        self.stop.set_sensitive(on);
        self.view.set_editable(!on);
    }

    /// Manager events for this window's session.
    pub fn on_event(&self, ev: &ManagerEvent) {
        match ev {
            ManagerEvent::ScriptLine { at, .. } if self.running.get() && at.file == self.name() => {
                self.mark_line(&self.running_tag, at.line);
            }
            // Also after the run ended (events and the result travel separately), and for
            // scripts an agent runs on this port.
            ManagerEvent::ScriptLog { message, .. } => self.append_log(message),
            _ => {}
        }
    }

    /// Log text (tests).
    #[cfg(test)]
    pub fn log_text(&self) -> String {
        self.log
            .text(&self.log.start_iter(), &self.log.end_iter(), false)
            .into()
    }

    /// Status line (tests).
    #[cfg(test)]
    pub fn status_text(&self) -> String {
        self.status.text().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_below_home_use_a_tilde() {
        let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            return;
        };
        assert_eq!(home_relative(&home.join("no-such-dir/x")), "~/no-such-dir/x");
        assert_eq!(home_relative(&home), "~");
        assert_eq!(home_relative(std::path::Path::new("/usr/share")), "/usr/share");
    }
}
