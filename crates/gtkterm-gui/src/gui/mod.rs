//! GTK user interface.
// GTK signal wiring: closures declared mid-function, widget handles moved into callbacks by
// value, and builder bindings with similar names are the normal style here.
#![allow(
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::type_complexity,
    clippy::similar_names
)]

mod activity;
mod charts;
mod frames;
pub mod i18n;
mod macros;
mod prefs;
mod rt;
mod scripts;
mod sendbar;
mod shortcuts;
mod signals;
mod state;
mod statusbar;
mod terminal;
mod transfer;
mod window;

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use adw::prelude::*;
use gtkterm_core::manager::SessionManager;

use crate::cli::{Cli, Resolved};

/// Application ID (D-Bus name, desktop file, icon).
pub const APP_ID: &str = "at.philipp.GtkTermSunshine";

/// Start the GUI and run until the window is closed.
pub fn run(cli: Cli, resolved: Resolved) -> ExitCode {
    i18n::init();
    gio::resources_register_include!("gtkterm.gresource").expect("embedded resources");
    let _guard = rt::rt().enter();
    let manager = SessionManager::new(resolved.config.clone(), Some(resolved.config_path.clone()));
    manager.set_macros(resolved.macros.0.clone(), Some(resolved.macros.1.clone()));
    let session = manager.create(&resolved.profile_name, resolved.profile.terminal.clone());
    session.apply_decoder_config(resolved.profile.decoder);

    // One instance per user: a second launch hands its command line to the running window
    // (a new tab, script files). A separate configuration file gets its own instance.
    let flags = if cli.config_file.is_some() {
        gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_COMMAND_LINE
    } else {
        gio::ApplicationFlags::HANDLES_COMMAND_LINE
    };
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_resource("/at/philipp/GtkTermSunshine/style.css");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            gtk::IconTheme::for_display(&display).add_resource_path("/at/philipp/GtkTermSunshine/icons");
        }
    });
    let quit = gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| {
        for w in a.windows() {
            match w.downcast::<adw::ApplicationWindow>() {
                Ok(w) => window::close_window(&w),
                Err(w) => w.close(),
            }
        }
    });
    app.add_action(&quit);

    let start = RefCell::new(Some(window::StartOptions {
        open_at_start: true,
        mcp_addr: cli.mcp_http,
        mcp_token: cli.mcp_token.clone(),
        mcp_read_only: cli.mcp_read_only,
        mcp_rate: cli.mcp_rate,
        mcp_allow_dirs: cli.mcp_allow_dir.clone(),
        script: cli.script.clone(),
        allow_exec: cli.allow_exec,
        log: cli.log_file.clone().map(|p| (p, cli.effective_log_format())),
        notice: resolved.import_message.clone(),
    }));
    let holder: Rc<RefCell<Option<Rc<window::Win>>>> = Rc::default();
    let h = Rc::clone(&holder);
    let files = cli.files.clone();
    let open_window = move |app: &adw::Application| {
        if let Some(win) = h.borrow().as_ref() {
            win.window.present();
            return;
        }
        let Some(start) = start.borrow_mut().take() else {
            return;
        };
        let win = window::Win::new(
            app,
            manager.clone(),
            session.clone(),
            resolved.profile_name.clone(),
            resolved.profile.clone(),
            start,
        );
        win.watch_unix_signals();
        win.restore_tabs();
        win.open_scripts(&files);
        *h.borrow_mut() = Some(win);
    };
    let open_window = Rc::new(open_window);
    let ow = Rc::clone(&open_window);
    app.connect_activate(move |app| ow(app));
    let h = Rc::clone(&holder);
    app.connect_command_line(move |app, cmdline| {
        if !cmdline.is_remote() {
            open_window(app);
            return glib::ExitCode::SUCCESS;
        }
        let window = h.borrow().clone();
        if let Some(win) = window {
            remote_launch(&win, cmdline)
        } else {
            open_window(app);
            glib::ExitCode::SUCCESS
        }
    });
    // The real arguments, so a second launch can hand them over; this process parsed them
    // already (clap), GTK sees no options of its own.
    let args: Vec<String> = std::env::args().collect();
    let status = app.run_with_args(&args);
    holder.borrow_mut().take();
    if status == glib::ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// A second `gtkterm-sunshine` started while this window is open: open the tab or scripts it
/// asks for. Its own process exits with the returned status.
fn remote_launch(win: &Rc<window::Win>, cmdline: &gio::ApplicationCommandLine) -> glib::ExitCode {
    use clap::Parser as _;
    let mut cli = match Cli::try_parse_from(cmdline.arguments()) {
        Ok(c) => c,
        Err(e) => {
            cmdline.printerr_literal(&e.render().to_string());
            return glib::ExitCode::from(2);
        }
    };
    if let Some(cwd) = cmdline.cwd() {
        cli.absolutize(&cwd);
    }
    let ignored = cli.ignored_by_running_window();
    if !ignored.is_empty() {
        cmdline.printerr_literal(&format!(
            "gtkterm-sunshine: the window is already running; ignored: {}\n",
            ignored.join(", ")
        ));
    }
    let mut target = Rc::clone(win);
    if cli.wants_tab() {
        let config = win.manager_config();
        let (name, profile, explicit) = match cli.resolve_profile(&config) {
            Ok(r) => r,
            Err(e) => {
                cmdline.printerr_literal(&format!("gtkterm-sunshine: {e}\n"));
                return glib::ExitCode::from(2);
            }
        };
        let connect = explicit || win.connects_at_start();
        target = win.open_tab_with(&name, profile, connect);
        if let Some(script) = cli.script.clone() {
            target.run_script_when_connected(script, cli.allow_exec);
        }
    }
    target.open_scripts(&cli.files);
    target.present_tab();
    glib::ExitCode::SUCCESS
}
