//! Decoded frames pane: a column view of the session's frame ring with decoder controls.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtkterm_core::decoding::{DecodedFrame, DecoderMode, DecoderStatus, FRAME_CAPACITY, FrameSource};
use gtkterm_core::session::Session;
use gtkterm_protocols::decode::Check;
use jiff::tz::TimeZone;

use super::i18n::{gettext, gettext_f};

/// The frames pane.
pub struct FramesView {
    /// Root widget.
    pub widget: gtk::Box,
    session: Session,
    store: gio::ListStore,
    filter: gtk::CustomFilter,
    query: Rc<RefCell<String>>,
    view: gtk::ColumnView,
    selection: gtk::SingleSelection,
    mode_drop: gtk::DropDown,
    signals: gtk::ToggleButton,
    follow: gtk::ToggleButton,
    count: gtk::Label,
    details: gtk::Label,
    modes: Vec<DecoderMode>,
    updating: Cell<bool>,
}

fn frame_of(obj: &glib::Object) -> std::cell::Ref<'_, DecodedFrame> {
    obj.downcast_ref::<glib::BoxedAnyObject>()
        .expect("frames store holds BoxedAnyObject")
        .borrow::<DecodedFrame>()
}

fn source_label(s: FrameSource) -> &'static str {
    match s {
        FrameSource::Rx => "RX",
        FrameSource::Tx => "TX",
        FrameSource::Lines => "LINES",
    }
}

/// Column text and optional CSS class for a frame.
type CellFn = fn(&DecodedFrame) -> (String, Option<&'static str>);

fn column(title: &str, cell: CellFn, expand: bool) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk::Label::builder().xalign(0.0).build();
        label.add_css_class("monospace");
        if expand {
            // Long summaries are cut at the pane edge; the tooltip has the whole line.
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        }
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&label));
        }
    });
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let (Some(obj), Some(label)) = (item.item(), item.child().and_downcast::<gtk::Label>()) else {
            return;
        };
        let (text, class) = cell(&frame_of(&obj));
        label.set_text(&text);
        if expand {
            label.set_tooltip_text(Some(&text));
        }
        for c in ["success", "error", "accent", "dim-label"] {
            label.remove_css_class(c);
        }
        if let Some(c) = class {
            label.add_css_class(c);
        }
    });
    let col = gtk::ColumnViewColumn::builder()
        .title(title)
        .factory(&factory)
        .resizable(true)
        .build();
    col.set_expand(expand);
    col
}

impl FramesView {
    /// Build the pane for `session`, showing the frames it already has.
    pub fn new(session: &Session) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let query: Rc<RefCell<String>> = Rc::default();
        let q = Rc::clone(&query);
        let filter = gtk::CustomFilter::new(move |obj| {
            let q = q.borrow();
            q.is_empty() || frame_of(obj).frame.summary.to_lowercase().contains(q.as_str())
        });
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let selection = gtk::SingleSelection::builder()
            .model(&filtered)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let view = gtk::ColumnView::builder()
            .model(&selection)
            .show_column_separators(true)
            .reorderable(false)
            .build();
        view.add_css_class("data-table");
        view.append_column(&column(
            &gettext("Time"),
            |f| {
                let t = f
                    .time
                    .to_zoned(TimeZone::system())
                    .strftime("%H:%M:%S%.3f")
                    .to_string();
                (t, Some("dim-label"))
            },
            false,
        ));
        view.append_column(&column(
            &gettext("Dir"),
            |f| {
                let class = match f.source {
                    FrameSource::Rx => None,
                    FrameSource::Tx => Some("accent"),
                    FrameSource::Lines => Some("dim-label"),
                };
                (source_label(f.source).to_owned(), class)
            },
            false,
        ));
        view.append_column(&column(
            &gettext("Check"),
            |f| match f.frame.check {
                Check::None => (String::new(), None),
                Check::Ok => ("✓".into(), Some("success")),
                Check::Failed { .. } => ("✗".into(), Some("error")),
            },
            false,
        ));
        view.append_column(&column(
            &gettext("Frame"),
            |f| (f.frame.summary.clone(), None),
            true,
        ));

        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .min_content_height(80)
            .build();
        let details = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(4)
            .label(gettext("Select a frame to see its fields and bytes."))
            .build();
        details.add_css_class("monospace");
        details.add_css_class("dim-label");
        let details_scroller = gtk::ScrolledWindow::builder()
            .child(&details)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(56)
            .max_content_height(140)
            .propagate_natural_height(true)
            .build();

        let modes: Vec<DecoderMode> = DecoderMode::all().collect();
        let labels: Vec<String> = modes.iter().map(|m| gettext(m.label())).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mode_drop = gtk::DropDown::builder()
            .model(&gtk::StringList::new(&labels))
            .tooltip_text(gettext("Protocol decoder"))
            .build();
        let signals = gtk::ToggleButton::builder()
            .icon_name("network-wired-symbolic")
            .tooltip_text(gettext("Record modem line changes"))
            .build();
        let follow = gtk::ToggleButton::builder()
            .icon_name("go-bottom-symbolic")
            .tooltip_text(gettext("Follow new frames"))
            .active(true)
            .build();
        let search = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Filter"))
            .hexpand(true)
            .build();
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text(gettext("Clear frames"))
            .build();
        let count = gtk::Label::new(None);
        count.add_css_class("dim-label");
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.add_css_class("chart-toolbar");
        toolbar.append(&gtk::Label::new(Some(&gettext("Decoder"))));
        toolbar.append(&mode_drop);
        toolbar.append(&signals);
        toolbar.append(&search);
        toolbar.append(&count);
        toolbar.append(&follow);
        toolbar.append(&clear);

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("frames-pane");
        widget.append(&toolbar);
        widget.append(&scroller);
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&details_scroller);

        let this = Rc::new(Self {
            widget,
            session: session.clone(),
            store,
            filter,
            query,
            view,
            selection,
            mode_drop,
            signals,
            follow,
            count,
            details,
            modes,
            updating: Cell::new(false),
        });

        let w = Rc::downgrade(&this);
        this.mode_drop.connect_selected_notify(move |d| {
            let Some(w) = w.upgrade() else { return };
            if w.updating.get() {
                return;
            }
            if let Some(mode) = w.modes.get(d.selected() as usize) {
                w.session.set_decoder(*mode, w.signals.is_active());
            }
        });
        let w = Rc::downgrade(&this);
        this.signals.connect_toggled(move |b| {
            let Some(w) = w.upgrade() else { return };
            if !w.updating.get() {
                w.session
                    .set_decoder(w.session.decoder_status().mode, b.is_active());
            }
        });
        let w = Rc::downgrade(&this);
        search.connect_search_changed(move |e| {
            let Some(w) = w.upgrade() else { return };
            *w.query.borrow_mut() = e.text().to_lowercase();
            w.filter.changed(gtk::FilterChange::Different);
            w.update_count();
        });
        let s = session.clone();
        clear.connect_clicked(move |_| s.clear_frames());
        let w = Rc::downgrade(&this);
        this.selection.connect_selected_item_notify(move |sel| {
            if let Some(w) = w.upgrade() {
                w.show_details(sel.selected_item().as_ref());
            }
        });

        this.set_status(&session.decoder_status());
        this.append(&session.last_frames(FRAME_CAPACITY));
        this
    }

    fn show_details(&self, item: Option<&glib::Object>) {
        let Some(obj) = item else {
            self.details
                .set_text(&gettext("Select a frame to see its fields and bytes."));
            return;
        };
        let f = frame_of(obj);
        let mut text = format!(
            "#{} {} {} {}\n",
            f.seq,
            source_label(f.source),
            f.frame.protocol.label(),
            f.time
        );
        for field in &f.frame.fields {
            let _ = writeln!(text, "{} = {}", field.name, field.value);
        }
        if let Check::Failed { received, computed } = &f.frame.check {
            let line = gettext_f(
                "check failed: received {received}, computed {computed}",
                &[("received", received), ("computed", computed)],
            );
            let _ = writeln!(text, "{line}");
        }
        if !f.frame.bytes.is_empty() {
            let line = gettext_f(
                "bytes ({count}): {hex}",
                &[
                    ("count", &f.frame.bytes.len()),
                    ("hex", &gtkterm_protocols::hex::to_hex(&f.frame.bytes)),
                ],
            );
            text.push_str(&line);
        }
        self.details.set_text(text.trim_end());
    }

    /// Add newly decoded frames.
    pub fn append(&self, frames: &[DecodedFrame]) {
        if frames.is_empty() {
            return;
        }
        let objs: Vec<glib::BoxedAnyObject> = frames.iter().cloned().map(glib::BoxedAnyObject::new).collect();
        self.store.extend_from_slice(&objs);
        let excess = self.store.n_items().saturating_sub(FRAME_CAPACITY as u32);
        if excess > 0 {
            self.store.splice(0, excess, &[] as &[glib::Object]);
        }
        self.update_count();
        if self.follow.is_active() {
            let n = self.selection.n_items();
            if n > 0 {
                self.view.scroll_to(
                    n - 1,
                    None::<&gtk::ColumnViewColumn>,
                    gtk::ListScrollFlags::NONE,
                    None,
                );
            }
        }
    }

    /// Reflect decoder settings changed elsewhere (menu, MCP, profile load).
    pub fn set_status(&self, st: &DecoderStatus) {
        self.updating.set(true);
        if let Some(i) = self.modes.iter().position(|m| *m == st.mode) {
            self.mode_drop.set_selected(u32::try_from(i).unwrap_or(0));
        }
        self.signals.set_active(st.signals);
        self.updating.set(false);
        if st.frames == 0 && self.store.n_items() > 0 {
            self.store.remove_all();
        }
        let label = match (st.mode, st.protocol) {
            (DecoderMode::Auto, Some(p)) => {
                gettext_f("Auto-detect ({protocol})", &[("protocol", &gettext(p.label()))])
            }
            (DecoderMode::Auto, None) => gettext("Auto-detect (waiting for data)"),
            (m, _) => gettext(m.label()),
        };
        self.mode_drop.set_tooltip_text(Some(&gettext_f(
            "Protocol decoder: {label}",
            &[("label", &label)],
        )));
        self.update_count();
    }

    fn update_count(&self) {
        let total = self.store.n_items();
        let shown = self.selection.n_items();
        let failures = self.session.decoder_status().check_failures;
        let mut text = if shown == total {
            gettext_f("{total} frames", &[("total", &total)])
        } else {
            gettext_f(
                "{shown} of {total} frames",
                &[("shown", &shown), ("total", &total)],
            )
        };
        if failures > 0 {
            text.push_str(&gettext_f(", {failures} bad", &[("failures", &failures)]));
        }
        self.count.set_text(&text);
    }

    /// Number of frames shown (after filtering).
    #[cfg(test)]
    pub fn len(&self) -> u32 {
        self.selection.n_items()
    }

    /// Summaries of all frames, oldest first.
    #[cfg(test)]
    pub fn summaries(&self) -> Vec<String> {
        (0..self.store.n_items())
            .filter_map(|i| self.store.item(i))
            .map(|o| frame_of(&o).frame.summary.clone())
            .collect()
    }
}
