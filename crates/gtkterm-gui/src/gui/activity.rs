//! Agent activity: the recent tool calls of the embedded MCP server, newest first, with outcome
//! and duration, a filter and "problems only".

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtkterm_mcp::ToolCall;
use jiff::tz::TimeZone;

use super::i18n::{gettext, gettext_f};

/// Calls kept.
const KEEP: usize = 500;

/// The calls and, once opened, their window.
pub struct Activity {
    /// Newest first; index i is row i of the list.
    entries: Rc<RefCell<VecDeque<ToolCall>>>,
    view: RefCell<Option<View>>,
}

struct View {
    window: adw::Window,
    list: gtk::ListBox,
    title: adw::WindowTitle,
    #[cfg_attr(not(test), allow(dead_code))]
    search: gtk::SearchEntry,
    #[cfg_attr(not(test), allow(dead_code))]
    problems: gtk::ToggleButton,
}

fn is_problem(c: &ToolCall) -> bool {
    c.outcome != "ok"
}

fn matches(c: &ToolCall, text: &str, problems_only: bool) -> bool {
    if problems_only && !is_problem(c) {
        return false;
    }
    let text = text.trim().to_lowercase();
    text.is_empty()
        || [
            Some(&c.tool),
            Some(&c.arguments),
            c.client.as_ref(),
            c.error.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|s| s.to_lowercase().contains(&text))
}

fn row(c: &ToolCall) -> adw::ActionRow {
    let (icon, outcome) = match c.outcome.as_str() {
        "ok" => ("emblem-ok-symbolic", gettext("succeeded")),
        "denied" => ("action-unavailable-symbolic", gettext("denied by you")),
        "limited" => ("action-unavailable-symbolic", gettext("refused: too many calls")),
        _ => ("dialog-warning-symbolic", gettext("failed")),
    };
    let title = match &c.client {
        Some(client) => format!("{}  ·  {client}", c.tool),
        None => c.tool.clone(),
    };
    let subtitle = c
        .error
        .as_ref()
        .map_or_else(|| c.arguments.clone(), |e| format!("{e}\n{}", c.arguments));
    let r = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(glib::markup_escape_text(&subtitle))
        .subtitle_lines(2)
        .tooltip_text(&subtitle)
        .build();
    let image = gtk::Image::from_icon_name(icon);
    image.update_property(&[gtk::accessible::Property::Label(&outcome)]);
    image.set_tooltip_text(Some(&outcome));
    if is_problem(c) {
        image.add_css_class("warning");
    }
    r.add_prefix(&image);
    let time = c
        .time
        .to_zoned(TimeZone::system())
        .strftime("%H:%M:%S")
        .to_string();
    let when = gtk::Label::builder()
        .label(format!("{time}  {} ms", c.ms))
        .css_classes(["dim-label", "numeric"])
        .build();
    r.add_suffix(&when);
    r
}

impl Activity {
    /// Empty; the window is built when first shown.
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            entries: Rc::default(),
            view: RefCell::default(),
        })
    }

    /// Follow `calls` (the embedded server's stream) until the server stops.
    pub fn follow(self: &Rc<Self>, mut calls: tokio::sync::broadcast::Receiver<ToolCall>) {
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            loop {
                let call = match calls.recv().await {
                    Ok(c) => c,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                match this.upgrade() {
                    Some(a) => a.record(call),
                    None => break,
                }
            }
        });
    }

    /// Add a finished call.
    pub fn record(&self, call: ToolCall) {
        let row = self.view.borrow().as_ref().map(|_| row(&call));
        self.entries.borrow_mut().push_front(call);
        let dropped = self.entries.borrow().len() > KEEP;
        if dropped {
            self.entries.borrow_mut().pop_back();
        }
        if let Some(v) = self.view.borrow().as_ref() {
            if let Some(r) = row {
                v.list.prepend(&r);
            }
            if dropped && let Some(last) = v.list.row_at_index(KEEP as i32) {
                v.list.remove(&last);
            }
            self.update_title(v);
        }
    }

    fn update_title(&self, v: &View) {
        let entries = self.entries.borrow();
        let problems = entries.iter().filter(|c| is_problem(c)).count();
        v.title.set_subtitle(&gettext_f(
            "Calls: {calls} · refused or failed: {problems}",
            &[("calls", &entries.len()), ("problems", &problems)],
        ));
    }

    /// Show the window (built on first use).
    pub fn present(self: &Rc<Self>, parent: &impl IsA<gtk::Window>) {
        if self.view.borrow().is_none() {
            let v = self.build(parent);
            *self.view.borrow_mut() = Some(v);
        }
        if let Some(v) = self.view.borrow().as_ref() {
            self.update_title(v);
            v.window.present();
        }
    }

    fn build(self: &Rc<Self>, parent: &impl IsA<gtk::Window>) -> View {
        let title = adw::WindowTitle::new(&gettext("Agent Activity"), "");
        let search = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Filter by tool, arguments or client"))
            .hexpand(true)
            .build();
        let problems = gtk::ToggleButton::builder()
            .icon_name("dialog-warning-symbolic")
            .tooltip_text(gettext("Only refused and failed calls"))
            .build();
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text(gettext("Clear the list"))
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        header.pack_start(&problems);
        header.pack_end(&clear);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .valign(gtk::Align::Start)
            .build();
        list.set_placeholder(Some(
            &gtk::Label::builder()
                .label(gettext(
                    "No tool calls yet. Agents connected to the MCP server appear here.",
                ))
                .wrap(true)
                .margin_top(24)
                .margin_bottom(24)
                .css_classes(["dim-label"])
                .build(),
        ));
        for c in self.entries.borrow().iter() {
            list.append(&row(c));
        }
        let entries = Rc::clone(&self.entries);
        let (s, p) = (search.downgrade(), problems.downgrade());
        list.set_filter_func(move |r| {
            let (Some(s), Some(p)) = (s.upgrade(), p.upgrade()) else {
                return true;
            };
            let entries = entries.borrow();
            usize::try_from(r.index())
                .ok()
                .and_then(|i| entries.get(i))
                .is_none_or(|c| matches(c, &s.text(), p.is_active()))
        });
        let l = list.downgrade();
        search.connect_search_changed(move |_| {
            if let Some(l) = l.upgrade() {
                l.invalidate_filter();
            }
        });
        let l = list.downgrade();
        problems.connect_toggled(move |_| {
            if let Some(l) = l.upgrade() {
                l.invalidate_filter();
            }
        });
        let w = Rc::downgrade(self);
        clear.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.clear();
            }
        });
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(8);
        content.set_margin_bottom(12);
        content.append(&search);
        content.append(&list);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scroller));
        let window = adw::Window::builder()
            .title(gettext("Agent Activity"))
            .transient_for(parent)
            .default_width(640)
            .default_height(520)
            .hide_on_close(true)
            .content(&toolbar)
            .build();
        View {
            window,
            list,
            title,
            search,
            problems,
        }
    }

    /// Forget all calls.
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        if let Some(v) = self.view.borrow().as_ref() {
            v.list.remove_all();
            self.update_title(v);
        }
    }

    /// Close the window (the main window closes).
    pub fn close(&self) {
        if let Some(v) = self.view.borrow_mut().take() {
            v.window.destroy();
        }
    }

    /// Rows shown with the current filter (tests).
    #[cfg(test)]
    pub fn visible_rows(&self) -> usize {
        let view = self.view.borrow();
        let Some(v) = view.as_ref() else { return 0 };
        (0..self.entries.borrow().len())
            .filter(|&i| {
                v.list
                    .row_at_index(i as i32)
                    .is_some_and(|r| r.is_child_visible())
            })
            .count()
    }

    /// Set the filter (tests).
    #[cfg(test)]
    pub fn set_filter(&self, text: &str, problems_only: bool) {
        if let Some(v) = self.view.borrow().as_ref() {
            v.search.set_text(text);
            v.problems.set_active(problems_only);
            v.list.invalidate_filter();
        }
    }

    /// The window, once built (tests).
    #[cfg(test)]
    pub fn window(&self) -> Option<adw::Window> {
        self.view.borrow().as_ref().map(|v| v.window.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str, outcome: &str) -> ToolCall {
        ToolCall {
            time: jiff::Timestamp::now(),
            tool: tool.into(),
            client: Some("claude-code 2.1".into()),
            arguments: r#"{"data":"AT"}"#.into(),
            outcome: outcome.into(),
            error: (outcome != "ok").then(|| "nope".into()),
            ms: 3,
        }
    }

    #[test]
    fn filtering() {
        let c = call("send", "ok");
        assert!(matches(&c, "", false));
        assert!(matches(&c, "SEND", false));
        assert!(matches(&c, "claude", false));
        assert!(matches(&c, "\"AT\"", false));
        assert!(!matches(&c, "", true));
        assert!(!matches(&c, "read", false));
        assert!(matches(&call("send", "limited"), "nope", true));
    }
}
