/************************************************************************/
/* gtkterm_toolbar.c                                                    */
/* -----------------                                                    */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Quick access toolbar implementation                            */
/*                                                                      */
/************************************************************************/

#include "gtkterm_toolbar.h"

struct _GtkTermToolbar {
    GtkBox parent_instance;

    /* Action buttons */
    GtkWidget *connect_button;
    GtkWidget *disconnect_button;
    GtkWidget *send_file_button;
    GtkWidget *receive_file_button;
    GtkWidget *record_button;
    GtkWidget *clear_button;
    GtkWidget *send_break_button;
    GtkWidget *run_script_button;
    GtkWidget *configure_button;

    /* Toggle buttons */
    GtkWidget *dtr_toggle;
    GtkWidget *rts_toggle;

    /* Protocol selection */
    GtkWidget *protocol_dropdown;
    GtkStringList *protocol_list;

    /* State */
    gboolean connected;
    gboolean recording;
    gboolean dtr_active;
    gboolean rts_active;
};

G_DEFINE_TYPE (GtkTermToolbar, gtkterm_toolbar, GTK_TYPE_BOX)

/* Signals */
enum {
    SIGNAL_ACTION,
    SIGNAL_PROTOCOL_CHANGED,
    LAST_SIGNAL
};

static guint signals[LAST_SIGNAL] = { 0 };

/**
 * @brief Create toolbar button with icon
 */
static GtkWidget *create_button (const char *icon_name, const char *tooltip) {
    GtkWidget *button = gtk_button_new_from_icon_name (icon_name);
    gtk_widget_set_tooltip_text (button, tooltip);
    gtk_widget_add_css_class (button, "toolbar-button");
    return button;
}

/**
 * @brief Create toggle button with icon
 */
static GtkWidget *create_toggle_button (const char *icon_name, const char *tooltip) {
    GtkWidget *button = gtk_toggle_button_new ();
    GtkWidget *icon = gtk_image_new_from_icon_name (icon_name);
    gtk_button_set_child (GTK_BUTTON (button), icon);
    gtk_widget_set_tooltip_text (button, tooltip);
    gtk_widget_add_css_class (button, "toolbar-toggle");
    return button;
}

/**
 * @brief Emit action signal
 */
static void emit_action (GtkTermToolbar *self, GtkTermToolbarAction action) {
    g_signal_emit (self, signals[SIGNAL_ACTION], 0, action);
}

/**
 * @brief Button click handlers
 */
static void on_connect_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_CONNECT);
}

static void on_disconnect_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_DISCONNECT);
}

static void on_send_file_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_SEND_FILE);
}

static void on_receive_file_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_RECEIVE_FILE);
}

static void on_record_toggled (GtkToggleButton *toggle, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    self->recording = gtk_toggle_button_get_active (toggle);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_RECORD);
}

static void on_clear_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_CLEAR);
}

static void on_send_break_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_SEND_BREAK);
}

static void on_dtr_toggled (GtkToggleButton *toggle, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    self->dtr_active = gtk_toggle_button_get_active (toggle);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_TOGGLE_DTR);
}

static void on_rts_toggled (GtkToggleButton *toggle, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    self->rts_active = gtk_toggle_button_get_active (toggle);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_TOGGLE_RTS);
}

static void on_run_script_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_RUN_SCRIPT);
}

static void on_configure_clicked (GtkButton *button, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    emit_action (self, GTKTERM_TOOLBAR_ACTION_CONFIGURE);
}

static void on_protocol_changed (GtkDropDown *dropdown, GParamSpec *pspec, gpointer user_data) {
    GtkTermToolbar *self = GTKTERM_TOOLBAR (user_data);
    g_signal_emit (self, signals[SIGNAL_PROTOCOL_CHANGED], 0);
}

/**
 * @brief Set connection state
 */
void gtkterm_toolbar_set_connected (GtkTermToolbar *self, gboolean connected) {
    g_return_if_fail (GTKTERM_IS_TOOLBAR (self));

    self->connected = connected;

    gtk_widget_set_visible (self->connect_button, !connected);
    gtk_widget_set_visible (self->disconnect_button, connected);

    // Enable/disable buttons based on connection state
    gtk_widget_set_sensitive (self->send_file_button, connected);
    gtk_widget_set_sensitive (self->receive_file_button, connected);
    gtk_widget_set_sensitive (self->record_button, connected);
    gtk_widget_set_sensitive (self->send_break_button, connected);
    gtk_widget_set_sensitive (self->dtr_toggle, connected);
    gtk_widget_set_sensitive (self->rts_toggle, connected);
    gtk_widget_set_sensitive (self->run_script_button, connected);
}

/**
 * @brief Set recording state
 */
void gtkterm_toolbar_set_recording (GtkTermToolbar *self, gboolean recording) {
    g_return_if_fail (GTKTERM_IS_TOOLBAR (self));

    self->recording = recording;

    g_signal_handlers_block_by_func (self->record_button, on_record_toggled, self);
    gtk_toggle_button_set_active (GTK_TOGGLE_BUTTON (self->record_button), recording);
    g_signal_handlers_unblock_by_func (self->record_button, on_record_toggled, self);

    if (recording) {
        gtk_widget_add_css_class (self->record_button, "recording");
    } else {
        gtk_widget_remove_css_class (self->record_button, "recording");
    }
}

/**
 * @brief Set DTR state
 */
void gtkterm_toolbar_set_dtr (GtkTermToolbar *self, gboolean active) {
    g_return_if_fail (GTKTERM_IS_TOOLBAR (self));

    self->dtr_active = active;

    g_signal_handlers_block_by_func (self->dtr_toggle, on_dtr_toggled, self);
    gtk_toggle_button_set_active (GTK_TOGGLE_BUTTON (self->dtr_toggle), active);
    g_signal_handlers_unblock_by_func (self->dtr_toggle, on_dtr_toggled, self);

    if (active) {
        gtk_widget_add_css_class (self->dtr_toggle, "signal-active");
    } else {
        gtk_widget_remove_css_class (self->dtr_toggle, "signal-active");
    }
}

/**
 * @brief Set RTS state
 */
void gtkterm_toolbar_set_rts (GtkTermToolbar *self, gboolean active) {
    g_return_if_fail (GTKTERM_IS_TOOLBAR (self));

    self->rts_active = active;

    g_signal_handlers_block_by_func (self->rts_toggle, on_rts_toggled, self);
    gtk_toggle_button_set_active (GTK_TOGGLE_BUTTON (self->rts_toggle), active);
    g_signal_handlers_unblock_by_func (self->rts_toggle, on_rts_toggled, self);

    if (active) {
        gtk_widget_add_css_class (self->rts_toggle, "signal-active");
    } else {
        gtk_widget_remove_css_class (self->rts_toggle, "signal-active");
    }
}

/**
 * @brief Enable/disable toolbar buttons
 */
void gtkterm_toolbar_set_sensitive (GtkTermToolbar *self, gboolean sensitive) {
    g_return_if_fail (GTKTERM_IS_TOOLBAR (self));

    gtk_widget_set_sensitive (GTK_WIDGET (self), sensitive);
}

/**
 * @brief Set available protocols
 */
void gtkterm_toolbar_set_protocols (GtkTermToolbar *self, const char **protocol_names) {
    g_return_if_fail (GTKTERM_IS_TOOLBAR (self));

    if (self->protocol_list) {
        g_object_unref (self->protocol_list);
    }

    self->protocol_list = gtk_string_list_new (protocol_names);
    gtk_drop_down_set_model (GTK_DROP_DOWN (self->protocol_dropdown),
                             G_LIST_MODEL (self->protocol_list));
}

/**
 * @brief Get selected protocol
 */
const char *gtkterm_toolbar_get_selected_protocol (GtkTermToolbar *self) {
    g_return_val_if_fail (GTKTERM_IS_TOOLBAR (self), NULL);

    guint selected = gtk_drop_down_get_selected (GTK_DROP_DOWN (self->protocol_dropdown));

    if (selected == GTK_INVALID_LIST_POSITION) {
        return NULL;
    }

    return gtk_string_list_get_string (self->protocol_list, selected);
}

/**
 * @brief Class initialization
 */
static void gtkterm_toolbar_class_init (GtkTermToolbarClass *klass) {
    /**
     * GtkTermToolbar::action:
     * @self: the toolbar
     * @action: the action that was triggered
     *
     * Emitted when a toolbar button is clicked
     */
    signals[SIGNAL_ACTION] = g_signal_new (
        "action",
        G_TYPE_FROM_CLASS (klass),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL, NULL,
        G_TYPE_NONE,
        1,
        G_TYPE_UINT
    );

    /**
     * GtkTermToolbar::protocol-changed:
     * @self: the toolbar
     *
     * Emitted when the protocol selection changes
     */
    signals[SIGNAL_PROTOCOL_CHANGED] = g_signal_new (
        "protocol-changed",
        G_TYPE_FROM_CLASS (klass),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL, NULL,
        G_TYPE_NONE,
        0
    );
}

/**
 * @brief Instance initialization
 */
static void gtkterm_toolbar_init (GtkTermToolbar *self) {
    gtk_orientable_set_orientation (GTK_ORIENTABLE (self), GTK_ORIENTATION_HORIZONTAL);
    gtk_box_set_spacing (GTK_BOX (self), 4);
    gtk_widget_add_css_class (GTK_WIDGET (self), "toolbar");

    // Connection buttons
    self->connect_button = create_button ("network-wired-symbolic", "Connect to port");
    gtk_widget_add_css_class (self->connect_button, "suggested-action");
    g_signal_connect (self->connect_button, "clicked", G_CALLBACK (on_connect_clicked), self);
    gtk_box_append (GTK_BOX (self), self->connect_button);

    self->disconnect_button = create_button ("network-wired-disconnected-symbolic", "Disconnect");
    gtk_widget_add_css_class (self->disconnect_button, "destructive-action");
    g_signal_connect (self->disconnect_button, "clicked", G_CALLBACK (on_disconnect_clicked), self);
    gtk_box_append (GTK_BOX (self), self->disconnect_button);
    gtk_widget_set_visible (self->disconnect_button, FALSE);

    // Separator
    GtkWidget *sep1 = gtk_separator_new (GTK_ORIENTATION_VERTICAL);
    gtk_box_append (GTK_BOX (self), sep1);

    // File transfer buttons
    self->send_file_button = create_button ("document-send-symbolic", "Send file");
    g_signal_connect (self->send_file_button, "clicked", G_CALLBACK (on_send_file_clicked), self);
    gtk_box_append (GTK_BOX (self), self->send_file_button);

    self->receive_file_button = create_button ("document-save-symbolic", "Receive file");
    g_signal_connect (self->receive_file_button, "clicked", G_CALLBACK (on_receive_file_clicked), self);
    gtk_box_append (GTK_BOX (self), self->receive_file_button);

    // Record button (toggle)
    self->record_button = create_toggle_button ("media-record-symbolic", "Record session");
    g_signal_connect (self->record_button, "toggled", G_CALLBACK (on_record_toggled), self);
    gtk_box_append (GTK_BOX (self), self->record_button);

    // Separator
    GtkWidget *sep2 = gtk_separator_new (GTK_ORIENTATION_VERTICAL);
    gtk_box_append (GTK_BOX (self), sep2);

    // Buffer control
    self->clear_button = create_button ("edit-clear-all-symbolic", "Clear buffer");
    g_signal_connect (self->clear_button, "clicked", G_CALLBACK (on_clear_clicked), self);
    gtk_box_append (GTK_BOX (self), self->clear_button);

    self->send_break_button = create_button ("media-playback-pause-symbolic", "Send break signal");
    g_signal_connect (self->send_break_button, "clicked", G_CALLBACK (on_send_break_clicked), self);
    gtk_box_append (GTK_BOX (self), self->send_break_button);

    // Separator
    GtkWidget *sep3 = gtk_separator_new (GTK_ORIENTATION_VERTICAL);
    gtk_box_append (GTK_BOX (self), sep3);

    // Signal controls
    GtkWidget *signals_box = gtk_box_new (GTK_ORIENTATION_HORIZONTAL, 2);
    gtk_widget_add_css_class (signals_box, "linked");

    self->dtr_toggle = create_toggle_button ("emblem-synchronizing-symbolic", "Toggle DTR");
    GtkWidget *dtr_label = gtk_label_new ("DTR");
    gtk_widget_add_css_class (dtr_label, "signal-label");
    GtkWidget *dtr_box = gtk_box_new (GTK_ORIENTATION_HORIZONTAL, 4);
    gtk_box_append (GTK_BOX (dtr_box), gtk_button_get_child (GTK_BUTTON (self->dtr_toggle)));
    gtk_button_set_child (GTK_BUTTON (self->dtr_toggle), NULL);
    gtk_box_append (GTK_BOX (dtr_box), dtr_label);
    gtk_button_set_child (GTK_BUTTON (self->dtr_toggle), dtr_box);
    g_signal_connect (self->dtr_toggle, "toggled", G_CALLBACK (on_dtr_toggled), self);
    gtk_box_append (GTK_BOX (signals_box), self->dtr_toggle);

    self->rts_toggle = create_toggle_button ("emblem-synchronizing-symbolic", "Toggle RTS");
    GtkWidget *rts_label = gtk_label_new ("RTS");
    gtk_widget_add_css_class (rts_label, "signal-label");
    GtkWidget *rts_box = gtk_box_new (GTK_ORIENTATION_HORIZONTAL, 4);
    gtk_box_append (GTK_BOX (rts_box), gtk_button_get_child (GTK_BUTTON (self->rts_toggle)));
    gtk_button_set_child (GTK_BUTTON (self->rts_toggle), NULL);
    gtk_box_append (GTK_BOX (rts_box), rts_label);
    gtk_button_set_child (GTK_BUTTON (self->rts_toggle), rts_box);
    g_signal_connect (self->rts_toggle, "toggled", G_CALLBACK (on_rts_toggled), self);
    gtk_box_append (GTK_BOX (signals_box), self->rts_toggle);

    gtk_box_append (GTK_BOX (self), signals_box);

    // Separator
    GtkWidget *sep4 = gtk_separator_new (GTK_ORIENTATION_VERTICAL);
    gtk_box_append (GTK_BOX (self), sep4);

    // Automation
    self->run_script_button = create_button ("media-playback-start-symbolic", "Run automation script");
    g_signal_connect (self->run_script_button, "clicked", G_CALLBACK (on_run_script_clicked), self);
    gtk_box_append (GTK_BOX (self), self->run_script_button);

    // Spacer to push right-aligned items
    GtkWidget *spacer = gtk_label_new ("");
    gtk_widget_set_hexpand (spacer, TRUE);
    gtk_box_append (GTK_BOX (self), spacer);

    // Protocol decoder dropdown
    const char *default_protocols[] = { "None", "AT Commands", "Modbus", "NMEA", "CAN", NULL };
    self->protocol_list = gtk_string_list_new (default_protocols);
    self->protocol_dropdown = gtk_drop_down_new (G_LIST_MODEL (self->protocol_list), NULL);
    gtk_widget_set_tooltip_text (self->protocol_dropdown, "Select protocol decoder");
    gtk_widget_add_css_class (self->protocol_dropdown, "protocol-selector");
    g_signal_connect (self->protocol_dropdown, "notify::selected", G_CALLBACK (on_protocol_changed), self);
    gtk_box_append (GTK_BOX (self), self->protocol_dropdown);

    // Configure button
    self->configure_button = create_button ("preferences-system-symbolic", "Configure port");
    g_signal_connect (self->configure_button, "clicked", G_CALLBACK (on_configure_clicked), self);
    gtk_box_append (GTK_BOX (self), self->configure_button);

    // Initialize state
    self->connected = FALSE;
    self->recording = FALSE;
    self->dtr_active = FALSE;
    self->rts_active = FALSE;

    // Set initial button states
    gtkterm_toolbar_set_connected (self, FALSE);
}

/**
 * @brief Create a new toolbar
 */
GtkWidget *gtkterm_toolbar_new (void) {
    return g_object_new (GTKTERM_TYPE_TOOLBAR, NULL);
}
