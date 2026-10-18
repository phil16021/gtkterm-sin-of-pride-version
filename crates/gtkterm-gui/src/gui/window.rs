//! The main window: header bar, terminal, chart panel, status bar and all actions.

use std::cell::{Cell, RefCell};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use gtkterm_core::config::{Connection, Profile, STANDARD_BAUD_RATES};
use gtkterm_core::decoding::{DecoderMode, DecoderStatus};
use gtkterm_core::manager::SessionManager;
use gtkterm_core::ports::{PortWatcher, list_ports, watch_ports};
use gtkterm_core::session::{LogOp, PortState, Session, SessionEvent};
use gtkterm_mcp::transport::{HttpHandle, HttpSecurity, random_token, serve_http};
use gtkterm_mcp::{Approval, ApprovalRequest, Approver, GtkTermMcp, ServerOptions};
use gtkterm_protocols::logfmt::{LogFormat, session_file_name};
use gtkterm_protocols::view::{HEX_WIDTHS, ViewMode};
use vte::prelude::TerminalExt as _;

use super::charts::{VizPanel, save_dialog};
use super::frames::FramesView;
use super::i18n::{gettext, gettext_f, ngettext_f};
use super::rt::{events, forward, rt, spawn, spawn_io};
use super::shortcuts::SHORTCUTS;
use super::state::{McpSettings, UiState};
use super::statusbar::StatusBar;
use super::terminal::TerminalView;
use super::{APP_ID, prefs};

/// Window controller.
pub struct Win {
    /// The window.
    pub window: adw::ApplicationWindow,
    toast: adw::ToastOverlay,
    banner: adw::Banner,
    term: Rc<TerminalView>,
    panel: Rc<VizPanel>,
    paned: gtk::Paned,
    frames: Rc<FramesView>,
    term_paned: gtk::Paned,
    status: StatusBar,
    port_drop: gtk::DropDown,
    port_paths: RefCell<Vec<String>>,
    baud_drop: gtk::DropDown,
    bauds: RefCell<Vec<u32>>,
    connect: gtk::ToggleButton,
    view_drop: gtk::DropDown,
    manager: Arc<SessionManager>,
    session: Session,
    profile_name: RefCell<String>,
    profile: RefCell<Profile>,
    state: RefCell<UiState>,
    mcp: RefCell<Option<HttpHandle>>,
    mcp_url: RefCell<Option<String>>,
    updating: Cell<bool>,
    watcher: RefCell<Option<PortWatcher>>,
    macro_bar: gtk::FlowBox,
    shortcut_ctl: RefCell<Option<gtk::ShortcutController>>,
    recorder: RefCell<Option<super::macros::Recorder>>,
    running_macro: RefCell<Option<String>>,
    macro_editor: RefCell<Option<Rc<super::macros::MacroEditor>>>,
    mcp_allow_dirs: RefCell<Vec<PathBuf>>,
    script_window: RefCell<Option<Rc<super::scripts::ScriptWindow>>>,
    signal_window: RefCell<Option<Rc<super::signals::SignalMonitor>>>,
    pending_script: RefCell<Option<PathBuf>>,
    cli_allow_exec: Cell<bool>,
}

/// Startup options passed from the command line.
pub struct StartOptions {
    /// Open the connection at startup.
    pub open_at_start: bool,
    /// `--mcp-http` address, overriding the saved setting.
    pub mcp_addr: Option<SocketAddr>,
    /// `--mcp-token`.
    pub mcp_token: Option<String>,
    /// `--mcp-read-only`.
    pub mcp_read_only: bool,
    /// `--mcp-allow-dir`.
    pub mcp_allow_dirs: Vec<PathBuf>,
    /// `--script`: run once the port is open.
    pub script: Option<PathBuf>,
    /// `--allow-exec`.
    pub allow_exec: bool,
    /// `--log-file` with its format.
    pub log: Option<(PathBuf, LogFormat)>,
    /// Message to show (e.g. legacy config import).
    pub notice: Option<String>,
}

fn icon_button(icon: &str, tooltip: &str, action: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .action_name(action)
        .build()
}

impl Win {
    /// Build and show the window.
    pub fn new(
        app: &adw::Application,
        manager: Arc<SessionManager>,
        session: Session,
        profile_name: String,
        profile: Profile,
        start: StartOptions,
    ) -> Rc<Self> {
        let state = UiState::load();
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("GTKTerm Sunshine")
            .default_width(state.width)
            .default_height(state.height)
            .maximized(state.maximized)
            .fullscreened(state.fullscreen)
            .icon_name(APP_ID)
            .build();

        // Header bar.
        let header = adw::HeaderBar::new();
        let port_drop = gtk::DropDown::builder().tooltip_text(gettext("Port")).build();
        port_drop.set_factory(Some(&ellipsizing_factory(28)));
        port_drop.set_list_factory(Some(&ellipsizing_factory(60)));
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(gettext("Rescan ports"))
            .build();
        let baud_drop = gtk::DropDown::builder()
            .tooltip_text(gettext("Baud rate (other rates: Preferences)"))
            .build();
        let connect = gtk::ToggleButton::builder()
            .label(gettext("Connect"))
            .tooltip_text(gettext("Connect / disconnect (F5)"))
            .build();
        connect.add_css_class("suggested-action");
        header.pack_start(&port_drop);
        header.pack_start(&refresh);
        header.pack_start(&baud_drop);
        header.pack_start(&connect);

        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu())
            .primary(true)
            .tooltip_text(gettext("Main menu"))
            .build();
        header.pack_end(&menu_button);
        let macro_button = gtk::MenuButton::builder()
            .icon_name("input-keyboard-symbolic")
            .tooltip_text(gettext("Macros"))
            .build();
        macro_button.set_create_popup_func(|b| b.set_menu_model(Some(&macro_menu())));
        header.pack_end(&macro_button);
        let viz_button = gtk::ToggleButton::builder()
            .icon_name("utilities-system-monitor-symbolic")
            .tooltip_text(gettext("Visualization panel (Ctrl+Shift+G)"))
            .action_name("win.visualization")
            .build();
        header.pack_end(&viz_button);
        let mode_labels: Vec<String> = ViewMode::ALL.iter().map(|m| gettext(m.label())).collect();
        let mode_refs: Vec<&str> = mode_labels.iter().map(String::as_str).collect();
        let view_drop = gtk::DropDown::builder()
            .model(&gtk::StringList::new(&mode_refs))
            .tooltip_text(gettext("View mode"))
            .build();
        header.pack_end(&view_drop);
        header.pack_end(&icon_button(
            "edit-find-symbolic",
            &gettext("Search (Ctrl+Shift+F)"),
            "win.search",
        ));

        // Content.
        let term = TerminalView::new(&session, &profile.terminal);
        let toast = adw::ToastOverlay::new();
        let panel = VizPanel::new(&session, &toast);
        let frames = FramesView::new(&session);
        let term_paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&term.widget)
            .end_child(&frames.widget)
            .wide_handle(true)
            .build();
        term_paned.set_resize_start_child(true);
        term_paned.set_resize_end_child(false);
        term_paned.set_shrink_start_child(false);
        term_paned.set_shrink_end_child(false);
        frames.widget.set_visible(state.frames_visible);
        if let Some(p) = state.frames_position {
            term_paned.set_position(p);
        }
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&term_paned)
            .end_child(&panel.widget)
            .wide_handle(true)
            .build();
        paned.set_resize_start_child(true);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        panel.widget.set_visible(state.panel_visible);
        if let Some(p) = state.pane_position {
            paned.set_position(p);
        }
        let banner = adw::Banner::builder()
            .button_label(gettext("Reconnect"))
            .revealed(false)
            .build();
        banner.set_action_name(Some("win.open-port"));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&banner);
        content.append(&paned);
        paned.set_vexpand(true);
        toast.set_child(Some(&content));

        let status = StatusBar::new();
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&toast));
        let macro_bar = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(12)
            .margin_start(6)
            .margin_end(6)
            .build();
        macro_bar.set_visible(state.macro_bar);
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom.append(&macro_bar);
        bottom.append(&status.widget);
        toolbar_view.add_bottom_bar(&bottom);
        window.set_content(Some(&toolbar_view));

        let win = Rc::new(Self {
            window,
            toast,
            banner,
            term,
            panel,
            paned,
            frames,
            term_paned,
            status,
            port_drop,
            port_paths: RefCell::default(),
            baud_drop,
            bauds: RefCell::default(),
            connect,
            view_drop,
            manager,
            session,
            profile_name: RefCell::new(profile_name),
            profile: RefCell::new(profile),
            state: RefCell::new(state),
            mcp: RefCell::default(),
            mcp_url: RefCell::default(),
            updating: Cell::new(false),
            watcher: RefCell::default(),
            macro_bar,
            shortcut_ctl: RefCell::default(),
            recorder: RefCell::default(),
            running_macro: RefCell::default(),
            macro_editor: RefCell::default(),
            mcp_allow_dirs: RefCell::new(start.mcp_allow_dirs.clone()),
            script_window: RefCell::default(),
            signal_window: RefCell::default(),
            pending_script: RefCell::new(start.script.clone()),
            cli_allow_exec: Cell::new(start.allow_exec),
        });
        win.install_actions(app);
        win.rebuild_macros();
        win.pump_manager_events();
        win.wire_header(&refresh);
        win.wire_status();
        win.refresh_header();
        win.pump_events();
        win.watch_hotplug();
        win.window.present();

        let w = Rc::downgrade(&win);
        win.window.connect_close_request(move |_| {
            if let Some(w) = w.upgrade() {
                w.shutdown();
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&win);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(w) = w.upgrade() {
                w.panel.redraw_all();
            }
        });

        if let Some(msg) = &start.notice {
            win.toast(msg);
        }
        if let Some((path, format)) = start.log.clone() {
            let label = win.profile.borrow().connection.summary();
            let w = Rc::downgrade(&win);
            let s = win.session.clone();
            spawn(
                async move {
                    s.log(LogOp::Start {
                        path,
                        format,
                        append: true,
                        port_label: Some(label),
                    })
                    .await
                },
                move |r| {
                    if let (Some(w), Err(e)) = (w.upgrade(), r) {
                        w.toast(&gettext_f("Cannot start logging: {error}", &[("error", &e)]));
                    }
                },
            );
        }
        let mcp_settings = {
            let mut st = win.state.borrow_mut();
            if let Some(addr) = start.mcp_addr {
                st.mcp.enabled = true;
                st.mcp.port = addr.port();
            }
            if start.mcp_read_only {
                st.mcp.read_only = true;
            }
            if let Some(t) = start.mcp_token {
                st.mcp.token = Some(t);
            }
            st.mcp.clone()
        };
        if mcp_settings.enabled {
            win.start_mcp(start.mcp_addr);
        }
        if start.open_at_start {
            win.open_port();
        }
        win
    }

    fn toast(&self, text: &str) {
        self.toast
            .add_toast(adw::Toast::builder().title(text).timeout(4).build());
    }

    // ----- header -----

    fn refresh_header(&self) {
        self.updating.set(true);
        let profile = self.profile.borrow();
        match &profile.connection {
            Connection::Serial(s) => {
                let mut ports: Vec<(String, String)> = list_ports()
                    .into_iter()
                    .map(|p| (p.path.clone(), p.label()))
                    .collect();
                if !ports.iter().any(|(p, _)| *p == s.port) {
                    // A symlink (e.g. socat's link=) or a device udev does not report.
                    let exists = std::path::Path::new(&s.port).exists();
                    let label = if exists {
                        s.port.clone()
                    } else {
                        gettext_f("{port} (not found)", &[("port", &s.port)])
                    };
                    ports.insert(0, (s.port.clone(), label));
                }
                let labels: Vec<&str> = ports.iter().map(|(_, l)| l.as_str()).collect();
                self.port_drop.set_model(Some(&gtk::StringList::new(&labels)));
                let idx = ports.iter().position(|(p, _)| *p == s.port).unwrap_or(0);
                self.port_drop.set_selected(idx as u32);
                *self.port_paths.borrow_mut() = ports.into_iter().map(|(p, _)| p).collect();
                let mut bauds: Vec<u32> = STANDARD_BAUD_RATES.to_vec();
                if !bauds.contains(&s.baudrate) {
                    bauds.push(s.baudrate);
                    bauds.sort_unstable();
                }
                let labels: Vec<String> = bauds.iter().map(ToString::to_string).collect();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                self.baud_drop.set_model(Some(&gtk::StringList::new(&refs)));
                self.baud_drop
                    .set_selected(bauds.iter().position(|b| *b == s.baudrate).unwrap_or(0) as u32);
                *self.bauds.borrow_mut() = bauds;
                self.baud_drop.set_visible(true);
                self.port_drop.set_sensitive(true);
            }
            other => {
                self.port_drop
                    .set_model(Some(&gtk::StringList::new(&[&other.summary()])));
                self.port_paths.borrow_mut().clear();
                self.port_drop.set_sensitive(false);
                self.baud_drop.set_visible(false);
            }
        }
        let mode = profile.terminal.view_mode;
        self.view_drop
            .set_selected(ViewMode::ALL.iter().position(|m| *m == mode).unwrap_or(0) as u32);
        self.updating.set(false);
    }

    fn wire_header(self: &Rc<Self>, refresh: &gtk::Button) {
        let w = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.refresh_header();
            }
        });
        let w = Rc::downgrade(self);
        self.port_drop.connect_selected_notify(move |d| {
            let Some(w) = w.upgrade() else { return };
            if w.updating.get() {
                return;
            }
            let Some(path) = w.port_paths.borrow().get(d.selected() as usize).cloned() else {
                return;
            };
            w.profile.borrow_mut().connection.serial_mut().port = path;
        });
        let w = Rc::downgrade(self);
        self.baud_drop.connect_selected_notify(move |d| {
            let Some(w) = w.upgrade() else { return };
            if w.updating.get() {
                return;
            }
            let Some(b) = w.bauds.borrow().get(d.selected() as usize).copied() else {
                return;
            };
            w.profile.borrow_mut().connection.serial_mut().baudrate = b;
            if w.is_open() {
                w.open_port();
            }
        });
        let w = Rc::downgrade(self);
        self.connect.connect_toggled(move |b| {
            let Some(w) = w.upgrade() else { return };
            if w.updating.get() {
                return;
            }
            if b.is_active() {
                w.open_port();
            } else {
                w.close_port();
            }
        });
        let w = Rc::downgrade(self);
        self.view_drop.connect_selected_notify(move |d| {
            let Some(w) = w.upgrade() else { return };
            if w.updating.get() {
                return;
            }
            let mode = ViewMode::ALL[(d.selected() as usize).min(ViewMode::ALL.len() - 1)];
            w.set_view_mode(mode);
        });
    }

    fn set_view_mode(&self, mode: ViewMode) {
        self.profile.borrow_mut().terminal.view_mode = mode;
        self.term.set_view_mode(mode);
        if let Some(a) = self.window.lookup_action("view-mode") {
            a.change_state(&format!("{mode:?}").to_lowercase().to_variant());
        }
        self.updating.set(true);
        self.view_drop
            .set_selected(ViewMode::ALL.iter().position(|m| *m == mode).unwrap_or(0) as u32);
        self.updating.set(false);
    }

    fn reflect_state(&self, state: &PortState) {
        self.updating.set(true);
        let open = matches!(state, PortState::Open | PortState::Opening);
        self.connect.set_active(open);
        self.connect.set_label(&if open {
            gettext("Disconnect")
        } else {
            gettext("Connect")
        });
        if open {
            self.connect.remove_css_class("suggested-action");
        } else {
            self.connect.add_css_class("suggested-action");
        }
        self.updating.set(false);
        match state {
            PortState::Error(e) => {
                self.banner
                    .set_title(&gettext_f("Connection error: {error}", &[("error", e)]));
                self.banner.set_revealed(true);
            }
            _ => self.banner.set_revealed(false),
        }
        let title = match state {
            PortState::Open => format!(
                "GTKTerm Sunshine — [{}] {}",
                self.profile_name.borrow(),
                self.session.status().description
            ),
            _ => format!("GTKTerm Sunshine — [{}]", self.profile_name.borrow()),
        };
        self.window.set_title(Some(&title));
    }

    fn is_open(&self) -> bool {
        matches!(self.session.status().state, PortState::Open)
    }

    fn open_port(self: &Rc<Self>) {
        let conn = self.profile.borrow().connection.clone();
        if let Err(e) = (Profile {
            connection: conn.clone(),
            terminal: self.term.config(),
            ..Profile::default()
        })
        .validate()
        {
            self.toast(&e.to_string());
            self.reflect_state(&self.session.status().state);
            return;
        }
        let s = self.session.clone();
        let w = Rc::downgrade(self);
        spawn(
            async move { s.open(conn).await.map_err(|e| e.to_string()) },
            move |r| {
                if let (Some(w), Err(e)) = (w.upgrade(), r) {
                    w.toast(&e);
                }
            },
        );
    }

    fn close_port(&self) {
        let s = self.session.clone();
        spawn_io(async move {
            let _ = s.close().await;
        });
    }

    /// SIGUSR1 opens and SIGUSR2 closes the port, as in the C GTKTerm.
    pub fn watch_unix_signals(&self) {
        use tokio::signal::unix::{SignalKind, signal};
        // Registered here, not in the task: a signal raised right after this call must not
        // hit the default action (terminate).
        let _guard = rt().enter();
        let (Ok(mut usr1), Ok(mut usr2)) = (
            signal(SignalKind::user_defined1()),
            signal(SignalKind::user_defined2()),
        ) else {
            tracing::warn!("cannot watch SIGUSR1/SIGUSR2");
            return;
        };
        let (tx, rx) = async_channel::unbounded();
        spawn_io(async move {
            loop {
                let action = tokio::select! {
                    _ = usr1.recv() => "win.open-port",
                    _ = usr2.recv() => "win.close-port",
                };
                if tx.send(action).await.is_err() {
                    break;
                }
            }
        });
        let w = self.window.downgrade();
        glib::spawn_future_local(async move {
            while let Ok(action) = rx.recv().await {
                let Some(w) = w.upgrade() else { break };
                let _ = WidgetExt::activate_action(&w, action, None);
            }
        });
    }

    // ----- events -----

    fn pump_events(self: &Rc<Self>) {
        let rx = events(&self.session);
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                let Some(w) = w.upgrade() else { break };
                w.handle(&ev);
                // Drain a burst before yielding to the main loop, so redraws stay batched.
                let mut n = 0;
                while let Ok(ev) = rx.try_recv() {
                    w.handle(&ev);
                    n += 1;
                    if n > 256 {
                        break;
                    }
                }
            }
        });
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(w) = w.upgrade() else {
                return glib::ControlFlow::Break;
            };
            w.status.update(&w.session.status());
            w.panel.update_footer();
            glib::ControlFlow::Continue
        });
    }

    fn handle(self: &Rc<Self>, ev: &SessionEvent) {
        match ev {
            SessionEvent::Data(chunk) => self.term.show(chunk),
            SessionEvent::State(state) => {
                self.reflect_state(state);
                self.status.update(&self.session.status());
                match state {
                    PortState::Open => {
                        let description = self.session.status().description;
                        self.term.notice(&gettext_f(
                            "connected: {description}",
                            &[("description", &description)],
                        ));
                        if let Some(path) = self.pending_script.borrow_mut().take() {
                            let sw = self.script_window();
                            match sw.load(&path) {
                                Ok(()) => {
                                    sw.present();
                                    sw.start();
                                }
                                Err(e) => self.toast(&e),
                            }
                        }
                    }
                    PortState::Closed => self.term.notice(&gettext("disconnected")),
                    PortState::Error(e) => self.term.notice(&gettext_f("error: {error}", &[("error", e)])),
                    PortState::Opening => {}
                }
            }
            SessionEvent::Info(msg) => self.term.notice(msg),
            SessionEvent::Modem(m) => self.status.set_modem(Some(*m)),
            SessionEvent::Sunshine(u) => {
                // Show the panel before the first tab is added, so the tab bar never lays out at zero width.
                let first = matches!(u, gtkterm_core::sunshine::SunshineUpdate::ChartAdded(_))
                    && self.panel.len() == 0;
                if first && !self.panel.widget.is_visible() {
                    self.set_panel_visible(true);
                }
                self.panel.on_update(u);
            }
            SessionEvent::Cleared => self.panel.sync(),
            SessionEvent::Frames(f) => self.frames.append(f),
            SessionEvent::Decoder(st) => self.on_decoder_status(st),
            SessionEvent::Transfer(st) => self.on_transfer(st),
            SessionEvent::Signals(_) => {
                if let Some(m) = self.signal_window.borrow().as_ref() {
                    m.update();
                }
            }
        }
    }

    fn watch_hotplug(self: &Rc<Self>) {
        let Some(mut watcher) = watch_ports() else { return };
        // Move the receiver out; the watcher itself must stay alive (dropping it stops the thread).
        let (_, placeholder) = tokio::sync::mpsc::channel(1);
        let rx = forward(std::mem::replace(&mut watcher.changes, placeholder));
        *self.watcher.borrow_mut() = Some(watcher);
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                let Some(w) = w.upgrade() else { break };
                w.refresh_header();
            }
        });
    }

    // ----- macros -----

    fn rebuild_macros(self: &Rc<Self>) {
        let macros: Vec<(String, Option<String>)> = self.manager.with_macros(|s| {
            s.list()
                .iter()
                .map(|m| (m.name.clone(), m.shortcut.clone()))
                .collect()
        });
        MACRO_NAMES.with(|n| n.borrow_mut().clone_from(&macros));

        // Shortcuts: capture phase, so function keys reach us before VTE turns them into escape codes.
        if let Some(old) = self.shortcut_ctl.borrow_mut().take() {
            self.window.remove_controller(&old);
        }
        let ctl = gtk::ShortcutController::new();
        ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
        for (name, shortcut) in &macros {
            let Some(accel) = shortcut else { continue };
            let Ok(accel) = super::macros::validate_shortcut(accel) else {
                continue;
            };
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(&accel) else {
                continue;
            };
            let w = Rc::downgrade(self);
            let name = name.clone();
            let action = gtk::CallbackAction::new(move |_, _| {
                if let Some(w) = w.upgrade() {
                    w.play_macro(&name);
                }
                glib::Propagation::Stop
            });
            ctl.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        }
        self.window.add_controller(ctl.clone());
        *self.shortcut_ctl.borrow_mut() = Some(ctl);

        // Macro bar.
        while let Some(child) = self.macro_bar.first_child() {
            self.macro_bar.remove(&child);
        }
        for (name, shortcut) in &macros {
            let label = match shortcut {
                Some(s) => format!("{name}  ·  {}", super::macros::shortcut_label(s)),
                None => name.clone(),
            };
            let b = gtk::Button::builder()
                .label(&label)
                .action_name("win.play-macro")
                .action_target(&name.to_variant())
                .build();
            b.add_css_class("flat");
            self.macro_bar.append(&b);
        }
        let edit = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(gettext("Edit macros"))
            .action_name("win.edit-macros")
            .build();
        edit.add_css_class("flat");
        self.macro_bar.append(&edit);
    }

    fn play_macro(self: &Rc<Self>, name: &str) {
        let (mgr, session, name) = (Arc::clone(&self.manager), self.session.clone(), name.to_owned());
        let w = Rc::downgrade(self);
        spawn(
            async move { mgr.run_macro(&session, &name).await.map(|r| (name, r)) },
            move |r| {
                let Some(w) = w.upgrade() else { return };
                match r {
                    Ok((name, report)) => w.toast(&gettext_f(
                        "Macro {name} finished ({steps} steps, {ms} ms)",
                        &[
                            ("name", &name),
                            ("steps", &report.steps.len()),
                            ("ms", &report.duration_ms),
                        ],
                    )),
                    Err(gtkterm_core::macros::MacroError::Cancelled) => w.toast(&gettext("Macro stopped")),
                    Err(e) => w.toast(&gettext_f("Macro: {error}", &[("error", &e)])),
                }
            },
        );
    }

    fn set_recording(self: &Rc<Self>, on: bool) {
        if on {
            *self.recorder.borrow_mut() = Some(super::macros::Recorder::default());
            let w = Rc::downgrade(self);
            *self.term.on_input.borrow_mut() = Some(Box::new(move |data| {
                if let Some(w) = w.upgrade()
                    && let Some(r) = w.recorder.borrow_mut().as_mut()
                {
                    r.push(data);
                }
            }));
        } else {
            self.term.on_input.borrow_mut().take();
            if let Some(rec) = self.recorder.borrow_mut().take() {
                let commands = rec.finish();
                if commands.is_empty() {
                    self.toast(&gettext("Nothing was recorded"));
                } else {
                    let name = self.manager.with_macros(|s| s.fresh_name(&gettext("Recorded")));
                    let m = gtkterm_core::macros::Macro {
                        name: name.clone(),
                        shortcut: None,
                        commands,
                    };
                    match self.manager.update_macros(|s| s.upsert(None, m)) {
                        Ok(()) => self.open_macro_editor(Some(name)),
                        Err(e) => self.toast(&e.to_string()),
                    }
                }
            }
        }
        if let Some(a) = self.window.lookup_action("record-macro") {
            a.change_state(&on.to_variant());
        }
        self.status
            .set_activity(on, self.running_macro.borrow().as_deref());
    }

    fn record_sent(&self, data: &[u8]) {
        if let Some(r) = self.recorder.borrow_mut().as_mut() {
            r.push(data);
        }
    }

    fn open_macro_editor(self: &Rc<Self>, select: Option<String>) {
        let w = Rc::downgrade(self);
        let w2 = Rc::downgrade(self);
        let editor = super::macros::MacroEditor::present(
            &self.window,
            Arc::clone(&self.manager),
            select,
            move |name| {
                if let Some(w) = w.upgrade() {
                    w.play_macro(name);
                }
            },
            move || {
                if let Some(w) = w2.upgrade() {
                    w.manager.stop_macro(w.session.id());
                }
            },
        );
        *self.macro_editor.borrow_mut() = Some(editor);
    }

    fn pump_manager_events(self: &Rc<Self>) {
        let rx = super::rt::forward_broadcast(self.manager.subscribe());
        let w = Rc::downgrade(self);
        let me = self.session.id();
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                let Some(w) = w.upgrade() else { break };
                match ev {
                    gtkterm_core::manager::ManagerEvent::MacrosChanged => w.rebuild_macros(),
                    gtkterm_core::manager::ManagerEvent::MacroStarted { session, name } if session == me => {
                        *w.running_macro.borrow_mut() = Some(name);
                        w.status
                            .set_activity(w.recorder.borrow().is_some(), w.running_macro.borrow().as_deref());
                    }
                    gtkterm_core::manager::ManagerEvent::MacroFinished { session, .. }
                    | gtkterm_core::manager::ManagerEvent::ScriptFinished { session, .. }
                        if session == me =>
                    {
                        w.running_macro.borrow_mut().take();
                        w.status.set_activity(w.recorder.borrow().is_some(), None);
                    }
                    gtkterm_core::manager::ManagerEvent::ScriptStarted { session, ref name }
                        if session == me =>
                    {
                        *w.running_macro.borrow_mut() = Some(gettext_f("script {name}", &[("name", name)]));
                        w.status
                            .set_activity(w.recorder.borrow().is_some(), w.running_macro.borrow().as_deref());
                    }
                    gtkterm_core::manager::ManagerEvent::ScriptLine { session, .. }
                    | gtkterm_core::manager::ManagerEvent::ScriptLog { session, .. }
                        if session == me =>
                    {
                        if let Some(sw) = w.script_window.borrow().as_ref() {
                            sw.on_event(&ev);
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    fn import_presets(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Import presets (GTKTerm JSON)"))
            .modal(true)
            .build();
        let w = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let (Some(w), Ok(file)) = (w.upgrade(), res) else {
                return;
            };
            let Some(path) = file.path() else { return };
            let result = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| gtkterm_core::presets::import_c_presets(&t).map_err(|e| e.to_string()));
            match result {
                Ok(list) => {
                    let n = list.len();
                    w.manager.with_config_mut(|c| c.presets.extend(list));
                    match w.manager.save_config() {
                        Ok(()) => w.toast(&ngettext_f(
                            "Imported {n} preset",
                            "Imported {n} presets",
                            u32::try_from(n).unwrap_or(u32::MAX),
                            &[],
                        )),
                        Err(e) => w.toast(&e.to_string()),
                    }
                }
                Err(e) => w.toast(&gettext_f("Cannot import presets: {error}", &[("error", &e)])),
            }
        });
    }

    fn set_panel_visible(&self, visible: bool) {
        self.panel.widget.set_visible(visible);
        if visible {
            self.panel.sync();
            if self.paned.position() < 100 || self.paned.position() > self.paned.width() - 200 {
                self.paned
                    .set_position((f64::from(self.paned.width()) * 0.55) as i32);
            }
        }
        self.state.borrow_mut().panel_visible = visible;
        if let Some(a) = self.window.lookup_action("visualization") {
            a.change_state(&visible.to_variant());
        }
    }

    fn set_frames_visible(&self, visible: bool) {
        self.frames.widget.set_visible(visible);
        if visible {
            let h = self.term_paned.height();
            if h > 0 && (self.term_paned.position() < 80 || self.term_paned.position() > h - 120) {
                self.term_paned.set_position((f64::from(h) * 0.6) as i32);
            }
        }
        self.state.borrow_mut().frames_visible = visible;
        if let Some(a) = self.window.lookup_action("frames") {
            a.change_state(&visible.to_variant());
        }
    }

    fn on_decoder_status(&self, st: &DecoderStatus) {
        self.frames.set_status(st);
        if let Some(a) = self.window.lookup_action("decoder") {
            a.change_state(&st.mode.to_string().to_variant());
        }
        if let Some(a) = self.window.lookup_action("decoder-signals") {
            a.change_state(&st.signals.to_variant());
        }
    }

    // ----- status bar -----

    fn wire_status(self: &Rc<Self>) {
        for (button, dtr) in [(&self.status.dtr, true), (&self.status.rts, false)] {
            let w = Rc::downgrade(self);
            button.connect_clicked(move |b| {
                let Some(w) = w.upgrade() else { return };
                let on = b.is_active();
                let s = w.session.clone();
                let ww = Rc::downgrade(&w);
                spawn(
                    async move {
                        if dtr {
                            s.set_dtr(on).await
                        } else {
                            s.set_rts(on).await
                        }
                        .map_err(|e| e.to_string())
                    },
                    move |r| {
                        if let (Some(w), Err(e)) = (ww.upgrade(), r) {
                            w.toast(&e);
                        }
                    },
                );
            });
        }
        let w = Rc::downgrade(self);
        self.status.profile.set_create_popup_func(move |button| {
            let Some(w) = w.upgrade() else { return };
            let menu = gio::Menu::new();
            let load = gio::Menu::new();
            for name in w
                .manager
                .with_config(|c| c.profiles.keys().cloned().collect::<Vec<_>>())
            {
                let current = *w.profile_name.borrow() == name;
                let label = if current {
                    gettext_f("{name}  (current)", &[("name", &name)])
                } else {
                    name.clone()
                };
                let item = gio::MenuItem::new(Some(&label), None);
                item.set_action_and_target_value(Some("win.load-profile"), Some(&name.to_variant()));
                load.append_item(&item);
            }
            menu.append_section(Some(&gettext("Load profile")), &load);
            let ops = gio::Menu::new();
            ops.append(Some(&gettext("Save current settings")), Some("win.save-profile"));
            ops.append(
                Some(&gettext("Save as new profile…")),
                Some("win.save-profile-as"),
            );
            ops.append(Some(&gettext("Delete this profile…")), Some("win.delete-profile"));
            menu.append_section(None, &ops);
            button.set_menu_model(Some(&menu));
        });
    }

    // ----- actions -----

    fn install_actions(self: &Rc<Self>, app: &adw::Application) {
        let add = |name: &str, f: Box<dyn Fn(&Rc<Self>)>| {
            let action = gio::SimpleAction::new(name, None);
            let w = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = w.upgrade() {
                    f(&w);
                }
            });
            self.window.add_action(&action);
        };
        add("copy", Box::new(|w| w.term.copy()));
        add("paste", Box::new(|w| w.term.paste()));
        add("select-all", Box::new(|w| w.term.select_all()));
        add("search", Box::new(|w| w.term.start_search()));
        add("clear-screen", Box::new(|w| w.term.clear(false)));
        add("clear-scrollback", Box::new(|w| w.term.clear(true)));
        add("open-port", Box::new(Self::open_port));
        add("close-port", Box::new(|w| w.close_port()));
        add(
            "toggle-connection",
            Box::new(|w| {
                if w.is_open() {
                    w.close_port();
                } else {
                    w.open_port();
                }
            }),
        );
        add("preferences", Box::new(Self::show_preferences));
        add(
            "send-break",
            Box::new(|w| {
                let s = w.session.clone();
                let ww = Rc::downgrade(w);
                spawn(
                    async move { s.send_default_break().await.map_err(|e| e.to_string()) },
                    move |r| {
                        if let Some(w) = ww.upgrade() {
                            w.toast(&r.map_or_else(|e| e, |()| gettext("Break sent")));
                        }
                    },
                );
            }),
        );
        add("dtr", Box::new(|w| w.status.dtr.emit_clicked()));
        add("rts", Box::new(|w| w.status.rts.emit_clicked()));
        add("send-hex", Box::new(Self::send_hex));
        add("send-file", Box::new(Self::send_file));
        add("receive-file", Box::new(Self::receive_file));
        add("scripts", Box::new(|w| w.script_window().present()));
        add("signal-monitor", Box::new(|w| w.signal_window().present()));
        add(
            "flush",
            Box::new(|w| {
                let s = w.session.clone();
                let ww = Rc::downgrade(w);
                spawn(
                    async move { s.flush().await.map_err(|e| e.to_string()) },
                    move |r| {
                        if let Some(w) = ww.upgrade() {
                            w.toast(&r.map_or_else(|e| e, |()| gettext("Buffers flushed")));
                        }
                    },
                );
            }),
        );
        add("auto-baud", Box::new(Self::auto_baud));
        add(
            "cancel-transfer",
            Box::new(|w| {
                let s = w.session.clone();
                spawn(async move { s.cancel_transfer().await }, |_| {});
            }),
        );
        add("save-raw", Box::new(Self::save_raw));
        add("log-start", Box::new(Self::log_start));
        add(
            "log-pause",
            Box::new(|w| {
                let paused = matches!(
                    w.session.status().log.map(|l| l.state),
                    Some(gtkterm_core::logging::LogState::Paused)
                );
                w.log_op(LogOp::Pause(!paused));
            }),
        );
        add("log-clear", Box::new(|w| w.log_op(LogOp::Clear)));
        add("log-stop", Box::new(|w| w.log_op(LogOp::Stop)));
        add(
            "clear-charts",
            Box::new(|w| {
                w.session
                    .with_sunshine_mut(gtkterm_core::sunshine::SunshineStore::clear);
                w.panel.sync();
            }),
        );
        add(
            "shortcuts",
            Box::new(|w| shortcuts_dialog().present(Some(&w.window))),
        );
        add("about", Box::new(|w| about_dialog().present(Some(&w.window))));
        // Terminal zoom, like GNOME Terminal (VTE scales the configured font).
        for (name, step) in [("zoom-in", 1.1), ("zoom-out", 1.0 / 1.1), ("zoom-reset", 0.0)] {
            add(
                name,
                Box::new(move |w| {
                    let t = &w.term.term;
                    let scale = if step == 0.0 { 1.0 } else { t.font_scale() * step };
                    t.set_font_scale(scale.clamp(0.25, 4.0));
                }),
            );
        }
        self.term
            .term
            .set_font_scale(self.state.borrow().font_scale.clamp(0.25, 4.0));
        let action =
            gio::SimpleAction::new_stateful("fullscreen", None, &self.window.is_fullscreen().to_variant());
        let w = self.window.downgrade();
        action.connect_activate(move |_, _| {
            if let Some(w) = w.upgrade() {
                w.set_fullscreened(!w.is_fullscreen());
            }
        });
        let a = action.downgrade();
        self.window.connect_fullscreened_notify(move |w| {
            if let Some(a) = a.upgrade() {
                a.set_state(&w.is_fullscreen().to_variant());
            }
        });
        self.window.add_action(&action);
        add(
            "save-profile",
            Box::new(|w| {
                let name = w.profile_name.borrow().clone();
                w.store_profile(&name);
            }),
        );
        add("save-profile-as", Box::new(Self::save_profile_as));
        add("delete-profile", Box::new(Self::delete_profile));

        // Stateful toggles of terminal options.
        let toggles: [(&str, fn(&mut gtkterm_core::config::TerminalConfig) -> &mut bool); 5] = [
            ("echo", |t| &mut t.echo),
            ("auto-cr", |t| &mut t.auto_cr),
            ("auto-lf", |t| &mut t.auto_lf),
            ("timestamps", |t| &mut t.timestamps),
            ("show-index", |t| &mut t.show_index),
        ];
        for (name, field) in toggles {
            let initial = *field(&mut self.profile.borrow_mut().terminal);
            let action = gio::SimpleAction::new_stateful(name, None, &initial.to_variant());
            let w = Rc::downgrade(self);
            action.connect_activate(move |a, _| {
                let Some(w) = w.upgrade() else { return };
                let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
                a.set_state(&on.to_variant());
                let t = {
                    let mut p = w.profile.borrow_mut();
                    *field(&mut p.terminal) = on;
                    p.terminal.clone()
                };
                w.term.apply_config(&t);
                w.session.set_terminal(t);
            });
            self.window.add_action(&action);
        }

        let mode = format!("{:?}", self.profile.borrow().terminal.view_mode).to_lowercase();
        let action =
            gio::SimpleAction::new_stateful("view-mode", Some(glib::VariantTy::STRING), &mode.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, param| {
            let (Some(w), Some(p)) = (w.upgrade(), param.and_then(glib::Variant::get::<String>)) else {
                return;
            };
            if let Some(mode) = ViewMode::ALL
                .into_iter()
                .find(|m| format!("{m:?}").to_lowercase() == p)
            {
                a.set_state(&p.to_variant());
                w.set_view_mode(mode);
            }
        });
        self.window.add_action(&action);

        let width = i32::from(self.profile.borrow().terminal.hex_chars);
        let action =
            gio::SimpleAction::new_stateful("hex-width", Some(glib::VariantTy::INT32), &width.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, param| {
            let (Some(w), Some(n)) = (w.upgrade(), param.and_then(glib::Variant::get::<i32>)) else {
                return;
            };
            a.set_state(&n.to_variant());
            let t = {
                let mut p = w.profile.borrow_mut();
                p.terminal.hex_chars = u8::try_from(n).unwrap_or(16);
                p.terminal.clone()
            };
            w.term.apply_config(&t);
            w.session.set_terminal(t);
        });
        self.window.add_action(&action);

        let visible = self.state.borrow().panel_visible;
        let action = gio::SimpleAction::new_stateful("visualization", None, &visible.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, _| {
            let Some(w) = w.upgrade() else { return };
            let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            w.set_panel_visible(on);
        });
        self.window.add_action(&action);

        let visible = self.state.borrow().frames_visible;
        let action = gio::SimpleAction::new_stateful("frames", None, &visible.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, _| {
            let Some(w) = w.upgrade() else { return };
            let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            w.set_frames_visible(on);
        });
        self.window.add_action(&action);

        let st = self.session.decoder_status();
        let action = gio::SimpleAction::new_stateful(
            "decoder",
            Some(glib::VariantTy::STRING),
            &st.mode.to_string().to_variant(),
        );
        let w = Rc::downgrade(self);
        action.connect_activate(move |_, param| {
            let (Some(w), Some(p)) = (w.upgrade(), param.and_then(glib::Variant::get::<String>)) else {
                return;
            };
            match p.parse::<DecoderMode>() {
                Ok(mode) => {
                    let signals = w.session.decoder_status().signals;
                    w.session.set_decoder(mode, signals);
                    if !mode.is_off() {
                        w.set_frames_visible(true);
                    }
                }
                Err(e) => w.toast(&e),
            }
        });
        self.window.add_action(&action);
        let action = gio::SimpleAction::new_stateful("decoder-signals", None, &st.signals.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, _| {
            let Some(w) = w.upgrade() else { return };
            // The session is the source of truth: the action state follows its events later.
            let st = w.session.decoder_status();
            a.set_state(&(!st.signals).to_variant());
            w.session.set_decoder(st.mode, !st.signals);
        });
        self.window.add_action(&action);
        add("clear-frames", Box::new(|w| w.session.clear_frames()));

        let enabled = self.state.borrow().mcp.enabled;
        let action = gio::SimpleAction::new_stateful("mcp", None, &enabled.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, _| {
            let Some(w) = w.upgrade() else { return };
            let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            a.set_state(&on.to_variant());
            w.state.borrow_mut().mcp.enabled = on;
            if on { w.start_mcp(None) } else { w.stop_mcp() }
        });
        self.window.add_action(&action);

        add("edit-macros", Box::new(|w| w.open_macro_editor(None)));
        add(
            "stop-macro",
            Box::new(|w| {
                if !w.manager.stop_macro(w.session.id()) {
                    w.toast(&gettext("No macro is running"));
                }
            }),
        );
        add("import-presets", Box::new(Self::import_presets));
        let action = gio::SimpleAction::new("play-macro", Some(glib::VariantTy::STRING));
        let w = Rc::downgrade(self);
        action.connect_activate(move |_, param| {
            if let (Some(w), Some(name)) = (w.upgrade(), param.and_then(glib::Variant::get::<String>)) {
                w.play_macro(&name);
            }
        });
        self.window.add_action(&action);
        let action = gio::SimpleAction::new_stateful("record-macro", None, &false.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, _| {
            let Some(w) = w.upgrade() else { return };
            let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            w.set_recording(on);
        });
        self.window.add_action(&action);
        let visible = self.state.borrow().macro_bar;
        let action = gio::SimpleAction::new_stateful("macro-bar", None, &visible.to_variant());
        let w = Rc::downgrade(self);
        action.connect_activate(move |a, _| {
            let Some(w) = w.upgrade() else { return };
            let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            a.set_state(&on.to_variant());
            w.macro_bar.set_visible(on);
            w.state.borrow_mut().macro_bar = on;
        });
        self.window.add_action(&action);

        let action = gio::SimpleAction::new("load-profile", Some(glib::VariantTy::STRING));
        let w = Rc::downgrade(self);
        action.connect_activate(move |_, param| {
            if let (Some(w), Some(name)) = (w.upgrade(), param.and_then(glib::Variant::get::<String>)) {
                w.load_profile(&name);
            }
        });
        self.window.add_action(&action);

        for (_, _, action, accel) in SHORTCUTS {
            app.set_accels_for_action(action, &[accel]);
        }
    }

    fn sync_toggle_states(&self) {
        let t = self.profile.borrow().terminal.clone();
        for (name, v) in [
            ("echo", t.echo),
            ("auto-cr", t.auto_cr),
            ("auto-lf", t.auto_lf),
            ("timestamps", t.timestamps),
            ("show-index", t.show_index),
        ] {
            if let Some(a) = self.window.lookup_action(name) {
                a.change_state(&v.to_variant());
            }
        }
        if let Some(a) = self.window.lookup_action("hex-width") {
            a.change_state(&i32::from(t.hex_chars).to_variant());
        }
    }

    fn log_op(self: &Rc<Self>, op: LogOp) {
        let s = self.session.clone();
        let w = Rc::downgrade(self);
        spawn(
            async move { s.log(op).await.map_err(|e| e.to_string()) },
            move |r| {
                let Some(w) = w.upgrade() else { return };
                match r {
                    Ok(Some(st)) if w.session.status().log.is_none() => {
                        w.toast(&gettext_f(
                            "Log saved: {path} ({bytes} bytes)",
                            &[("path", &st.path.display()), ("bytes", &st.bytes)],
                        ));
                    }
                    Ok(_) => {}
                    Err(e) => w.toast(&e),
                }
                w.status.update(&w.session.status());
            },
        );
    }

    fn log_start(self: &Rc<Self>) {
        let st = self.session.status();
        let stamp = jiff::Zoned::now().strftime("%Y%m%d_%H%M%S").to_string();
        let name = session_file_name(&stamp, &st.description, &st.name, LogFormat::Text);
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        for f in LogFormat::ALL {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&format!("{f:?} (*.{})", f.extension())));
            filter.add_suffix(f.extension());
            filters.append(&filter);
        }
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Log to file — format follows the extension"))
            .initial_name(name)
            .filters(&filters)
            .modal(true)
            .build();
        let w = Rc::downgrade(self);
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let (Some(w), Ok(file)) = (w.upgrade(), res) else {
                return;
            };
            let Some(path) = file.path() else { return };
            let format = path
                .extension()
                .and_then(|e| LogFormat::parse(&e.to_string_lossy()))
                .unwrap_or(LogFormat::Raw);
            w.log_op(LogOp::Start {
                path,
                format,
                append: false,
                port_label: None,
            });
        });
    }

    fn send_hex(self: &Rc<Self>) {
        let entry = gtk::Entry::builder()
            .placeholder_text(gettext("DE AD BE EF, 0x01 0x02 or \\x41\\x42"))
            .activates_default(true)
            .build();
        let history = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(3)
            .build();
        for h in self.state.borrow().hex_history.iter().take(9) {
            let b = gtk::Button::with_label(h);
            b.add_css_class("flat");
            let e = entry.clone();
            let text = h.clone();
            b.connect_clicked(move |_| e.set_text(&text));
            history.insert(&b, -1);
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&entry);
        body.append(&history);
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Send hex data"))
            .extra_child(&body)
            .default_response("send")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", gettext("Cancel").as_str()),
            ("send", gettext("Send").as_str()),
        ]);
        dialog.set_response_appearance("send", adw::ResponseAppearance::Suggested);
        let w = Rc::downgrade(self);
        let input = entry.clone();
        dialog.connect_response(None, move |_, resp| {
            let Some(w) = w.upgrade() else { return };
            if resp != "send" {
                return;
            }
            let text = input.text().to_string();
            match gtkterm_protocols::hex::parse_hex(&text) {
                Ok(bytes) if !bytes.is_empty() => {
                    w.state.borrow_mut().remember_hex(text.trim());
                    w.record_sent(&bytes);
                    let n = bytes.len();
                    let s = w.session.clone();
                    let ww = Rc::downgrade(&w);
                    spawn(
                        async move { s.send(bytes).await.map_err(|e| e.to_string()) },
                        move |r| {
                            if let Some(w) = ww.upgrade() {
                                w.toast(
                                    &r.map_or_else(|e| e, |()| gettext_f("Sent {n} bytes", &[("n", &n)])),
                                );
                            }
                        },
                    );
                }
                Ok(_) => {}
                Err(e) => w.toast(&gettext_f("Invalid hex: {error}", &[("error", &e)])),
            }
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    fn send_file(self: &Rc<Self>) {
        let last = self.state.borrow().transfer_protocol.clone();
        let w = Rc::downgrade(self);
        super::transfer::choose(&self.window, true, last.as_deref(), move |choice| {
            if let Some(w) = w.upgrade() {
                w.pick_files(choice);
            }
        });
    }

    fn pick_files(self: &Rc<Self>, choice: super::transfer::Choice) {
        use super::transfer::Choice;
        let multiple = matches!(choice, Choice::Protocol(p, _) if p.batch());
        let dialog = gtk::FileDialog::builder()
            .title(if multiple {
                gettext("Send files")
            } else {
                gettext("Send file")
            })
            .modal(true)
            .build();
        let w = Rc::downgrade(self);
        let done = move |paths: Vec<PathBuf>| {
            let Some(w) = w.upgrade() else { return };
            if paths.is_empty() {
                return;
            }
            match choice.clone() {
                Choice::Raw => w.send_raw(paths[0].clone()),
                Choice::Protocol(protocol, _) => {
                    w.state.borrow_mut().transfer_protocol = Some(protocol.id().to_owned());
                    w.run_transfer(gtkterm_core::transfer::TransferRequest::Send {
                        protocol,
                        files: paths,
                    });
                }
            }
        };
        if multiple {
            dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |res| {
                let Ok(list) = res else { return };
                let paths = (0..list.n_items())
                    .filter_map(|i| list.item(i).and_downcast::<gio::File>())
                    .filter_map(|f| f.path())
                    .collect();
                done(paths);
            });
        } else {
            dialog.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
                if let Some(path) = res.ok().and_then(|f| f.path()) {
                    done(vec![path]);
                }
            });
        }
    }

    fn send_raw(self: &Rc<Self>, path: PathBuf) {
        let s = self.session.clone();
        let ww = Rc::downgrade(self);
        spawn(
            async move {
                let data = tokio::fs::read(&path)
                    .await
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let n = data.len();
                s.send(data).await.map_err(|e| e.to_string())?;
                Ok::<_, String>(gettext_f(
                    "Sent {path} ({n} bytes)",
                    &[("path", &path.display()), ("n", &n)],
                ))
            },
            move |r| {
                if let Some(w) = ww.upgrade() {
                    w.toast(&r.unwrap_or_else(|e| e));
                }
            },
        );
    }

    fn receive_file(self: &Rc<Self>) {
        let last = self.state.borrow().transfer_protocol.clone();
        let w = Rc::downgrade(self);
        super::transfer::choose(&self.window, false, last.as_deref(), move |choice| {
            let (Some(w), super::transfer::Choice::Protocol(protocol, name)) = (w.upgrade(), choice) else {
                return;
            };
            let dialog = gtk::FileDialog::builder()
                .title(gettext("Save received files in"))
                .modal(true)
                .build();
            let initial = w
                .state
                .borrow()
                .transfer_dir
                .clone()
                .or_else(|| glib::user_special_dir(glib::UserDirectory::Downloads));
            if let Some(dir) = initial {
                dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
            }
            let ww = Rc::downgrade(&w);
            dialog.select_folder(Some(&w.window), gio::Cancellable::NONE, move |res| {
                let (Some(w), Some(dir)) = (ww.upgrade(), res.ok().and_then(|f| f.path())) else {
                    return;
                };
                {
                    let mut st = w.state.borrow_mut();
                    st.transfer_protocol = Some(protocol.id().to_owned());
                    st.transfer_dir = Some(dir.clone());
                }
                w.run_transfer(gtkterm_core::transfer::TransferRequest::Receive {
                    protocol,
                    dir,
                    name,
                    overwrite: false,
                });
            });
        });
    }

    fn script_window(self: &Rc<Self>) -> Rc<super::scripts::ScriptWindow> {
        if let Some(w) = self.script_window.borrow().as_ref() {
            return Rc::clone(w);
        }
        let w1 = Rc::downgrade(self);
        let w2 = Rc::downgrade(self);
        let sw = super::scripts::ScriptWindow::new(
            &self.window,
            Arc::clone(&self.manager),
            self.session.clone(),
            move || {
                w1.upgrade()
                    .is_some_and(|w| w.cli_allow_exec.get() || w.state.borrow().script_allow_exec)
            },
            move |p| {
                if let Some(w) = w2.upgrade() {
                    w.state.borrow_mut().script_path = Some(p);
                }
            },
        );
        if let Some(p) = self.state.borrow().script_path.clone()
            && p.is_file()
        {
            let _ = sw.load(&p);
        }
        *self.script_window.borrow_mut() = Some(Rc::clone(&sw));
        sw
    }

    fn signal_window(self: &Rc<Self>) -> Rc<super::signals::SignalMonitor> {
        if let Some(w) = self.signal_window.borrow().as_ref() {
            return Rc::clone(w);
        }
        let m = super::signals::SignalMonitor::new(&self.window, self.session.clone());
        *self.signal_window.borrow_mut() = Some(Rc::clone(&m));
        m
    }

    fn auto_baud(self: &Rc<Self>) {
        if !matches!(self.session.connection(), Some(Connection::Serial(_))) || !self.is_open() {
            self.toast(&gettext(
                "Open a serial port first; the device must be sending text",
            ));
            return;
        }
        self.toast(&gettext("Detecting the baud rate…"));
        let s = self.session.clone();
        let ww = Rc::downgrade(self);
        spawn(
            async move {
                gtkterm_core::autobaud::auto_baud(
                    &s,
                    &gtkterm_core::autobaud::CANDIDATES,
                    Duration::from_millis(400),
                    &tokio_util::sync::CancellationToken::new(),
                )
                .await
            },
            move |r| {
                let Some(w) = ww.upgrade() else { return };
                match r {
                    Ok(r) => match r.best {
                        Some(b) => {
                            if let Connection::Serial(cfg) = &mut w.profile.borrow_mut().connection {
                                cfg.baudrate = b;
                            }
                            w.refresh_header();
                            w.status.update(&w.session.status());
                            w.toast(&gettext_f("Detected {baud} baud", &[("baud", &b)]));
                        }
                        None => w.toast(&gettext("No rate gave readable text; the original rate is kept")),
                    },
                    Err(e) => w.toast(&gettext_f("Auto-baud failed: {error}", &[("error", &e)])),
                }
            },
        );
    }

    fn run_transfer(self: &Rc<Self>, req: gtkterm_core::transfer::TransferRequest) {
        let s = self.session.clone();
        let ww = Rc::downgrade(self);
        spawn(async move { s.start_transfer(req).await }, move |r| {
            if let (Some(w), Err(e)) = (ww.upgrade(), r) {
                w.toast(&gettext_f("Cannot start the transfer: {error}", &[("error", &e)]));
            }
        });
    }

    fn on_transfer(&self, st: &gtkterm_core::transfer::TransferStatus) {
        use gtkterm_core::transfer::{TransferDirection, TransferState};
        self.status.set_transfer(Some(st));
        if st.state == TransferState::Running {
            return;
        }
        let toast = adw::Toast::builder().title(st.summary()).timeout(6).build();
        if st.direction == TransferDirection::Receive
            && st.state == TransferState::Finished
            && let Some(f) = st.files.first()
        {
            toast.set_button_label(Some(&gettext("Show")));
            let (file, window) = (gio::File::for_path(&f.path), self.window.downgrade());
            toast.connect_button_clicked(move |_| {
                let window = window.upgrade();
                gtk::FileLauncher::new(Some(&file)).open_containing_folder(
                    window.as_ref(),
                    gio::Cancellable::NONE,
                    |_| {},
                );
            });
        }
        self.toast.add_toast(toast);
    }

    fn save_raw(self: &Rc<Self>) {
        let data = self.session.read_rx(0, usize::MAX).data;
        if data.is_empty() {
            self.toast(&gettext("Nothing received yet"));
            return;
        }
        save_dialog(&self.window, "received.bin", data, self.toast.clone());
    }

    // ----- preferences and profiles -----

    fn show_preferences(self: &Rc<Self>) {
        let profile = Profile {
            connection: self.profile.borrow().connection.clone(),
            terminal: self.term.config(),
            decoder: self.session.decoder_config(),
        };
        let mcp = self.state.borrow().mcp.clone();
        let name = self.profile_name.borrow().clone();
        let w = Rc::downgrade(self);
        let presets: Vec<(String, gtkterm_core::presets::SerialPreset)> = self.manager.with_config(|c| {
            gtkterm_core::presets::all_presets(&c.presets)
                .into_iter()
                .map(|(n, p, _)| (n, p))
                .collect()
        });
        let wp = Rc::downgrade(self);
        let save_preset = move |name: String, preset: gtkterm_core::presets::SerialPreset| {
            let Some(w) = wp.upgrade() else { return };
            w.manager
                .with_config_mut(|c| c.presets.insert(name.clone(), preset));
            match w.manager.save_config() {
                Ok(()) => w.toast(&gettext_f("Preset {name} saved", &[("name", &name)])),
                Err(e) => w.toast(&e.to_string()),
            }
        };
        prefs::show(
            &self.window,
            &name,
            &profile,
            &mcp,
            self.mcp_url.borrow().clone(),
            self.state.borrow().script_allow_exec,
            presets,
            save_preset,
            move |p, m, exec| {
                if let Some(w) = w.upgrade() {
                    w.state.borrow_mut().script_allow_exec = exec;
                    w.apply_settings(p, m);
                }
            },
        );
    }

    fn apply_settings(self: &Rc<Self>, p: Profile, m: McpSettings) {
        if let Err(e) = p.validate() {
            self.toast(&gettext_f("Settings not applied: {error}", &[("error", &e)]));
            return;
        }
        let reconnect = self.is_open() && p.connection != self.profile.borrow().connection;
        self.term.apply_config(&p.terminal);
        self.session.set_terminal(p.terminal.clone());
        *self.profile.borrow_mut() = p;
        let name = self.profile_name.borrow().clone();
        self.store_profile(&name);
        self.refresh_header();
        self.sync_toggle_states();
        if reconnect {
            self.open_port();
        }
        let old = self.state.borrow().mcp.clone();
        self.state.borrow_mut().mcp = m.clone();
        if let Some(a) = self.window.lookup_action("mcp") {
            a.change_state(&m.enabled.to_variant());
        }
        if m.enabled && (m != old || self.mcp.borrow().is_none()) {
            self.stop_mcp();
            self.start_mcp(None);
        } else if !m.enabled {
            self.stop_mcp();
        }
        self.state.borrow().save();
    }

    fn store_profile(&self, name: &str) {
        let p = Profile {
            connection: self.profile.borrow().connection.clone(),
            terminal: self.term.config(),
            decoder: self.session.decoder_config(),
        };
        let result = self
            .manager
            .with_config_mut(|c| c.profile_mut(name).map(|slot| *slot = p))
            .and_then(|()| self.manager.save_config());
        match result {
            Ok(()) => self.toast(&gettext_f("Profile [{name}] saved", &[("name", &name)])),
            Err(e) => self.toast(&gettext_f("Cannot save profile: {error}", &[("error", &e)])),
        }
    }

    fn load_profile(self: &Rc<Self>, name: &str) {
        let Ok(p) = self.manager.with_config(|c| c.profile(name).cloned()) else {
            self.toast(&gettext_f("No profile [{name}]", &[("name", &name)]));
            return;
        };
        name.clone_into(&mut self.profile_name.borrow_mut());
        self.term.apply_config(&p.terminal);
        self.session.set_terminal(p.terminal.clone());
        self.session.apply_decoder_config(p.decoder);
        let reconnect = self.is_open();
        *self.profile.borrow_mut() = p;
        self.refresh_header();
        self.sync_toggle_states();
        self.status.update(&self.session.status());
        self.status.profile.set_label(&format!("[{name}]"));
        if reconnect {
            self.open_port();
        }
        self.toast(&gettext_f("Profile [{name}] loaded", &[("name", &name)]));
    }

    fn save_profile_as(self: &Rc<Self>) {
        let entry = gtk::Entry::builder()
            .placeholder_text(gettext("Profile name"))
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Save profile as"))
            .extra_child(&entry)
            .default_response("save")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", gettext("Cancel").as_str()),
            ("save", gettext("Save").as_str()),
        ]);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        let w = Rc::downgrade(self);
        dialog.connect_response(None, move |_, resp| {
            let Some(w) = w.upgrade() else { return };
            if resp != "save" {
                return;
            }
            let name = entry.text().trim().to_owned();
            if let Err(e) = gtkterm_core::config::check_profile_name(&name) {
                w.toast(&e.to_string());
                return;
            }
            w.store_profile(&name);
            w.profile_name.borrow_mut().clone_from(&name);
            w.status.profile.set_label(&format!("[{name}]"));
        });
        dialog.present(Some(&self.window));
    }

    fn delete_profile(self: &Rc<Self>) {
        let name = self.profile_name.borrow().clone();
        let dialog = adw::AlertDialog::builder()
            .heading(gettext_f("Delete profile [{name}]?", &[("name", &name)]))
            .body(if name == gtkterm_core::config::DEFAULT_PROFILE {
                gettext("The default profile is reset to default settings.")
            } else {
                gettext("The profile is removed from the configuration file.")
            })
            .close_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", gettext("Cancel").as_str()),
            ("delete", gettext("Delete").as_str()),
        ]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        let w = Rc::downgrade(self);
        dialog.connect_response(None, move |_, resp| {
            let Some(w) = w.upgrade() else { return };
            if resp != "delete" {
                return;
            }
            let result = w
                .manager
                .with_config_mut(|c| c.remove_profile(&name))
                .and_then(|()| w.manager.save_config());
            match result {
                Ok(()) => w.load_profile(gtkterm_core::config::DEFAULT_PROFILE),
                Err(e) => w.toast(&e.to_string()),
            }
        });
        dialog.present(Some(&self.window));
    }

    // ----- MCP -----

    fn start_mcp(self: &Rc<Self>, addr: Option<SocketAddr>) {
        if self.mcp.borrow().is_some() {
            return;
        }
        let settings = {
            let mut st = self.state.borrow_mut();
            if st.mcp.token.is_none() {
                st.mcp.token = Some(random_token());
            }
            st.save();
            st.mcp.clone()
        };
        let addr = addr.unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], settings.port)));
        let server = GtkTermMcp::new(
            Arc::clone(&self.manager),
            ServerOptions {
                read_only: settings.read_only,
                allowed_dirs: self.mcp_allow_dirs.borrow().clone(),
                allow_exec: self.state.borrow().mcp.allow_exec,
                audit: Some(gtkterm_mcp::default_audit_path()),
                approver: (settings.confirm_writes && !settings.read_only).then(|| self.approver()),
                ..ServerOptions::default()
            },
        );
        let security = HttpSecurity {
            token: settings.token.clone(),
            ..HttpSecurity::default()
        };
        let w = Rc::downgrade(self);
        spawn(
            async move { serve_http(server, addr, security).await },
            move |r| {
                let Some(w) = w.upgrade() else { return };
                match r {
                    Ok(handle) => {
                        let url = handle.url.clone();
                        let token = settings.token.unwrap_or_default();
                        w.status.set_mcp(Some(&url));
                        *w.mcp_url.borrow_mut() = Some(url.clone());
                        *w.mcp.borrow_mut() = Some(handle);
                        let command = format!(
                            "claude mcp add --transport http gtkterm {url} --header \"Authorization: Bearer {token}\""
                        );
                        let toast = adw::Toast::builder()
                            .title(gettext_f("MCP server on {url}", &[("url", &url)]))
                            .button_label(gettext("Copy setup command"))
                            .timeout(8)
                            .build();
                        let win = w.window.clone();
                        toast.connect_button_clicked(move |_| win.clipboard().set_text(&command));
                        w.toast.add_toast(toast);
                    }
                    Err(e) => {
                        w.toast(&gettext_f(
                            "MCP server could not start: {error}",
                            &[("error", &e)],
                        ));
                        if let Some(a) = w.window.lookup_action("mcp") {
                            a.change_state(&false.to_variant());
                        }
                    }
                }
            },
        );
    }

    /// Approval hook of the embedded MCP server: questions arrive from the I/O runtime and are
    /// answered one at a time with a dialog on the main thread.
    fn approver(self: &Rc<Self>) -> Approver {
        let (tx, rx) =
            async_channel::unbounded::<(ApprovalRequest, tokio::sync::oneshot::Sender<Approval>)>();
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok((request, mut reply)) = rx.recv().await {
                let Some(win) = w.upgrade().map(|w| w.window.clone()) else {
                    let _ = reply.send(Approval::Deny);
                    continue;
                };
                let dialog = approval_dialog(&request);
                let choice = dialog.clone().choose_future(Some(&win));
                let answer = tokio::select! {
                    r = choice => match r.as_str() {
                        "always" => Approval::Always,
                        "once" => Approval::Once,
                        _ => Approval::Deny,
                    },
                    // The request timed out or the agent went away.
                    () = reply.closed() => {
                        dialog.force_close();
                        continue;
                    }
                };
                let _ = reply.send(answer);
            }
        });
        Approver::new(move |request| {
            let tx = tx.clone();
            async move {
                let (reply, answer) = tokio::sync::oneshot::channel();
                if tx.send((request, reply)).await.is_err() {
                    return Approval::Deny;
                }
                match tokio::time::timeout(APPROVAL_TIMEOUT, answer).await {
                    Ok(Ok(a)) => a,
                    _ => Approval::Deny,
                }
            }
        })
    }

    fn stop_mcp(&self) {
        if let Some(h) = self.mcp.borrow_mut().take() {
            spawn_io(h.shutdown());
        }
        self.mcp_url.replace(None);
        self.status.set_mcp(None);
    }

    fn shutdown(&self) {
        if let Some(w) = self.script_window.borrow_mut().take() {
            w.window.destroy();
        }
        if let Some(w) = self.signal_window.borrow_mut().take() {
            w.window.destroy();
        }
        {
            let mut st = self.state.borrow_mut();
            st.maximized = self.window.is_maximized();
            st.fullscreen = self.window.is_fullscreen();
            st.font_scale = self.term.term.font_scale();
            if !st.maximized && !st.fullscreen {
                st.width = self.window.width();
                st.height = self.window.height();
            }
            st.pane_position = Some(self.paned.position());
            st.panel_visible = self.panel.widget.is_visible();
            st.frames_visible = self.frames.widget.is_visible();
            if st.frames_visible {
                st.frames_position = Some(self.term_paned.position());
            }
            st.save();
        }
        self.stop_mcp();
        self.watcher.borrow_mut().take();
        let s = self.session.clone();
        // Close synchronously enough that the device lock and termios are released.
        let _ = rt().block_on(async move { tokio::time::timeout(Duration::from_secs(2), s.close()).await });
    }
}

/// Dropdown item factory with middle-ellipsized labels (long device paths).
fn ellipsizing_factory(max_chars: i32) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::Middle)
            .max_width_chars(max_chars)
            .build();
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&label));
        }
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let text = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .map(|o| o.string());
        if let (Some(label), Some(text)) = (item.child().and_downcast::<gtk::Label>(), text) {
            label.set_text(&text);
            label.set_tooltip_text(Some(&text));
        }
    });
    factory
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let s = gio::Menu::new();
    s.append(Some(&gettext("Preferences")), Some("win.preferences"));
    menu.append_section(None, &s);

    let data = gio::Menu::new();
    data.append(Some(&gettext("Send Hex Data…")), Some("win.send-hex"));
    data.append(Some(&gettext("Send File…")), Some("win.send-file"));
    data.append(Some(&gettext("Receive File…")), Some("win.receive-file"));
    data.append(
        Some(&gettext("Cancel File Transfer")),
        Some("win.cancel-transfer"),
    );
    data.append(Some(&gettext("Save Received Data…")), Some("win.save-raw"));
    let log = gio::Menu::new();
    log.append(Some(&gettext("Start Logging…")), Some("win.log-start"));
    log.append(Some(&gettext("Pause / Resume")), Some("win.log-pause"));
    log.append(Some(&gettext("Clear Log File")), Some("win.log-clear"));
    log.append(Some(&gettext("Stop Logging")), Some("win.log-stop"));
    data.append_submenu(Some(&gettext("Logging")), &log);
    menu.append_section(None, &data);

    let control = gio::Menu::new();
    control.append(Some(&gettext("Send Break")), Some("win.send-break"));
    control.append(Some(&gettext("Flush Buffers")), Some("win.flush"));
    control.append(Some(&gettext("Detect Baud Rate")), Some("win.auto-baud"));
    control.append(Some(&gettext("Toggle DTR")), Some("win.dtr"));
    control.append(Some(&gettext("Toggle RTS")), Some("win.rts"));
    menu.append_section(None, &control);

    let view = gio::Menu::new();
    view.append(Some(&gettext("Local Echo")), Some("win.echo"));
    view.append(Some(&gettext("Auto CR (LF → CR LF)")), Some("win.auto-cr"));
    view.append(Some(&gettext("Auto LF (CR → CR LF)")), Some("win.auto-lf"));
    view.append(Some(&gettext("Timestamps")), Some("win.timestamps"));
    view.append(Some(&gettext("Show Byte Index")), Some("win.show-index"));
    let modes = gio::Menu::new();
    for m in ViewMode::ALL {
        let item = gio::MenuItem::new(Some(&gettext(m.label())), None);
        item.set_action_and_target_value(
            Some("win.view-mode"),
            Some(&format!("{m:?}").to_lowercase().to_variant()),
        );
        modes.append_item(&item);
    }
    view.append_submenu(Some(&gettext("View Mode")), &modes);
    let widths = gio::Menu::new();
    for n in HEX_WIDTHS {
        let item = gio::MenuItem::new(Some(&gettext_f("{n} bytes", &[("n", &n)])), None);
        item.set_action_and_target_value(Some("win.hex-width"), Some(&(n as i32).to_variant()));
        widths.append_item(&item);
    }
    view.append_submenu(Some(&gettext("Hex Width")), &widths);
    let zoom = gio::Menu::new();
    zoom.append(Some(&gettext("Zoom In")), Some("win.zoom-in"));
    zoom.append(Some(&gettext("Zoom Out")), Some("win.zoom-out"));
    zoom.append(Some(&gettext("Normal Size")), Some("win.zoom-reset"));
    zoom.append(Some(&gettext("Fullscreen")), Some("win.fullscreen"));
    view.append_submenu(Some(&gettext("Zoom")), &zoom);
    view.append(Some(&gettext("Clear Screen")), Some("win.clear-screen"));
    view.append(Some(&gettext("Remove All Charts")), Some("win.clear-charts"));
    menu.append_section(None, &view);

    let decode = gio::Menu::new();
    decode.append(Some(&gettext("Decoded Frames")), Some("win.frames"));
    let decoders = gio::Menu::new();
    for m in DecoderMode::all() {
        let item = gio::MenuItem::new(Some(&gettext(m.label())), None);
        item.set_action_and_target_value(Some("win.decoder"), Some(&m.to_string().to_variant()));
        decoders.append_item(&item);
    }
    decode.append_submenu(Some(&gettext("Decoder")), &decoders);
    decode.append(
        Some(&gettext("Record Modem Line Changes")),
        Some("win.decoder-signals"),
    );
    decode.append(Some(&gettext("Clear Frames")), Some("win.clear-frames"));
    menu.append_section(None, &decode);

    let auto = gio::Menu::new();
    auto.append(Some(&gettext("Scripts…")), Some("win.scripts"));
    auto.append(Some(&gettext("Signal Monitor")), Some("win.signal-monitor"));
    menu.append_section(None, &auto);

    let app = gio::Menu::new();
    app.append(Some(&gettext("Import Presets…")), Some("win.import-presets"));
    app.append(Some(&gettext("MCP Server for AI Agents")), Some("win.mcp"));
    app.append(Some(&gettext("Keyboard Shortcuts")), Some("win.shortcuts"));
    app.append(Some(&gettext("About GTKTerm Sunshine")), Some("win.about"));
    app.append(Some(&gettext("Quit")), Some("app.quit"));
    menu.append_section(None, &app);
    menu
}

fn macro_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let list = gio::Menu::new();
    // Built when the menu opens; the names come from the snapshot `rebuild_macros` keeps.
    MACRO_NAMES.with(|names| {
        for (name, shortcut) in names.borrow().iter() {
            let label = match shortcut {
                Some(s) => format!("{name}  ·  {}", super::macros::shortcut_label(s)),
                None => name.clone(),
            };
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(Some("win.play-macro"), Some(&name.to_variant()));
            list.append_item(&item);
        }
    });
    menu.append_section(Some(&gettext("Play")), &list);
    let ops = gio::Menu::new();
    ops.append(Some(&gettext("Record")), Some("win.record-macro"));
    ops.append(Some(&gettext("Stop Running Macro")), Some("win.stop-macro"));
    ops.append(Some(&gettext("Edit Macros…")), Some("win.edit-macros"));
    ops.append(Some(&gettext("Show Macro Bar")), Some("win.macro-bar"));
    menu.append_section(None, &ops);
    menu
}

thread_local! {
    static MACRO_NAMES: RefCell<Vec<(String, Option<String>)>> = const { RefCell::new(Vec::new()) };
}

fn shortcuts_dialog() -> adw::ShortcutsDialog {
    let dialog = adw::ShortcutsDialog::new();
    let mut sections: Vec<(&str, adw::ShortcutsSection)> = Vec::new();
    for (section, title, _, accel) in SHORTCUTS {
        let pos = sections
            .iter()
            .position(|(s, _)| s == section)
            .unwrap_or_else(|| {
                sections.push((section, adw::ShortcutsSection::new(Some(&gettext(*section)))));
                sections.len() - 1
            });
        sections[pos]
            .1
            .add(adw::ShortcutsItem::new(&gettext(*title), accel));
    }
    let typing = adw::ShortcutsSection::new(Some(&gettext("Device")));
    typing.add(adw::ShortcutsItem::new(
        &gettext("Control characters (Ctrl+C, Ctrl+A, …) go to the device"),
        "<Control>c",
    ));
    sections.push(("Device", typing));
    for (_, s) in sections {
        dialog.add(s);
    }
    dialog
}

/// How long an agent's write waits for the user before it is denied.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// "Allow the agent to …?" for one tool call.
fn approval_dialog(request: &ApprovalRequest) -> adw::AlertDialog {
    let client = request.client.clone().unwrap_or_else(|| gettext("An AI agent"));
    let dialog = adw::AlertDialog::builder()
        .heading(gettext_f("Allow {tool}?", &[("tool", &request.tool)]))
        .body(gettext_f(
            "{client} wants to run {tool} with {arguments}.\nIt can send data to the device, change the connection or write files.",
            &[
                ("client", &client),
                ("tool", &request.tool),
                ("arguments", &request.arguments),
            ],
        ))
        .close_response("deny")
        .default_response("once")
        .build();
    dialog.add_responses(&[
        ("deny", gettext("Deny").as_str()),
        ("once", gettext("Allow Once").as_str()),
        ("always", gettext("Allow Until the Server Stops").as_str()),
    ]);
    dialog.set_response_appearance("deny", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("once", adw::ResponseAppearance::Suggested);
    dialog
}

/// Versions and system details for bug reports (About → Troubleshooting).
fn debug_info() -> String {
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| "unknown".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let kernel = kernel.trim();
    format!(
        "GTKTerm Sunshine {}\nOS: {os} (Linux {kernel})\nGTK {}.{}.{}\nlibadwaita {}.{}.{}\nRenderer: {}\nDesktop: {}\n",
        env!("CARGO_PKG_VERSION"),
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
        adw::major_version(),
        adw::minor_version(),
        adw::micro_version(),
        std::env::var("GSK_RENDERER").unwrap_or_else(|_| "default".into()),
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into()),
    )
}

fn about_dialog() -> adw::AboutDialog {
    adw::AboutDialog::builder()
        .debug_info(debug_info())
        .debug_info_filename("gtkterm-sunshine-debug.txt")
        .application_name("GTKTerm Sunshine")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Philipp")
        .license_type(gtk::License::Gpl30)
        .comments(gettext(
            "Serial, TCP and UDP terminal with live Sunshine data plots and an MCP server for AI agents.\nRust rewrite of the GTKTerm fork.",
        ))
        .website("http://192.168.1.100:8460/philipp/gtkterm-sin-of-pride-version")
        .copyright(gettext("© 2025-2026 Philipp; GTKTerm © Julien Schmitt and contributors"))
        .build()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use gtkterm_core::config::Config;
    use gtkterm_core::port::loopback;

    use super::*;

    static UNEXPECTED_LOGS: AtomicUsize = AtomicUsize::new(0);

    /// Count GTK warnings/criticals. Two known upstream issues are ignored:
    /// * libadwaita 1.9: "AdwGizmo (tabboxchild) reported min width -6" during the tab-open animation;
    /// * GTK 4.22: every GtkSwitch's state icons report invalid baselines (a bare `gtk::Switch` in a
    ///   window reproduces it, so AdwSwitchRow in the preferences triggers it too).
    fn install_log_guard() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            glib::log_set_writer_func(|level, fields| {
                let message = fields
                    .iter()
                    .find(|f| f.key() == "MESSAGE")
                    .and_then(|f| f.value_str())
                    .unwrap_or("");
                let serious = matches!(
                    level,
                    glib::LogLevel::Warning | glib::LogLevel::Critical | glib::LogLevel::Error
                );
                let known = message.contains("tabboxchild")
                    || (message.contains("GtkImage") && message.contains("baselines"));
                if serious && !known {
                    eprintln!("GTK {level:?}: {message}");
                    if UNEXPECTED_LOGS.load(Ordering::SeqCst) == 0 {
                        eprintln!("{}", std::backtrace::Backtrace::force_capture());
                    }
                    UNEXPECTED_LOGS.fetch_add(1, Ordering::SeqCst);
                }
                glib::LogWriterOutput::Handled
            });
        });
    }

    fn pump_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
        let ctx = glib::MainContext::default();
        let start = Instant::now();
        while start.elapsed() < timeout {
            while ctx.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// The device side of an XMODEM download: a native sender on the loopback port.
    async fn device_send(
        mut dev: gtkterm_core::port::loopback::DeviceEnd,
        data: Vec<u8>,
    ) -> gtkterm_core::port::loopback::DeviceEnd {
        use gtkterm_protocols::xmodem::{FileData, Sender, Timing, Transfer, Variant};
        let file = FileData {
            name: String::new(),
            data,
            mtime: None,
        };
        let mut m = Sender::new(Variant::Xmodem, vec![file], Timing::default()).unwrap();
        let t0 = std::time::Instant::now();
        let now = || u64::try_from(t0.elapsed().as_millis()).unwrap();
        let mut out = m.start(now());
        while !m.is_done() || !out.bytes.is_empty() {
            if !out.bytes.is_empty() {
                dev.send(std::mem::take(&mut out.bytes)).await.unwrap();
                out = m.drained(now());
                continue;
            }
            out = match tokio::time::timeout(Duration::from_millis(100), dev.recv()).await {
                Ok(Some(b)) => m.input(&b, now()),
                Ok(None) => break,
                Err(_) => m.tick(now()),
            };
        }
        dev
    }

    fn snapshot(widget: &impl IsA<gtk::Widget>, name: &str) {
        let paintable = gtk::WidgetPaintable::new(Some(widget));
        let (w, h) = (widget.width().max(1), widget.height().max(1));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let Some(node) = snapshot.to_node() else { return };
        let Some(renderer) = widget.native().and_then(|n| n.renderer()) else {
            return;
        };
        let texture = renderer.render_texture(node, None);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/gui-screenshots");
        let _ = std::fs::create_dir_all(&dir);
        let _ = texture.save_to_png(dir.join(format!("{name}.png")));
    }

    fn setup() -> (
        Rc<Win>,
        gtkterm_core::port::loopback::DeviceEnd,
        tempfile::TempDir,
    ) {
        install_log_guard();
        let dir = tempfile::tempdir().unwrap();
        super::super::state::set_test_dir(dir.path());
        let _guard = rt().enter();
        let app = adw::Application::builder()
            .application_id("at.philipp.GtkTermSunshine.Test")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let manager = SessionManager::new(Config::default(), None);
        let profile = Profile::default();
        let session = manager.create("default", profile.terminal.clone());
        let start = StartOptions {
            open_at_start: false,
            mcp_addr: None,
            mcp_token: None,
            mcp_read_only: false,
            mcp_allow_dirs: Vec::new(),
            script: None,
            allow_exec: false,
            log: None,
            notice: None,
        };
        let win = Win::new(
            &app,
            Arc::clone(&manager),
            session.clone(),
            "default".into(),
            profile,
            start,
        );
        let (port, dev) = loopback::pair_with_lines("loopback");
        rt().block_on(session.attach(port)).unwrap();
        assert!(pump_until(Duration::from_secs(5), || win.is_open()));
        (win, dev, dir)
    }

    #[gtk::test]
    fn window_charts_actions_and_dialogs() {
        // Opens real windows: only run when asked (xtask/CI run it inside xvfb-run).
        if std::env::var_os("GTKTERM_GUI_TESTS").is_none() {
            eprintln!("skipped: set GTKTERM_GUI_TESTS=1 (and use xvfb-run) to run the GUI test");
            return;
        }
        let (win, mut dev, _dir) = setup();
        win.window.set_default_size(1300, 800);

        // Sunshine data creates one tab per ID with the right chart kind (C defect S5).
        rt().block_on(async {
            for i in 0..60 {
                let v = f64::from(i);
                dev.send(format!("boot line {i}\r\n{{env|T|{v}}}\r\n{{B:bar|a|{v}|b|3}}\r\n{{P:pie|x|1|y|2}}\r\n{{S:xy|p|{v} {}}}\r\n", v * v).into_bytes()).await.unwrap();
            }
        });
        assert!(
            pump_until(Duration::from_secs(5), || win.panel.len() == 4),
            "tabs: {}",
            win.panel.len()
        );
        assert!(win.panel.widget.is_visible(), "panel opens with the first chart");
        let kinds: Vec<_> = win
            .session
            .with_sunshine(|s| s.charts().iter().map(|c| c.kind).collect());
        use gtkterm_protocols::sunshine::ChartKind::{Bar, Line, Pie, Scatter};
        assert_eq!(kinds, [Line, Bar, Pie, Scatter]);
        pump_until(Duration::from_millis(300), || false);
        snapshot(&win.window, "main-ascii");

        // Keyboard input reaches the device.
        win.term.term.feed_child(b"AT\r");
        let got = rt().block_on(async { tokio::time::timeout(Duration::from_secs(2), dev.recv()).await });
        assert_eq!(got.ok().flatten().as_deref(), Some(&b"AT\r"[..]));

        // Every action that does not need a file chooser works without GTK warnings.
        for action in [
            "copy",
            "paste",
            "select-all",
            "search",
            "clear-screen",
            "clear-scrollback",
            "echo",
            "auto-cr",
            "auto-lf",
            "timestamps",
            "show-index",
            "visualization",
            "visualization",
            "shortcuts",
            "about",
            "zoom-in",
            "zoom-out",
            "zoom-reset",
            "send-break",
            "clear-charts",
        ] {
            gio::prelude::ActionGroupExt::activate_action(&win.window, action, None);
            pump_until(Duration::from_millis(50), || false);
        }
        for mode in ViewMode::ALL {
            let target = format!("{mode:?}").to_lowercase();
            gio::prelude::ActionGroupExt::activate_action(
                &win.window,
                "view-mode",
                Some(&target.to_variant()),
            );
            pump_until(Duration::from_millis(20), || false);
            assert_eq!(win.term.config().view_mode, mode);
        }
        gio::prelude::ActionGroupExt::activate_action(&win.window, "hex-width", Some(&8i32.to_variant()));
        assert_eq!(win.term.config().hex_chars, 8);
        gio::prelude::ActionGroupExt::activate_action(&win.window, "view-mode", Some(&"mixed".to_variant()));
        rt().block_on(dev.send(&b"Hello\x00\x01\xff\n"[..])).unwrap();
        pump_until(Duration::from_millis(300), || false);
        snapshot(&win.window, "main-mixed-hex");

        gio::prelude::ActionGroupExt::activate_action(&win.window, "preferences", None);
        pump_until(Duration::from_millis(300), || false);
        snapshot(&win.window, "preferences");
        gio::prelude::ActionGroupExt::activate_action(&win.window, "send-hex", None);
        pump_until(Duration::from_millis(200), || false);

        // Charts were removed by clear-charts; new data brings them back.
        rt().block_on(dev.send(&b"{env|T|1}\n"[..])).unwrap();
        assert!(pump_until(Duration::from_secs(2), || win.panel.len() == 1));

        // Macros: a stored macro with a shortcut is bound and plays via its action.
        win.manager
            .update_macros(|s| {
                s.upsert(
                    None,
                    gtkterm_core::macros::Macro {
                        name: "hello".into(),
                        shortcut: Some("F9".into()),
                        commands: gtkterm_core::macros::parse_text("send: hi\\r\n").unwrap(),
                    },
                )
            })
            .unwrap();
        assert!(pump_until(Duration::from_secs(2), || win
            .shortcut_ctl
            .borrow()
            .as_ref()
            .is_some_and(|c| c.upcast_ref::<gio::ListModel>().n_items() == 1)));
        assert_eq!(
            win.macro_bar.observe_children().n_items(),
            2,
            "one macro button plus edit"
        );
        rt().block_on(async { while !dev.drain().is_empty() {} });
        gio::prelude::ActionGroupExt::activate_action(&win.window, "play-macro", Some(&"hello".to_variant()));
        let got = rt().block_on(async { tokio::time::timeout(Duration::from_secs(2), dev.recv()).await });
        assert_eq!(got.ok().flatten().as_deref(), Some(&b"hi\r"[..]));

        // Recording: typed input becomes a macro, the editor opens on it.
        gio::prelude::ActionGroupExt::activate_action(&win.window, "record-macro", None);
        win.term.term.feed_child(b"AT\r");
        pump_until(Duration::from_millis(100), || false);
        gio::prelude::ActionGroupExt::activate_action(&win.window, "record-macro", None);
        let recorded = win
            .manager
            .with_macros(|s| s.get("Recorded").cloned())
            .expect("recorded macro");
        assert_eq!(
            recorded.commands,
            vec![gtkterm_core::macros::MacroCommand::Send {
                data: "AT\\r".into(),
                delay_ms: 0
            }]
        );
        pump_until(Duration::from_millis(300), || false);
        snapshot(&win.window, "macro-editor");
        gio::prelude::ActionGroupExt::activate_action(&win.window, "macro-bar", None);
        pump_until(Duration::from_millis(50), || false);

        // Decoders: choosing one shows the frames pane; requests (TX) and responses (RX) are listed.
        {
            use gtkterm_protocols::decode::modbus;
            gio::prelude::ActionGroupExt::activate_action(
                &win.window,
                "decoder",
                Some(&"modbus_rtu".to_variant()),
            );
            assert!(win.frames.widget.is_visible());
            win.session
                .send_nowait(modbus::rtu_frame(1, &[0x03, 0x00, 0x00, 0x00, 0x01]))
                .unwrap();
            rt().block_on(dev.send(modbus::rtu_frame(1, &[0x03, 0x02, 0x00, 0x2A])))
                .unwrap();
            assert!(pump_until(Duration::from_secs(2), || win.frames.len() == 2));
            let summaries = win.frames.summaries();
            assert!(
                summaries.iter().any(|s| s.contains("[1 register] 0x002A")),
                "{summaries:?}"
            );
            rt().block_on(dev.send(vec![0x01, 0x03])).unwrap();
            assert!(pump_until(Duration::from_secs(2), || win.frames.len() == 3));
            pump_until(Duration::from_millis(200), || false);
            snapshot(&win.frames.widget, "frames");
            gio::prelude::ActionGroupExt::activate_action(&win.window, "decoder-signals", None);
            assert!(win.session.decoder_status().signals);
            gio::prelude::ActionGroupExt::activate_action(&win.window, "decoder-signals", None);
            assert!(!win.session.decoder_status().signals);
            gio::prelude::ActionGroupExt::activate_action(&win.window, "clear-frames", None);
            assert!(pump_until(Duration::from_secs(1), || win.frames.len() == 0));
            gio::prelude::ActionGroupExt::activate_action(&win.window, "decoder", Some(&"off".to_variant()));
            gio::prelude::ActionGroupExt::activate_action(&win.window, "frames", None);
            assert!(!win.frames.widget.is_visible());
            rt().block_on(async { while !dev.drain().is_empty() {} });
        }

        // File transfers: a device-side XMODEM sender; the file lands in the chosen folder and the
        // status bar indicator goes away again. Then a transfer nobody answers is cancelled.
        {
            use gtkterm_core::transfer::{TransferProtocol, TransferRequest, TransferState};
            let dir = tempfile::tempdir().unwrap();
            let data: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
            rt().block_on(win.session.start_transfer(TransferRequest::Receive {
                protocol: TransferProtocol::Xmodem,
                dir: dir.path().to_owned(),
                name: Some("dl.bin".into()),
                overwrite: false,
            }))
            .unwrap();
            let device = rt().spawn(device_send(dev, data.clone()));
            assert!(pump_until(Duration::from_secs(10), || win
                .session
                .transfer_status()
                .is_some_and(|t| t.state != TransferState::Running)));
            dev = rt().block_on(device).unwrap();
            pump_until(Duration::from_millis(100), || false);
            assert_eq!(
                win.session.transfer_status().unwrap().state,
                TransferState::Finished
            );
            assert_eq!(std::fs::read(dir.path().join("dl.bin")).unwrap(), data);
            assert!(!win.status.transfer_visible());

            rt().block_on(win.session.start_transfer(TransferRequest::Receive {
                protocol: TransferProtocol::Ymodem,
                dir: dir.path().to_owned(),
                name: None,
                overwrite: false,
            }))
            .unwrap();
            assert!(pump_until(Duration::from_secs(2), || win
                .status
                .transfer_visible()));
            gio::prelude::ActionGroupExt::activate_action(&win.window, "cancel-transfer", None);
            assert!(pump_until(Duration::from_secs(2), || win
                .session
                .transfer_status()
                .is_some_and(|t| t.state == TransferState::Cancelled)));
            rt().block_on(async { while !dev.drain().is_empty() {} });
            for action in ["send-file", "receive-file"] {
                gio::prelude::ActionGroupExt::activate_action(&win.window, action, None);
                pump_until(Duration::from_millis(150), || false);
            }
            snapshot(&win.window, "transfer-dialog");
        }

        // Scripts: the editor runs on the window's session; logs, status and parse errors show.
        {
            let sw = win.script_window();
            sw.set_text("send PING\\r\nexpect PONG (\\d)\nlog got ${MATCH_1}\n");
            sw.present();
            sw.start();
            let got = rt().block_on(async { tokio::time::timeout(Duration::from_secs(2), dev.recv()).await });
            assert_eq!(got.ok().flatten().as_deref(), Some(&b"PING\r"[..]));
            rt().block_on(dev.send(&b"PONG 5\r\n"[..])).unwrap();
            assert!(
                pump_until(Duration::from_secs(3), || sw
                    .status_text()
                    .starts_with("Finished")),
                "{}",
                sw.status_text()
            );
            assert!(
                pump_until(Duration::from_secs(2), || sw.log_text().contains("got 5")),
                "{}",
                sw.log_text()
            );
            sw.set_text("send a\nbogus\n");
            sw.start();
            assert!(
                sw.status_text().contains(":2: unknown command"),
                "{}",
                sw.status_text()
            );
            pump_until(Duration::from_millis(200), || false);
            snapshot(&sw.window, "script-editor");
            sw.window.close();
            rt().block_on(async { while !dev.drain().is_empty() {} });
        }

        // Signal monitor: DTR from the window, CTS from the device, drawn as traces.
        {
            gio::prelude::ActionGroupExt::activate_action(&win.window, "signal-monitor", None);
            gio::prelude::ActionGroupExt::activate_action(&win.window, "dtr", None);
            dev.set_inputs(gtkterm_core::port::loopback::InputLines {
                cts: true,
                ..Default::default()
            });
            assert!(pump_until(Duration::from_secs(2), || win
                .session
                .signal_history(None)
                .iter()
                .any(|s| s.changed.contains(&"CTS"))));
            pump_until(Duration::from_millis(300), || false);
            let m = win.signal_window.borrow().clone().unwrap();
            snapshot(&m.window, "signal-monitor");
            m.window.close();
            for action in ["flush", "auto-baud"] {
                gio::prelude::ActionGroupExt::activate_action(&win.window, action, None);
                pump_until(Duration::from_millis(50), || false);
            }
        }

        // Echo off by default, toggled on above: the TX chunk is displayed (no assertion on VTE
        // contents, but the path must not warn).
        win.term.term.feed_child(b"x");
        pump_until(Duration::from_millis(100), || false);

        // The MCP approval dialog builds and closes without warnings.
        let dialog = approval_dialog(&ApprovalRequest {
            tool: "send".into(),
            arguments: r#"{"data":"AT"}"#.into(),
            client: Some("claude-code".into()),
        });
        dialog.present(Some(&win.window));
        pump_until(Duration::from_millis(100), || false);
        dialog.force_close();
        pump_until(Duration::from_millis(100), || false);

        // SIGUSR2 closes the port (C GTKTerm compatibility).
        win.watch_unix_signals();
        nix::sys::signal::raise(nix::sys::signal::Signal::SIGUSR2).unwrap();
        assert!(
            pump_until(Duration::from_secs(5), || !win.is_open()),
            "SIGUSR2 closes"
        );

        win.window.close();
        pump_until(Duration::from_millis(200), || false);
        // Tear everything down: finalizing widgets must not warn (e.g. popovers still parented).
        let window = win.window.clone();
        let term = win.term.term.downgrade();
        let term_box = win.term.widget.downgrade();
        let paned = win.paned.downgrade();
        let wweak = win.window.downgrade();
        let tv = Rc::downgrade(&win.term);
        let rc = Rc::downgrade(&win);
        drop(win);
        window.destroy();
        drop(window);
        pump_until(Duration::from_millis(300), || false);
        // No reference cycles: the whole window, including the terminal, must be finalized.
        assert!(
            rc.upgrade().is_none() && tv.upgrade().is_none(),
            "Win/TerminalView leaked"
        );
        for (name, alive) in [
            ("window", wweak.upgrade().is_some()),
            ("paned", paned.upgrade().is_some()),
            ("terminal box", term_box.upgrade().is_some()),
            ("terminal", term.upgrade().is_some()),
        ] {
            assert!(!alive, "{name} was not finalized (reference cycle)");
        }
        assert_eq!(
            UNEXPECTED_LOGS.load(Ordering::SeqCst),
            0,
            "GTK reported warnings or criticals"
        );
    }
}
