//! Signal monitor: a timeline of DTR, RTS, CTS, DSR, DCD and RI over the last seconds.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtkterm_core::session::Session;
use gtkterm_core::signals::{NAMES, SignalSample, levels};
use jiff::{SignedDuration, Timestamp};

use super::i18n::{gettext, gettext_f};

const SPANS: [i64; 4] = [10, 30, 60, 300];
const LABEL_W: f64 = 48.0;
const VALUE_W: f64 = 28.0;
const AXIS_H: f64 = 22.0;

/// The signal monitor window.
pub struct SignalMonitor {
    /// The window.
    pub window: adw::Window,
    area: gtk::DrawingArea,
    session: Session,
    span: Cell<i64>,
    frozen: Cell<Option<Timestamp>>,
    toast: adw::ToastOverlay,
}

impl SignalMonitor {
    /// Build the (hidden) window.
    pub fn new(parent: &impl IsA<gtk::Window>, session: Session) -> Rc<Self> {
        let area = gtk::DrawingArea::builder()
            .content_height(6 * 34 + AXIS_H as i32)
            .content_width(640)
            .vexpand(true)
            .hexpand(true)
            .build();
        let labels: Vec<String> = SPANS
            .iter()
            .map(|s| gettext_f("{seconds} s", &[("seconds", s)]))
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let span_drop = gtk::DropDown::builder()
            .model(&gtk::StringList::new(&refs))
            .selected(1)
            .tooltip_text(gettext("Time window"))
            .build();
        let pause = gtk::ToggleButton::builder()
            .icon_name("media-playback-pause-symbolic")
            .tooltip_text(gettext("Freeze the view (recording continues)"))
            .build();
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text(gettext("Clear the history"))
            .build();
        let export = gtk::Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text(gettext("Export CSV"))
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(
            &gettext("Signal Monitor"),
            &gettext("polled every 50 ms; short pulses from the driver's counters"),
        )));
        header.pack_start(&span_drop);
        header.pack_start(&pause);
        header.pack_end(&export);
        header.pack_end(&clear);
        let toast = adw::ToastOverlay::new();
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.set_margin_start(8);
        frame.set_margin_end(8);
        frame.set_margin_top(8);
        frame.set_margin_bottom(8);
        frame.append(&area);
        toast.set_child(Some(&frame));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toast));
        let window = adw::Window::builder()
            .title(gettext("Signal Monitor"))
            .transient_for(parent)
            .default_width(760)
            .default_height(330)
            .hide_on_close(true)
            .content(&toolbar)
            .build();
        let this = Rc::new(Self {
            window,
            area,
            session,
            span: Cell::new(SPANS[1]),
            frozen: Cell::new(None),
            toast,
        });

        let w = Rc::downgrade(&this);
        this.area.set_draw_func(move |area, cr, width, height| {
            if let Some(w) = w.upgrade() {
                w.draw(area, cr, f64::from(width), f64::from(height));
            }
        });
        let w = Rc::downgrade(&this);
        span_drop.connect_selected_notify(move |d| {
            if let Some(w) = w.upgrade() {
                w.span.set(SPANS[(d.selected() as usize).min(SPANS.len() - 1)]);
                w.area.queue_draw();
            }
        });
        let w = Rc::downgrade(&this);
        pause.connect_toggled(move |b| {
            if let Some(w) = w.upgrade() {
                w.frozen.set(b.is_active().then(Timestamp::now));
                w.area.queue_draw();
            }
        });
        let s = this.session.clone();
        let a = this.area.downgrade();
        clear.connect_clicked(move |_| {
            s.clear_signals();
            if let Some(a) = a.upgrade() {
                a.queue_draw();
            }
        });
        let w = Rc::downgrade(&this);
        export.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                let stamp = jiff::Zoned::now().strftime("%Y%m%d_%H%M%S");
                super::charts::save_dialog(
                    &w.window,
                    &format!("signals_{stamp}.csv"),
                    w.session.signals_csv().into_bytes(),
                    w.toast.clone(),
                );
            }
        });
        // Scroll with time while shown.
        let a = this.area.downgrade();
        let win = this.window.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            match (a.upgrade(), win.upgrade()) {
                (Some(a), Some(win)) => {
                    if win.is_visible() {
                        a.queue_draw();
                    }
                    glib::ControlFlow::Continue
                }
                _ => glib::ControlFlow::Break,
            }
        });
        this
    }

    /// Show the window.
    pub fn present(&self) {
        self.window.present();
    }

    /// New samples arrived.
    pub fn update(&self) {
        if self.window.is_visible() {
            self.area.queue_draw();
        }
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let fg = area.color();
        let accent = adw::StyleManager::default().accent_color_rgba();
        let now = self.frozen.get().unwrap_or_else(Timestamp::now);
        let span = self.span.get();
        let start = now - SignedDuration::from_secs(span);
        let samples: Vec<SignalSample> = self.session.signal_history(Some(start));
        let rows = NAMES.len() as f64;
        let row_h = ((height - AXIS_H) / rows).max(12.0);
        let plot_w = (width - LABEL_W - VALUE_W).max(10.0);
        let x_of = |t: Timestamp| {
            let dt = t.duration_since(start).as_secs_f64().clamp(0.0, span as f64);
            LABEL_W + dt / span as f64 * plot_w
        };
        let set = |cr: &gtk::cairo::Context, c: &gtk::gdk::RGBA, alpha: f64| {
            cr.set_source_rgba(
                f64::from(c.red()),
                f64::from(c.green()),
                f64::from(c.blue()),
                alpha,
            );
        };
        cr.select_font_face(
            "monospace",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
        cr.set_font_size(11.0);
        // Grid and time axis.
        set(cr, &fg, 0.12);
        cr.set_line_width(1.0);
        let step = match span {
            10 => 2,
            30 => 5,
            60 => 10,
            _ => 60,
        };
        for k in 0..=span / step {
            let x = LABEL_W + (k * step) as f64 / span as f64 * plot_w;
            cr.move_to(x.round() + 0.5, 0.0);
            cr.line_to(x.round() + 0.5, height - AXIS_H);
            let _ = cr.stroke();
        }
        set(cr, &fg, 0.6);
        for k in 0..=span / step {
            let x = LABEL_W + (k * step) as f64 / span as f64 * plot_w;
            let label = format!("-{}s", span - k * step);
            cr.move_to(x - 12.0, height - 6.0);
            let _ = cr.show_text(&label);
        }
        if samples.is_empty() {
            cr.move_to(LABEL_W + 8.0, (height - AXIS_H) / 2.0);
            let _ = cr.show_text(&gettext("No modem lines (open a serial port with modem lines)"));
            return;
        }
        for (row, name) in NAMES.iter().enumerate() {
            let top = row as f64 * row_h;
            let hi = top + row_h * 0.22;
            let lo = top + row_h * 0.78;
            let output = row < 2; // DTR, RTS are ours
            set(cr, &fg, 0.85);
            cr.move_to(6.0, top + row_h * 0.6);
            let _ = cr.show_text(name);
            let color = if output { &accent } else { &fg };
            set(cr, color, 0.95);
            cr.set_line_width(2.0);
            let level = |s: &SignalSample| levels(&s.lines)[row];
            let mut prev = level(&samples[0]);
            cr.move_to(LABEL_W, if prev { hi } else { lo });
            for s in &samples[1..] {
                let x = x_of(s.time);
                let v = level(s);
                if v != prev {
                    cr.line_to(x, if prev { hi } else { lo });
                    cr.line_to(x, if v { hi } else { lo });
                    prev = v;
                }
            }
            cr.line_to(LABEL_W + plot_w, if prev { hi } else { lo });
            let _ = cr.stroke();
            // Pulses too short to draw get a marker.
            for s in samples.iter().filter(|s| s.pulse && s.changed.contains(name)) {
                let x = x_of(s.time);
                cr.arc(x, top + row_h * 0.5, 3.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
            set(cr, &fg, 0.85);
            cr.move_to(LABEL_W + plot_w + 8.0, top + row_h * 0.6);
            let _ = cr.show_text(if prev { "1" } else { "0" });
        }
    }
}
