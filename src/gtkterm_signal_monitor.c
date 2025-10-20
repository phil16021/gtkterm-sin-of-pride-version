/************************************************************************/
/* gtkterm_signal_monitor.c                                             */
/* ------------------------                                             */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Real-time signal line monitoring implementation                */
/*                                                                      */
/************************************************************************/

#include "gtkterm_signal_monitor.h"
#include <string.h>

#define MAX_HISTORY_EVENTS 10000

struct _GtkTermSignalMonitor {
    GtkBox parent_instance;

    /* Signal indicators */
    GtkWidget *dtr_indicator;
    GtkWidget *rts_indicator;
    GtkWidget *cts_indicator;
    GtkWidget *dsr_indicator;
    GtkWidget *dcd_indicator;
    GtkWidget *ri_indicator;

    /* Signal state labels */
    GtkWidget *dtr_label;
    GtkWidget *rts_label;
    GtkWidget *cts_label;
    GtkWidget *dsr_label;
    GtkWidget *dcd_label;
    GtkWidget *ri_label;

    /* Signal counters */
    GtkWidget *dtr_counter;
    GtkWidget *rts_counter;
    GtkWidget *cts_counter;
    GtkWidget *dsr_counter;
    GtkWidget *dcd_counter;
    GtkWidget *ri_counter;

    /* Current state */
    guint32 current_signals;

    /* Statistics */
    guint dtr_changes;
    guint rts_changes;
    guint cts_changes;
    guint dsr_changes;
    guint dcd_changes;
    guint ri_changes;

    /* History */
    GQueue *history;
    guint history_duration;  /* seconds */

    /* Settings */
    guint update_interval;   /* milliseconds */
};

G_DEFINE_TYPE (GtkTermSignalMonitor, gtkterm_signal_monitor, GTK_TYPE_BOX)


/**
 * @brief Create signal indicator row
 */
static void create_signal_row (GtkTermSignalMonitor *self,
                                 GtkWidget *grid,
                                 int row,
                                 const char *name,
                                 const char *label_text,
                                 GtkWidget **indicator,
                                 GtkWidget **label,
                                 GtkWidget **counter) {
    /* Signal name */
    GtkWidget *name_label = gtk_label_new (name);
    gtk_widget_add_css_class (name_label, "signal-name");
    gtk_widget_set_halign (name_label, GTK_ALIGN_START);
    gtk_widget_set_size_request (name_label, 50, -1);
    gtk_grid_attach (GTK_GRID (grid), name_label, 0, row, 1, 1);

    /* LED indicator */
    GtkWidget *led = gtk_label_new ("●");
    gtk_widget_add_css_class (led, "signal-led");
    gtk_widget_add_css_class (led, "signal-off");
    gtk_widget_set_halign (led, GTK_ALIGN_CENTER);
    gtk_widget_set_size_request (led, 30, -1);
    gtk_grid_attach (GTK_GRID (grid), led, 1, row, 1, 1);
    *indicator = led;

    /* State label */
    GtkWidget *state = gtk_label_new ("LOW");
    gtk_widget_add_css_class (state, "signal-state");
    gtk_widget_set_halign (state, GTK_ALIGN_START);
    gtk_widget_set_size_request (state, 60, -1);
    gtk_grid_attach (GTK_GRID (grid), state, 2, row, 1, 1);
    *label = state;

    /* Description */
    GtkWidget *desc = gtk_label_new (label_text);
    gtk_widget_add_css_class (desc, "signal-description");
    gtk_widget_set_halign (desc, GTK_ALIGN_START);
    gtk_widget_set_hexpand (desc, TRUE);
    gtk_grid_attach (GTK_GRID (grid), desc, 3, row, 1, 1);

    /* Change counter */
    GtkWidget *count = gtk_label_new ("0");
    gtk_widget_add_css_class (count, "signal-counter");
    gtk_widget_set_halign (count, GTK_ALIGN_END);
    gtk_widget_set_size_request (count, 60, -1);
    gtk_grid_attach (GTK_GRID (grid), count, 4, row, 1, 1);
    *counter = count;
}

/**
 * @brief Update signal indicator
 */
static void update_signal_indicator (GtkWidget *indicator,
                                       GtkWidget *label,
                                       GtkWidget *counter,
                                       gboolean active,
                                       guint changes) {
    if (active) {
        gtk_widget_remove_css_class (indicator, "signal-off");
        gtk_widget_add_css_class (indicator, "signal-on");
        gtk_label_set_text (GTK_LABEL (label), "HIGH");
    } else {
        gtk_widget_remove_css_class (indicator, "signal-on");
        gtk_widget_add_css_class (indicator, "signal-off");
        gtk_label_set_text (GTK_LABEL (label), "LOW");
    }

    char count_str[32];
    g_snprintf (count_str, sizeof (count_str), "%u", changes);
    gtk_label_set_text (GTK_LABEL (counter), count_str);
}

/**
 * @brief Add event to history
 */
static void add_to_history (GtkTermSignalMonitor *self,
                              guint32 changed_signals,
                              guint32 current_state) {
    GtkTermSignalEvent *event = g_new0 (GtkTermSignalEvent, 1);
    event->signal_mask = changed_signals;
    event->state = current_state;
    event->timestamp = g_get_real_time ();

    g_queue_push_tail (self->history, event);

    /* Remove old events */
    gint64 cutoff_time = event->timestamp - (self->history_duration * G_USEC_PER_SEC);

    while (g_queue_get_length (self->history) > MAX_HISTORY_EVENTS ||
           (g_queue_peek_head (self->history) &&
            ((GtkTermSignalEvent *) g_queue_peek_head (self->history))->timestamp < cutoff_time)) {
        GtkTermSignalEvent *old_event = g_queue_pop_head (self->history);
        g_free (old_event);
    }
}

/**
 * @brief Update signal states
 */
void gtkterm_signal_monitor_update (GtkTermSignalMonitor *self,
                                     guint32 signals) {
    g_return_if_fail (GTKTERM_IS_SIGNAL_MONITOR (self));

    guint32 changed = self->current_signals ^ signals;

    if (changed) {
        /* Update change counters */
        if (changed & GTKTERM_SIGNAL_DTR) self->dtr_changes++;
        if (changed & GTKTERM_SIGNAL_RTS) self->rts_changes++;
        if (changed & GTKTERM_SIGNAL_CTS) self->cts_changes++;
        if (changed & GTKTERM_SIGNAL_DSR) self->dsr_changes++;
        if (changed & GTKTERM_SIGNAL_DCD) self->dcd_changes++;
        if (changed & GTKTERM_SIGNAL_RI) self->ri_changes++;

        /* Add to history */
        add_to_history (self, changed, signals);
    }

    /* Update indicators */
    update_signal_indicator (self->dtr_indicator, self->dtr_label, self->dtr_counter,
                             signals & GTKTERM_SIGNAL_DTR, self->dtr_changes);
    update_signal_indicator (self->rts_indicator, self->rts_label, self->rts_counter,
                             signals & GTKTERM_SIGNAL_RTS, self->rts_changes);
    update_signal_indicator (self->cts_indicator, self->cts_label, self->cts_counter,
                             signals & GTKTERM_SIGNAL_CTS, self->cts_changes);
    update_signal_indicator (self->dsr_indicator, self->dsr_label, self->dsr_counter,
                             signals & GTKTERM_SIGNAL_DSR, self->dsr_changes);
    update_signal_indicator (self->dcd_indicator, self->dcd_label, self->dcd_counter,
                             signals & GTKTERM_SIGNAL_DCD, self->dcd_changes);
    update_signal_indicator (self->ri_indicator, self->ri_label, self->ri_counter,
                             signals & GTKTERM_SIGNAL_RI, self->ri_changes);

    self->current_signals = signals;
}

/**
 * @brief Clear signal history
 */
void gtkterm_signal_monitor_clear_history (GtkTermSignalMonitor *self) {
    g_return_if_fail (GTKTERM_IS_SIGNAL_MONITOR (self));

    g_queue_clear_full (self->history, g_free);

    self->dtr_changes = 0;
    self->rts_changes = 0;
    self->cts_changes = 0;
    self->dsr_changes = 0;
    self->dcd_changes = 0;
    self->ri_changes = 0;

    /* Update counters */
    gtk_label_set_text (GTK_LABEL (self->dtr_counter), "0");
    gtk_label_set_text (GTK_LABEL (self->rts_counter), "0");
    gtk_label_set_text (GTK_LABEL (self->cts_counter), "0");
    gtk_label_set_text (GTK_LABEL (self->dsr_counter), "0");
    gtk_label_set_text (GTK_LABEL (self->dcd_counter), "0");
    gtk_label_set_text (GTK_LABEL (self->ri_counter), "0");
}

/**
 * @brief Set update interval
 */
void gtkterm_signal_monitor_set_interval (GtkTermSignalMonitor *self,
                                           guint interval_ms) {
    g_return_if_fail (GTKTERM_IS_SIGNAL_MONITOR (self));

    self->update_interval = interval_ms;
}

/**
 * @brief Set history duration
 */
void gtkterm_signal_monitor_set_history_duration (GtkTermSignalMonitor *self,
                                                    guint duration_sec) {
    g_return_if_fail (GTKTERM_IS_SIGNAL_MONITOR (self));

    self->history_duration = duration_sec;
}

/**
 * @brief Get signal statistics
 */
guint gtkterm_signal_monitor_get_change_count (GtkTermSignalMonitor *self,
                                                guint32 signal_mask) {
    g_return_val_if_fail (GTKTERM_IS_SIGNAL_MONITOR (self), 0);

    if (signal_mask & GTKTERM_SIGNAL_DTR) return self->dtr_changes;
    if (signal_mask & GTKTERM_SIGNAL_RTS) return self->rts_changes;
    if (signal_mask & GTKTERM_SIGNAL_CTS) return self->cts_changes;
    if (signal_mask & GTKTERM_SIGNAL_DSR) return self->dsr_changes;
    if (signal_mask & GTKTERM_SIGNAL_DCD) return self->dcd_changes;
    if (signal_mask & GTKTERM_SIGNAL_RI) return self->ri_changes;

    return 0;
}

/**
 * @brief Export signal history to file
 */
gboolean gtkterm_signal_monitor_export (GtkTermSignalMonitor *self,
                                         const char *filename,
                                         GError **error) {
    g_return_val_if_fail (GTKTERM_IS_SIGNAL_MONITOR (self), FALSE);
    g_return_val_if_fail (filename != NULL, FALSE);

    FILE *fp = fopen (filename, "w");
    if (!fp) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_FAILED,
                    "Failed to open file: %s", filename);
        return FALSE;
    }

    /* Write header */
    fprintf (fp, "Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals\n");

    /* Write events */
    GList *l;
    for (l = self->history->head; l != NULL; l = l->next) {
        GtkTermSignalEvent *event = (GtkTermSignalEvent *) l->data;

        /* Format timestamp */
        GDateTime *dt = g_date_time_new_from_unix_local (event->timestamp / G_USEC_PER_SEC);
        char *timestamp = g_date_time_format (dt, "%Y-%m-%d %H:%M:%S");

        /* Signal states */
        int dtr = (event->state & GTKTERM_SIGNAL_DTR) ? 1 : 0;
        int rts = (event->state & GTKTERM_SIGNAL_RTS) ? 1 : 0;
        int cts = (event->state & GTKTERM_SIGNAL_CTS) ? 1 : 0;
        int dsr = (event->state & GTKTERM_SIGNAL_DSR) ? 1 : 0;
        int dcd = (event->state & GTKTERM_SIGNAL_DCD) ? 1 : 0;
        int ri = (event->state & GTKTERM_SIGNAL_RI) ? 1 : 0;

        /* Changed signals */
        GString *changed = g_string_new ("");
        if (event->signal_mask & GTKTERM_SIGNAL_DTR) g_string_append (changed, "DTR ");
        if (event->signal_mask & GTKTERM_SIGNAL_RTS) g_string_append (changed, "RTS ");
        if (event->signal_mask & GTKTERM_SIGNAL_CTS) g_string_append (changed, "CTS ");
        if (event->signal_mask & GTKTERM_SIGNAL_DSR) g_string_append (changed, "DSR ");
        if (event->signal_mask & GTKTERM_SIGNAL_DCD) g_string_append (changed, "DCD ");
        if (event->signal_mask & GTKTERM_SIGNAL_RI) g_string_append (changed, "RI ");

        fprintf (fp, "%s,%d,%d,%d,%d,%d,%d,%s\n",
                 timestamp, dtr, rts, cts, dsr, dcd, ri, changed->str);

        g_string_free (changed, TRUE);
        g_free (timestamp);
        g_date_time_unref (dt);
    }

    fclose (fp);

    return TRUE;
}

/**
 * @brief Dispose
 */
static void gtkterm_signal_monitor_dispose (GObject *object) {
    GtkTermSignalMonitor *self = GTKTERM_SIGNAL_MONITOR (object);

    if (self->history) {
        g_queue_free_full (self->history, g_free);
        self->history = NULL;
    }

    G_OBJECT_CLASS (gtkterm_signal_monitor_parent_class)->dispose (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_signal_monitor_class_init (GtkTermSignalMonitorClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_signal_monitor_dispose;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_signal_monitor_init (GtkTermSignalMonitor *self) {
    gtk_orientable_set_orientation (GTK_ORIENTABLE (self), GTK_ORIENTATION_VERTICAL);
    gtk_widget_add_css_class (GTK_WIDGET (self), "signal-monitor");

    /* Title */
    GtkWidget *title = gtk_label_new ("Signal Line Monitor");
    gtk_widget_add_css_class (title, "title");
    gtk_widget_set_margin_top (title, 12);
    gtk_widget_set_margin_bottom (title, 12);
    gtk_box_append (GTK_BOX (self), title);

    /* Signal grid */
    GtkWidget *grid = gtk_grid_new ();
    gtk_widget_add_css_class (grid, "signal-grid");
    gtk_grid_set_row_spacing (GTK_GRID (grid), 8);
    gtk_grid_set_column_spacing (GTK_GRID (grid), 12);
    gtk_widget_set_margin_start (grid, 12);
    gtk_widget_set_margin_end (grid, 12);
    gtk_widget_set_margin_top (grid, 12);
    gtk_widget_set_margin_bottom (grid, 12);

    /* Headers */
    GtkWidget *header1 = gtk_label_new ("Signal");
    gtk_widget_add_css_class (header1, "header");
    gtk_grid_attach (GTK_GRID (grid), header1, 0, 0, 1, 1);

    GtkWidget *header2 = gtk_label_new ("LED");
    gtk_widget_add_css_class (header2, "header");
    gtk_grid_attach (GTK_GRID (grid), header2, 1, 0, 1, 1);

    GtkWidget *header3 = gtk_label_new ("State");
    gtk_widget_add_css_class (header3, "header");
    gtk_grid_attach (GTK_GRID (grid), header3, 2, 0, 1, 1);

    GtkWidget *header4 = gtk_label_new ("Description");
    gtk_widget_add_css_class (header4, "header");
    gtk_grid_attach (GTK_GRID (grid), header4, 3, 0, 1, 1);

    GtkWidget *header5 = gtk_label_new ("Changes");
    gtk_widget_add_css_class (header5, "header");
    gtk_grid_attach (GTK_GRID (grid), header5, 4, 0, 1, 1);

    /* Create signal rows */
    create_signal_row (self, grid, 1, "DTR", "Data Terminal Ready",
                       &self->dtr_indicator, &self->dtr_label, &self->dtr_counter);
    create_signal_row (self, grid, 2, "RTS", "Request To Send",
                       &self->rts_indicator, &self->rts_label, &self->rts_counter);
    create_signal_row (self, grid, 3, "CTS", "Clear To Send",
                       &self->cts_indicator, &self->cts_label, &self->cts_counter);
    create_signal_row (self, grid, 4, "DSR", "Data Set Ready",
                       &self->dsr_indicator, &self->dsr_label, &self->dsr_counter);
    create_signal_row (self, grid, 5, "DCD", "Data Carrier Detect",
                       &self->dcd_indicator, &self->dcd_label, &self->dcd_counter);
    create_signal_row (self, grid, 6, "RI", "Ring Indicator",
                       &self->ri_indicator, &self->ri_label, &self->ri_counter);

    gtk_box_append (GTK_BOX (self), grid);

    /* Initialize state */
    self->current_signals = 0;
    self->dtr_changes = 0;
    self->rts_changes = 0;
    self->cts_changes = 0;
    self->dsr_changes = 0;
    self->dcd_changes = 0;
    self->ri_changes = 0;

    self->history = g_queue_new ();
    self->history_duration = 300;  /* 5 minutes */
    self->update_interval = 100;   /* 100ms */
}

/**
 * @brief Create a new signal monitor
 */
GtkWidget *gtkterm_signal_monitor_new (void) {
    return g_object_new (GTKTERM_TYPE_SIGNAL_MONITOR, NULL);
}
