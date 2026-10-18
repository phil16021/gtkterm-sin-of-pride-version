//! GTK user interface.
// GTK signal wiring: closures declared mid-function, widget handles moved into callbacks by
// value, and builder bindings with similar names are the normal style here.
#![allow(
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::type_complexity,
    clippy::similar_names
)]

mod charts;
mod frames;
mod macros;
mod prefs;
mod rt;
mod scripts;
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
    gio::resources_register_include!("gtkterm.gresource").expect("embedded resources");
    let _guard = rt::rt().enter();
    let manager = SessionManager::new(resolved.config.clone(), Some(resolved.config_path.clone()));
    manager.set_macros(resolved.macros.0.clone(), Some(resolved.macros.1.clone()));
    let session = manager.create(&resolved.profile_name, resolved.profile.terminal.clone());
    session.apply_decoder_config(resolved.profile.decoder);

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
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
            w.close();
        }
    });
    app.add_action(&quit);

    let start = RefCell::new(Some(window::StartOptions {
        open_at_start: true,
        mcp_addr: cli.mcp_http,
        mcp_token: cli.mcp_token.clone(),
        mcp_read_only: cli.mcp_read_only,
        mcp_allow_dirs: cli.mcp_allow_dir.clone(),
        script: cli.script.clone(),
        allow_exec: cli.allow_exec,
        log: cli.log_file.clone().map(|p| (p, cli.effective_log_format())),
        notice: resolved.import_message.clone(),
    }));
    let holder: Rc<RefCell<Option<Rc<window::Win>>>> = Rc::default();
    let h = Rc::clone(&holder);
    app.connect_activate(move |app| {
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
        *h.borrow_mut() = Some(win);
    });
    let status = app.run_with_args::<&str>(&["gtkterm-sunshine"]);
    holder.borrow_mut().take();
    if status == glib::ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
