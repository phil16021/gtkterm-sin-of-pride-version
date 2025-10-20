/************************************************************************/
/* gtkterm_statusbar.c                                                  */
/* -------------------                                                  */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Enhanced status bar implementation                             */
/*                                                                      */
/************************************************************************/

#include "gtkterm_statusbar.h"
#include <string.h>

struct _GtkTermStatusbar {
    GtkBox parent_instance;

    /* Status widgets */
    GtkWidget *status_icon;
    GtkWidget *status_label;
    GtkWidget *port_label;
    GtkWidget *stats_label;
    GtkWidget *protocol_label;
    GtkWidget *recording_icon;

    /* Signal indicators */
    GtkWidget *dtr_indicator;
    GtkWidget *rts_indicator;
    GtkWidget *cts_indicator;
    GtkWidget *dsr_indicator;
    GtkWidget *dcd_indicator;

    /* Temporary message */
    GtkWidget *message_label;
    guint message_timeout_id;

    /* Current state */
    GtkTermStatusState current_state;
    guint64 total_rx;
    guint64 total_tx;
};

G_DEFINE_TYPE (GtkTermStatusbar, gtkterm_statusbar, GTK_TYPE_BOX)

/**
 * @brief Create signal indicator widget
 */
static GtkWidget *create_signal_indicator (const char *name) {
    GtkWidget *box = gtk_box_new (GTK_ORIENTATION_HORIZONTAL, 2);
    gtk_widget_add_css_class (box, "signal-indicator");

    GtkWidget *label = gtk_label_new (name);
    gtk_widget_add_css_class (label, "signal-label");

    GtkWidget *led = gtk_label_new ("●");
    gtk_widget_add_css_class (led, "signal-led");
    gtk_widget_add_css_class (led, "signal-off");

    gtk_box_append (GTK_BOX (box), label);
    gtk_box_append (GTK_BOX (box), led);

    g_object_set_data (G_OBJECT (box), "led", led);

    return box;
}

/**
 * @brief Update signal indicator
 */
static void update_signal_indicator (GtkWidget *indicator, gboolean active) {
    GtkWidget *led = g_object_get_data (G_OBJECT (indicator), "led");

    if (active) {
        gtk_widget_remove_css_class (led, "signal-off");
        gtk_widget_add_css_class (led, "signal-on");
    } else {
        gtk_widget_remove_css_class (led, "signal-on");
        gtk_widget_add_css_class (led, "signal-off");
    }
}

/**
 * @brief Clear temporary message
 */
static gboolean clear_message_timeout (gpointer user_data) {
    GtkTermStatusbar *self = GTKTERM_STATUSBAR (user_data);

    gtk_label_set_text (GTK_LABEL (self->message_label), "");
    gtk_widget_set_visible (self->message_label, FALSE);
    self->message_timeout_id = 0;

    return G_SOURCE_REMOVE;
}

/**
 * @brief Set connection status
 */
void gtkterm_statusbar_set_status (GtkTermStatusbar *self,
                                    GtkTermStatusState state,
                                    const char *message) {
    g_return_if_fail (GTKTERM_IS_STATUSBAR (self));

    self->current_state = state;

    const char *icon_name = NULL;
    const char *css_class = NULL;

    switch (state) {
        case GTKTERM_STATUS_DISCONNECTED:
            icon_name = "network-offline-symbolic";
            css_class = "status-disconnected";
            break;
        case GTKTERM_STATUS_CONNECTING:
            icon_name = "network-transmit-symbolic";
            css_class = "status-connecting";
            break;
        case GTKTERM_STATUS_CONNECTED:
            icon_name = "network-transmit-receive-symbolic";
            css_class = "status-connected";
            break;
        case GTKTERM_STATUS_ERROR:
            icon_name = "dialog-error-symbolic";
            css_class = "status-error";
            break;
    }

    gtk_image_set_from_icon_name (GTK_IMAGE (self->status_icon), icon_name);

    // Remove old CSS classes
    gtk_widget_remove_css_class (self->status_icon, "status-disconnected");
    gtk_widget_remove_css_class (self->status_icon, "status-connecting");
    gtk_widget_remove_css_class (self->status_icon, "status-connected");
    gtk_widget_remove_css_class (self->status_icon, "status-error");

    if (css_class)
        gtk_widget_add_css_class (self->status_icon, css_class);

    if (message)
        gtk_label_set_text (GTK_LABEL (self->status_label), message);
}

/**
 * @brief Update port information
 */
void gtkterm_statusbar_set_port_info (GtkTermStatusbar *self,
                                       const char *port_name,
                                       guint32 baud_rate,
                                       const char *format) {
    g_return_if_fail (GTKTERM_IS_STATUSBAR (self));

    char *text = g_strdup_printf ("%s @ %u bps %s",
                                   port_name ? port_name : "No Port",
                                   baud_rate,
                                   format ? format : "");

    gtk_label_set_text (GTK_LABEL (self->port_label), text);
    g_free (text);
}

/**
 * @brief Format byte count
 */
static char *format_bytes (guint64 bytes) {
    if (bytes < 1024)
        return g_strdup_printf ("%lu B", bytes);
    else if (bytes < 1024 * 1024)
        return g_strdup_printf ("%.1f KB", bytes / 1024.0);
    else if (bytes < 1024 * 1024 * 1024)
        return g_strdup_printf ("%.1f MB", bytes / (1024.0 * 1024.0));
    else
        return g_strdup_printf ("%.2f GB", bytes / (1024.0 * 1024.0 * 1024.0));
}

/**
 * @brief Format rate
 */
static char *format_rate (gdouble bytes_per_sec) {
    if (bytes_per_sec < 1024)
        return g_strdup_printf ("%.0f B/s", bytes_per_sec);
    else if (bytes_per_sec < 1024 * 1024)
        return g_strdup_printf ("%.1f KB/s", bytes_per_sec / 1024.0);
    else
        return g_strdup_printf ("%.1f MB/s", bytes_per_sec / (1024.0 * 1024.0));
}

/**
 * @brief Update statistics
 */
void gtkterm_statusbar_update_stats (GtkTermStatusbar *self,
                                      guint64 rx_bytes,
                                      guint64 tx_bytes,
                                      gdouble rx_rate,
                                      gdouble tx_rate) {
    g_return_if_fail (GTKTERM_IS_STATUSBAR (self));

    self->total_rx = rx_bytes;
    self->total_tx = tx_bytes;

    char *rx_str = format_bytes (rx_bytes);
    char *tx_str = format_bytes (tx_bytes);
    char *rx_rate_str = format_rate (rx_rate);
    char *tx_rate_str = format_rate (tx_rate);

    char *stats_text = g_strdup_printf ("↓ %s (%s) ↑ %s (%s)",
                                         rx_str, rx_rate_str,
                                         tx_str, tx_rate_str);

    gtk_label_set_text (GTK_LABEL (self->stats_label), stats_text);

    g_free (rx_str);
    g_free (tx_str);
    g_free (rx_rate_str);
    g_free (tx_rate_str);
    g_free (stats_text);
}

/**
 * @brief Set signal states
 */
void gtkterm_statusbar_set_signals (GtkTermStatusbar *self,
                                     gboolean dtr,
                                     gboolean rts,
                                     gboolean cts,
                                     gboolean dsr,
                                     gboolean dcd) {
    g_return_if_fail (GTKTERM_IS_STATUSBAR (self));

    update_signal_indicator (self->dtr_indicator, dtr);
    update_signal_indicator (self->rts_indicator, rts);
    update_signal_indicator (self->cts_indicator, cts);
    update_signal_indicator (self->dsr_indicator, dsr);
    update_signal_indicator (self->dcd_indicator, dcd);
}

/**
 * @brief Show temporary message
 */
void gtkterm_statusbar_show_message (GtkTermStatusbar *self,
                                      const char *message,
                                      guint timeout_ms) {
    g_return_if_fail (GTKTERM_IS_STATUSBAR (self));

    if (self->message_timeout_id) {
        g_source_remove (self->message_timeout_id);
        self->message_timeout_id = 0;
    }

    gtk_label_set_text (GTK_LABEL (self->message_label), message);
    gtk_widget_set_visible (self->message_label, TRUE);

    if (timeout_ms > 0) {
        self->message_timeout_id = g_timeout_add (timeout_ms,
                                                   clear_message_timeout,
                                                   self);
    }
}

/**
 * @brief Set protocol decoder status
 */
void gtkterm_statusbar_set_protocol (GtkTermStatusbar *self,
                                      const char *protocol_name) {
    g_return_if_fail (GTKTERM_IS_STATUSBAR (self));

    if (protocol_name && strlen (protocol_name) > 0) {
        char *text = g_strdup_printf ("Protocol: %s", protocol_name);
        gtk_label_set_text (GTK_LABEL (self->protocol_label), text);
        gtk_widget_set_visible (self->protocol_label, TRUE);
        g_free (text);
    } else {
        gtk_widget_set_visible (self->protocol_label, FALSE);
    }
}

/**
 * @brief Set recording status
 */
void gtkterm_statusbar_set_recording (GtkTermStatusbar *self,
                                       gboolean recording,
                                       const char *filename) {
    g_return_if_fail (GTKTERM_IS_STATUSBAR (self));

    if (recording) {
        gtk_image_set_from_icon_name (GTK_IMAGE (self->recording_icon),
                                       "media-record-symbolic");
        gtk_widget_add_css_class (self->recording_icon, "recording");

        if (filename) {
            char *tooltip = g_strdup_printf ("Recording to: %s", filename);
            gtk_widget_set_tooltip_text (self->recording_icon, tooltip);
            g_free (tooltip);
        }
    } else {
        gtk_widget_remove_css_class (self->recording_icon, "recording");
        gtk_widget_set_visible (self->recording_icon, FALSE);
    }

    gtk_widget_set_visible (self->recording_icon, recording);
}

/**
 * @brief Dispose
 */
static void gtkterm_statusbar_dispose (GObject *object) {
    GtkTermStatusbar *self = GTKTERM_STATUSBAR (object);

    if (self->message_timeout_id) {
        g_source_remove (self->message_timeout_id);
        self->message_timeout_id = 0;
    }

    G_OBJECT_CLASS (gtkterm_statusbar_parent_class)->dispose (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_statusbar_class_init (GtkTermStatusbarClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_statusbar_dispose;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_statusbar_init (GtkTermStatusbar *self) {
    gtk_orientable_set_orientation (GTK_ORIENTABLE (self), GTK_ORIENTATION_HORIZONTAL);
    gtk_box_set_spacing (GTK_BOX (self), 6);
    gtk_widget_add_css_class (GTK_WIDGET (self), "statusbar");

    // Status icon and label
    self->status_icon = gtk_image_new_from_icon_name ("network-offline-symbolic");
    gtk_box_append (GTK_BOX (self), self->status_icon);

    self->status_label = gtk_label_new ("Disconnected");
    gtk_widget_add_css_class (self->status_label, "status-text");
    gtk_box_append (GTK_BOX (self), self->status_label);

    // Separator
    GtkWidget *sep1 = gtk_separator_new (GTK_ORIENTATION_VERTICAL);
    gtk_box_append (GTK_BOX (self), sep1);

    // Port information
    self->port_label = gtk_label_new ("No Port");
    gtk_widget_add_css_class (self->port_label, "port-info");
    gtk_box_append (GTK_BOX (self), self->port_label);

    // Separator
    GtkWidget *sep2 = gtk_separator_new (GTK_ORIENTATION_VERTICAL);
    gtk_box_append (GTK_BOX (self), sep2);

    // Statistics
    self->stats_label = gtk_label_new ("↓ 0 B ↑ 0 B");
    gtk_widget_add_css_class (self->stats_label, "stats-info");
    gtk_label_set_width_chars (GTK_LABEL (self->stats_label), 30);
    gtk_box_append (GTK_BOX (self), self->stats_label);

    // Separator
    GtkWidget *sep3 = gtk_separator_new (GTK_ORIENTATION_VERTICAL);
    gtk_box_append (GTK_BOX (self), sep3);

    // Signal indicators
    GtkWidget *signals_box = gtk_box_new (GTK_ORIENTATION_HORIZONTAL, 4);
    gtk_widget_add_css_class (signals_box, "signal-box");

    self->dtr_indicator = create_signal_indicator ("DTR");
    self->rts_indicator = create_signal_indicator ("RTS");
    self->cts_indicator = create_signal_indicator ("CTS");
    self->dsr_indicator = create_signal_indicator ("DSR");
    self->dcd_indicator = create_signal_indicator ("DCD");

    gtk_box_append (GTK_BOX (signals_box), self->dtr_indicator);
    gtk_box_append (GTK_BOX (signals_box), self->rts_indicator);
    gtk_box_append (GTK_BOX (signals_box), self->cts_indicator);
    gtk_box_append (GTK_BOX (signals_box), self->dsr_indicator);
    gtk_box_append (GTK_BOX (signals_box), self->dcd_indicator);

    gtk_box_append (GTK_BOX (self), signals_box);

    // Spacer to push right-aligned items
    GtkWidget *spacer = gtk_label_new ("");
    gtk_widget_set_hexpand (spacer, TRUE);
    gtk_box_append (GTK_BOX (self), spacer);

    // Protocol label
    self->protocol_label = gtk_label_new ("");
    gtk_widget_add_css_class (self->protocol_label, "protocol-info");
    gtk_widget_set_visible (self->protocol_label, FALSE);
    gtk_box_append (GTK_BOX (self), self->protocol_label);

    // Recording icon
    self->recording_icon = gtk_image_new_from_icon_name ("media-record-symbolic");
    gtk_widget_set_visible (self->recording_icon, FALSE);
    gtk_box_append (GTK_BOX (self), self->recording_icon);

    // Temporary message label
    self->message_label = gtk_label_new ("");
    gtk_widget_add_css_class (self->message_label, "message-label");
    gtk_widget_set_visible (self->message_label, FALSE);
    gtk_box_append (GTK_BOX (self), self->message_label);

    // Initialize state
    self->current_state = GTKTERM_STATUS_DISCONNECTED;
    self->total_rx = 0;
    self->total_tx = 0;
    self->message_timeout_id = 0;
}

/**
 * @brief Create a new statusbar
 */
GtkWidget *gtkterm_statusbar_new (void) {
    return g_object_new (GTKTERM_TYPE_STATUSBAR, NULL);
}
