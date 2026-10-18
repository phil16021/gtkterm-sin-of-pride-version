//! Sunshine visualization panel: one tab per packet ID.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtkterm_charts::model::{Hit, ViewState, XAxis};
use gtkterm_charts::render::{Layout, Style, draw, render_png};
use gtkterm_core::session::Session;
use gtkterm_core::sunshine::SunshineUpdate;
use gtkterm_protocols::sunshine::ChartKind;

use super::i18n::{gettext, gettext_f};

fn style_for(widget: &impl IsA<gtk::Widget>) -> Style {
    let dark = adw::StyleManager::default().is_dark();
    let mut s = if dark { Style::dark() } else { Style::light() };
    let fg = widget.color();
    s.fg = (f64::from(fg.red()), f64::from(fg.green()), f64::from(fg.blue()));
    s
}

/// One chart tab.
pub struct ChartView {
    /// Tab content.
    pub widget: gtk::Box,
    area: gtk::DrawingArea,
    id: String,
    session: Session,
    view: RefCell<ViewState>,
    layout: RefCell<Option<Layout>>,
    hover: RefCell<Option<Hit>>,
    drag_origin: Cell<Option<f64>>,
    pause: gtk::ToggleButton,
    kind: gtk::DropDown,
}

impl ChartView {
    fn new(session: &Session, id: &str, toast: adw::ToastOverlay) -> Rc<Self> {
        let area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .content_width(360)
            .content_height(240)
            .build();
        let pause = gtk::ToggleButton::builder()
            .icon_name("media-playback-pause-symbolic")
            .tooltip_text(gettext("Pause"))
            .build();
        let axis = gtk::ToggleButton::builder()
            .icon_name("preferences-system-time-symbolic")
            .tooltip_text(gettext("Time axis instead of sample number"))
            .build();
        let kind_labels = ChartKind::ALL.map(|k| gettext(k.label()));
        let kinds = gtk::StringList::new(&kind_labels.each_ref().map(String::as_str));
        let kind = gtk::DropDown::builder()
            .model(&kinds)
            .tooltip_text(gettext("Chart type"))
            .build();
        let history = gtk::SpinButton::with_range(10.0, 1_000_000.0, 100.0);
        history.set_tooltip_text(Some(&gettext("Samples kept per series")));
        let csv = gtk::Button::builder()
            .icon_name("x-office-spreadsheet-symbolic")
            .tooltip_text(gettext("Export CSV"))
            .build();
        let png = gtk::Button::builder()
            .icon_name("image-x-generic-symbolic")
            .tooltip_text(gettext("Export PNG"))
            .build();
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text(gettext("Clear data"))
            .build();
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("chart-toolbar");
        for w in [
            pause.upcast_ref::<gtk::Widget>(),
            axis.upcast_ref(),
            kind.upcast_ref(),
            history.upcast_ref(),
        ] {
            toolbar.append(w);
        }
        let spacer = gtk::Box::builder().hexpand(true).build();
        toolbar.append(&spacer);
        for w in [&csv, &png, &clear] {
            w.add_css_class("flat");
            toolbar.append(w);
        }
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&area);
        widget.append(&toolbar);

        let me = Rc::new(Self {
            widget,
            area,
            id: id.to_owned(),
            session: session.clone(),
            view: RefCell::new(ViewState::default()),
            layout: RefCell::new(None),
            hover: RefCell::new(None),
            drag_origin: Cell::new(None),
            pause: pause.clone(),
            kind: kind.clone(),
        });
        me.sync_toolbar(&history);

        let w = Rc::downgrade(&me);
        me.area.set_draw_func(move |area, cr, width, height| {
            let Some(me) = w.upgrade() else { return };
            let Some(chart) = me.session.with_sunshine(|s| s.chart(&me.id).cloned()) else {
                return;
            };
            let mut view = me.view.borrow_mut();
            view.max_points = usize::try_from(width).unwrap_or(400).max(100) * 2;
            let hover = me.hover.borrow();
            match draw(
                cr,
                &chart,
                &view,
                &style_for(area),
                f64::from(width),
                f64::from(height),
                hover.as_ref(),
            ) {
                Ok(l) => *me.layout.borrow_mut() = Some(l),
                Err(e) => tracing::warn!(error = %e, "chart drawing failed"),
            }
        });

        // Interaction: hover, legend click, double click, scroll zoom, drag pan.
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(&me);
        motion.connect_motion(move |_, x, y| {
            if let Some(me) = w.upgrade() {
                let hit = me.layout.borrow().as_ref().and_then(|l| l.hit(x, y, 12.0));
                if hit != *me.hover.borrow() {
                    *me.hover.borrow_mut() = hit;
                    me.area.queue_draw();
                }
            }
        });
        let w = Rc::downgrade(&me);
        motion.connect_leave(move |_| {
            if let Some(me) = w.upgrade() {
                me.hover.replace(None);
                me.area.queue_draw();
            }
        });
        me.area.add_controller(motion);

        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(&me);
        click.connect_pressed(move |_, n, x, y| {
            let Some(me) = w.upgrade() else { return };
            let legend = me.layout.borrow().as_ref().and_then(|l| l.legend_at(x, y));
            if let Some(index) = legend {
                me.session.with_sunshine_mut(|s| {
                    if let Some(c) = s.chart_mut(&me.id)
                        && let Some(series) = c.series.get_mut(index)
                    {
                        series.visible = !series.visible;
                    }
                });
            } else if n == 2 {
                me.view.borrow_mut().reset();
            }
            me.area.queue_draw();
        });
        me.area.add_controller(click);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let w = Rc::downgrade(&me);
        let pointer = Rc::new(Cell::new(0.0));
        let p = Rc::clone(&pointer);
        let track = gtk::EventControllerMotion::new();
        track.connect_motion(move |_, x, _| p.set(x));
        me.area.add_controller(track);
        scroll.connect_scroll(move |_, _, dy| {
            let Some(me) = w.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let layout = me.layout.borrow();
            let Some((map, full)) = layout.as_ref().and_then(|l| {
                let m = l.mapping?;
                match &l.model {
                    gtkterm_charts::model::Model::Line(x) | gtkterm_charts::model::Model::Scatter(x) => {
                        Some((m, x.x_full))
                    }
                    _ => None,
                }
            }) else {
                return glib::Propagation::Proceed;
            };
            drop(layout);
            let center = map.to_data(pointer.get(), 0.0).0;
            me.view
                .borrow_mut()
                .zoom(full, center, if dy < 0.0 { 0.8 } else { 1.25 });
            me.area.queue_draw();
            glib::Propagation::Stop
        });
        me.area.add_controller(scroll);

        let drag = gtk::GestureDrag::new();
        let w = Rc::downgrade(&me);
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(me) = w.upgrade() {
                me.drag_origin.set(Some(0.0));
            }
        });
        let w = Rc::downgrade(&me);
        drag.connect_drag_update(move |_, dx, _| {
            let Some(me) = w.upgrade() else { return };
            let Some(last) = me.drag_origin.get() else { return };
            let layout = me.layout.borrow();
            let Some((map, full)) = layout.as_ref().and_then(|l| {
                let m = l.mapping?;
                match &l.model {
                    gtkterm_charts::model::Model::Line(x) | gtkterm_charts::model::Model::Scatter(x) => {
                        Some((m, x.x_full))
                    }
                    _ => None,
                }
            }) else {
                return;
            };
            drop(layout);
            let per_px = map.x.span() / map.plot.w.max(1.0);
            me.view.borrow_mut().pan(full, -(dx - last) * per_px);
            me.drag_origin.set(Some(dx));
            me.area.queue_draw();
        });
        let w = Rc::downgrade(&me);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(me) = w.upgrade() {
                me.drag_origin.set(None);
            }
        });
        me.area.add_controller(drag);

        // Toolbar.
        let w = Rc::downgrade(&me);
        pause.connect_toggled(move |b| {
            if let Some(me) = w.upgrade() {
                me.session.with_sunshine_mut(|s| {
                    if let Some(c) = s.chart_mut(&me.id) {
                        c.paused = b.is_active();
                    }
                });
                me.area.queue_draw();
            }
        });
        let w = Rc::downgrade(&me);
        axis.connect_toggled(move |b| {
            if let Some(me) = w.upgrade() {
                let mut v = me.view.borrow_mut();
                v.x_axis = if b.is_active() { XAxis::Time } else { XAxis::Index };
                v.reset();
                drop(v);
                me.area.queue_draw();
            }
        });
        let w = Rc::downgrade(&me);
        kind.connect_selected_notify(move |d| {
            let Some(me) = w.upgrade() else { return };
            let Some(k) = ChartKind::ALL.get(d.selected() as usize).copied() else {
                return;
            };
            let current = me.session.with_sunshine(|s| s.chart(&me.id).map(|c| c.kind));
            if current != Some(k) {
                me.session.with_sunshine_mut(|s| s.set_kind(&me.id, k));
                me.area.queue_draw();
            }
        });
        let w = Rc::downgrade(&me);
        history.connect_value_changed(move |sp| {
            if let Some(me) = w.upgrade() {
                me.session.with_sunshine_mut(|s| {
                    if let Some(c) = s.chart_mut(&me.id) {
                        c.set_capacity(sp.value() as usize);
                    }
                });
                me.area.queue_draw();
            }
        });
        let w = Rc::downgrade(&me);
        clear.connect_clicked(move |_| {
            if let Some(me) = w.upgrade() {
                me.session
                    .with_sunshine_mut(|s| s.chart_mut(&me.id).map(gtkterm_core::sunshine::ChartData::clear));
                me.view.borrow_mut().reset();
                me.area.queue_draw();
            }
        });
        // The toast overlay contains this tab, so closures inside the tab may only hold it weakly;
        // a strong reference would keep the whole window content alive after closing.
        let toast = toast.downgrade();
        let t = toast.clone();
        let w = Rc::downgrade(&me);
        csv.connect_clicked(move |b| {
            let Some(me) = w.upgrade() else { return };
            let Some(text) = me
                .session
                .with_sunshine(|s| s.chart(&me.id).map(gtkterm_core::sunshine::ChartData::to_csv))
            else {
                return;
            };
            if let Some(t) = t.upgrade() {
                save_dialog(b, &format!("{}.csv", me.id), text.into_bytes(), t);
            }
        });
        let w = Rc::downgrade(&me);
        png.connect_clicked(move |b| {
            let Some(me) = w.upgrade() else { return };
            let Some(chart) = me.session.with_sunshine(|s| s.chart(&me.id).cloned()) else {
                return;
            };
            let width = me.area.width().max(640);
            let height = me.area.height().max(400);
            let Some(toast) = toast.upgrade() else { return };
            match render_png(&chart, &me.view.borrow(), &style_for(&me.area), width, height) {
                Ok(bytes) => save_dialog(b, &format!("{}.png", me.id), bytes, toast),
                Err(e) => toast.add_toast(adw::Toast::new(&gettext_f(
                    "PNG export failed: {error}",
                    &[("error", &e)],
                ))),
            }
        });
        me
    }

    fn sync_toolbar(&self, history: &gtk::SpinButton) {
        if let Some((kind, paused, cap)) = self
            .session
            .with_sunshine(|s| s.chart(&self.id).map(|c| (c.kind, c.paused, c.capacity)))
        {
            let idx = ChartKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
            self.kind.set_selected(idx as u32);
            self.pause.set_active(paused);
            history.set_value(cap as f64);
        }
    }

    fn refresh(&self) {
        if let Some(kind) = self.session.with_sunshine(|s| s.chart(&self.id).map(|c| c.kind)) {
            let idx = ChartKind::ALL.iter().position(|k| *k == kind).unwrap_or(0) as u32;
            if self.kind.selected() != idx {
                self.kind.set_selected(idx);
            }
        }
        if self.area.is_mapped() {
            self.area.queue_draw();
        }
    }
}

/// Save `bytes` through a file dialog.
pub fn save_dialog(parent: &impl IsA<gtk::Widget>, name: &str, bytes: Vec<u8>, toast: adw::ToastOverlay) {
    let dialog = gtk::FileDialog::builder()
        .title(gettext("Export"))
        .initial_name(name)
        .modal(true)
        .build();
    let window = parent.root().and_downcast::<gtk::Window>();
    dialog.save(window.as_ref(), gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        match std::fs::write(&path, &bytes) {
            Ok(()) => toast.add_toast(adw::Toast::new(&gettext_f(
                "Saved {path}",
                &[("path", &path.display())],
            ))),
            Err(e) => toast.add_toast(adw::Toast::new(&gettext_f(
                "Cannot save {path}: {error}",
                &[("path", &path.display()), ("error", &e)],
            ))),
        }
    });
}

/// The panel with all chart tabs.
pub struct VizPanel {
    /// Top-level widget.
    pub widget: gtk::Stack,
    tab_view: adw::TabView,
    tabs: RefCell<HashMap<String, (adw::TabPage, Rc<ChartView>)>>,
    session: Session,
    toast: glib::WeakRef<adw::ToastOverlay>,
    footer: gtk::Label,
}

impl VizPanel {
    /// Build an empty panel.
    pub fn new(session: &Session, toast: &adw::ToastOverlay) -> Rc<Self> {
        let empty = adw::StatusPage::builder()
            .icon_name("utilities-system-monitor-symbolic")
            .title(gettext("No Sunshine data yet"))
            .description(gettext("Lines like <tt>{temp|Inside|21.5}</tt> are plotted here.\nPrefixes B:, P: and S: select bar, pie and scatter charts."))
            .build();
        if let Some(label) = empty.first_child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            label.set_use_markup(true);
        }
        let tab_view = adw::TabView::new();
        let tab_bar = adw::TabBar::builder()
            .view(&tab_view)
            .autohide(false)
            .expand_tabs(false)
            .build();
        let footer = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .margin_start(8)
            .margin_end(8)
            .margin_bottom(2)
            .build();
        let tabs_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tabs_box.append(&tab_bar);
        tabs_box.append(&tab_view);
        tab_view.set_vexpand(true);
        tabs_box.append(&footer);
        let widget = gtk::Stack::new();
        widget.add_named(&empty, Some("empty"));
        widget.add_named(&tabs_box, Some("tabs"));
        widget.set_size_request(440, -1);

        let me = Rc::new(Self {
            widget,
            tab_view,
            tabs: RefCell::default(),
            session: session.clone(),
            toast: toast.downgrade(),
            footer,
        });
        let w = Rc::downgrade(&me);
        me.tab_view.connect_close_page(move |tv, page| {
            if let Some(me) = w.upgrade() {
                let id = me
                    .tabs
                    .borrow()
                    .iter()
                    .find(|(_, (p, _))| p == page)
                    .map(|(id, _)| id.clone());
                if let Some(id) = id {
                    me.tabs.borrow_mut().remove(&id);
                    me.session.with_sunshine_mut(|s| s.remove(&id));
                }
                if me.tabs.borrow().is_empty() {
                    me.widget.set_visible_child_name("empty");
                }
            }
            tv.close_page_finish(page, true);
            glib::Propagation::Stop
        });
        me.sync();
        me
    }

    fn add_tab(self: &Rc<Self>, id: &str) {
        if self.tabs.borrow().contains_key(id) {
            return;
        }
        self.add_tab_now(id);
    }

    fn add_tab_now(&self, id: &str) {
        if self.tabs.borrow().contains_key(id) {
            return;
        }
        // Note: libadwaita 1.9 logs "AdwGizmo (tabboxchild) reported min width -6" while the
        // tab-open animation starts; it is cosmetic and disappears with animations disabled.
        let Some(toast) = self.toast.upgrade() else { return };
        let view = ChartView::new(&self.session, id, toast);
        let page = self.tab_view.append(&view.widget);
        page.set_title(id);
        page.set_tooltip(&gettext_f("Packet ID {id}", &[("id", &id)]));
        self.tabs.borrow_mut().insert(id.to_owned(), (page, view));
        self.widget.set_visible_child_name("tabs");
    }

    /// Create tabs for charts that already exist (e.g. after reopening the panel).
    pub fn sync(self: &Rc<Self>) {
        let ids: Vec<String> = self
            .session
            .with_sunshine(|s| s.charts().iter().map(|c| c.id.clone()).collect());
        for id in &ids {
            self.add_tab(id);
        }
        let stale: Vec<String> = self
            .tabs
            .borrow()
            .keys()
            .filter(|k| !ids.contains(k))
            .cloned()
            .collect();
        for id in stale {
            let page = self.tabs.borrow_mut().remove(&id).map(|(p, _)| p);
            if let Some(page) = page {
                self.tab_view.close_page(&page);
            }
        }
        if self.tabs.borrow().is_empty() {
            self.widget.set_visible_child_name("empty");
        }
        self.update_footer();
    }

    /// Refresh the packet counters below the tabs.
    pub fn update_footer(&self) {
        let (stats, last) = self
            .session
            .with_sunshine(|s| (s.stats(), s.last_error().map(str::to_owned)));
        let mut text = gettext_f(
            "{packets} packets · {rejected} rejected",
            &[("packets", &stats.packets), ("rejected", &stats.errors)],
        );
        if let Some(e) = last {
            let error: String = e.chars().take(80).collect();
            text.push_str(&gettext_f(" · last: {error}", &[("error", &error)]));
        }
        self.footer.set_text(&text);
    }

    /// React to a Sunshine update. Returns true if a chart was added.
    pub fn on_update(self: &Rc<Self>, update: &SunshineUpdate) -> bool {
        match update {
            SunshineUpdate::ChartAdded(id) => {
                self.add_tab(id);
                self.update_footer();
                true
            }
            SunshineUpdate::ChartUpdated(id) => {
                if let Some((_, view)) = self.tabs.borrow().get(id) {
                    view.refresh();
                }
                false
            }
            SunshineUpdate::Rejected { .. } => {
                self.update_footer();
                false
            }
        }
    }

    /// Redraw everything (theme change).
    pub fn redraw_all(&self) {
        for (_, view) in self.tabs.borrow().values() {
            view.area.queue_draw();
        }
    }

    /// Show the `index`th chart tab.
    #[cfg(test)]
    pub fn select(&self, index: i32) {
        if index < self.tab_view.n_pages() {
            self.tab_view.set_selected_page(&self.tab_view.nth_page(index));
        }
    }

    /// Number of chart tabs.
    pub fn len(&self) -> usize {
        self.tabs.borrow().len()
    }
}
