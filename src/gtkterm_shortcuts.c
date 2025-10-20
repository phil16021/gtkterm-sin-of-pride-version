/************************************************************************/
/* gtkterm_shortcuts.c                                                  */
/* -------------------                                                  */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Keyboard shortcuts management implementation                   */
/*                                                                      */
/************************************************************************/

#include "gtkterm_shortcuts.h"
#include <string.h>

struct _GtkTermShortcuts {
    GObject parent_instance;

    GtkApplication *app;
    char *accelerators[GTKTERM_ACTION_COUNT];
};

G_DEFINE_TYPE (GtkTermShortcuts, gtkterm_shortcuts, G_TYPE_OBJECT)

/**
 * @brief Default shortcut definitions
 * IMPORTANT: This array MUST be in the same order as the GtkTermAction enum!
 */
static const GtkTermShortcutInfo shortcut_infos[GTKTERM_ACTION_COUNT] = {
    /* Keep in sync with GtkTermAction enum in gtkterm_shortcuts.h */
    { GTKTERM_ACTION_QUIT, "app.quit", "Quit application", "<Ctrl>Q", "Application" },
    { GTKTERM_ACTION_NEW_TAB, "win.new-tab", "New tab", "<Ctrl>T", "Tabs" },
    { GTKTERM_ACTION_CLOSE_TAB, "win.close-tab", "Close tab", "<Ctrl>W", "Tabs" },
    { GTKTERM_ACTION_NEXT_TAB, "win.next-tab", "Next tab", "<Ctrl>Page_Down", "Tabs" },
    { GTKTERM_ACTION_PREV_TAB, "win.prev-tab", "Previous tab", "<Ctrl>Page_Up", "Tabs" },
    { GTKTERM_ACTION_CONNECT, "win.connect", "Connect to port", "<Ctrl>O", "Connection" },
    { GTKTERM_ACTION_DISCONNECT, "win.disconnect", "Disconnect", "<Ctrl><Shift>O", "Connection" },
    { GTKTERM_ACTION_RECONNECT, "win.reconnect", "Reconnect", "<Ctrl>R", "Connection" },
    { GTKTERM_ACTION_SEND_FILE, "win.send-file", "Send file", "<Ctrl><Shift>S", "File Transfer" },
    { GTKTERM_ACTION_RECEIVE_FILE, "win.receive-file", "Receive file", "<Ctrl><Shift>R", "File Transfer" },
    { GTKTERM_ACTION_RECORD_START, "win.record-start", "Start recording", "<Ctrl><Shift>E", "Recording" },
    { GTKTERM_ACTION_RECORD_STOP, "win.record-stop", "Stop recording", NULL, "Recording" },
    { GTKTERM_ACTION_CLEAR_BUFFER, "win.clear-buffer", "Clear buffer", "<Ctrl>L", "Editing" },
    { GTKTERM_ACTION_COPY, "win.copy", "Copy", "<Ctrl>C", "Editing" },
    { GTKTERM_ACTION_PASTE, "win.paste", "Paste", "<Ctrl>V", "Editing" },
    { GTKTERM_ACTION_SELECT_ALL, "win.select-all", "Select all", "<Ctrl>A", "Editing" },
    { GTKTERM_ACTION_FIND, "win.find", "Find", "<Ctrl>F", "Editing" },
    { GTKTERM_ACTION_PREFERENCES, "app.preferences", "Open preferences", "<Ctrl>comma", "Application" },
    { GTKTERM_ACTION_SEND_BREAK, "win.send-break", "Send break signal", "<Ctrl>B", "Serial Control" },
    { GTKTERM_ACTION_TOGGLE_DTR, "win.toggle-dtr", "Toggle DTR", "<Ctrl>D", "Serial Control" },
    { GTKTERM_ACTION_TOGGLE_RTS, "win.toggle-rts", "Toggle RTS", NULL, "Serial Control" },
    { GTKTERM_ACTION_RUN_SCRIPT, "win.run-script", "Run automation script", "<Ctrl><Shift>A", "Automation" },
    { GTKTERM_ACTION_TOGGLE_HEX, "win.toggle-hex", "Toggle hex view", "<Ctrl>H", "View" },
    { GTKTERM_ACTION_TOGGLE_TIMESTAMP, "win.toggle-timestamp", "Toggle timestamps", "<Ctrl><Shift>T", "View" },
    { GTKTERM_ACTION_INCREASE_FONT, "win.increase-font", "Increase font size", "<Ctrl>plus", "View" },
    { GTKTERM_ACTION_DECREASE_FONT, "win.decrease-font", "Decrease font size", "<Ctrl>minus", "View" },
    { GTKTERM_ACTION_RESET_FONT, "win.reset-font", "Reset font size", "<Ctrl>0", "View" },
    { GTKTERM_ACTION_FULLSCREEN, "win.fullscreen", "Toggle fullscreen", "F11", "Application" },
    { GTKTERM_ACTION_HELP, "app.help", "Show help", "F1", "Application" },
    { GTKTERM_ACTION_ABOUT, "app.about", "About GTKTerm", NULL, "Application" },
};

/**
 * @brief Get shortcut info
 */
const GtkTermShortcutInfo *gtkterm_shortcuts_get_info (GtkTermAction action) {
    if (action >= 0 && action < GTKTERM_ACTION_COUNT) {
        return &shortcut_infos[action];
    }
    return NULL;
}

/**
 * @brief Get all shortcut infos
 */
const GtkTermShortcutInfo *gtkterm_shortcuts_get_all_infos (void) {
    return shortcut_infos;
}

/**
 * @brief Apply accelerators to application
 */
static void apply_accelerators (GtkTermShortcuts *self) {
    for (int i = 0; i < GTKTERM_ACTION_COUNT; i++) {
        const char *accel = self->accelerators[i];
        if (!accel && shortcut_infos[i].default_accel) {
            accel = shortcut_infos[i].default_accel;
        }

        if (accel) {
            const char *accels[] = { accel, NULL };
            gtk_application_set_accels_for_action (self->app,
                                                    shortcut_infos[i].name,
                                                    accels);
        }
    }
}

/**
 * @brief Set custom accelerator for action
 */
gboolean gtkterm_shortcuts_set_accelerator (GtkTermShortcuts *self,
                                             GtkTermAction action,
                                             const char *accel,
                                             GError **error) {
    g_return_val_if_fail (GTKTERM_IS_SHORTCUTS (self), FALSE);
    g_return_val_if_fail (action >= 0 && action < GTKTERM_ACTION_COUNT, FALSE);

    // Validate accelerator
    if (accel && strlen (accel) > 0) {
        guint key;
        GdkModifierType mods;
        gtk_accelerator_parse (accel, &key, &mods);

        if (key == 0 && mods == 0) {
            g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "Invalid accelerator: %s", accel);
            return FALSE;
        }
    }

    // Update accelerator
    g_free (self->accelerators[action]);
    self->accelerators[action] = accel ? g_strdup (accel) : NULL;

    apply_accelerators (self);
    return TRUE;
}

/**
 * @brief Get accelerator for action
 */
const char *gtkterm_shortcuts_get_accelerator (GtkTermShortcuts *self,
                                                 GtkTermAction action) {
    g_return_val_if_fail (GTKTERM_IS_SHORTCUTS (self), NULL);
    g_return_val_if_fail (action >= 0 && action < GTKTERM_ACTION_COUNT, NULL);

    if (self->accelerators[action]) {
        return self->accelerators[action];
    }

    return shortcut_infos[action].default_accel;
}

/**
 * @brief Reset action to default accelerator
 */
void gtkterm_shortcuts_reset_to_default (GtkTermShortcuts *self,
                                          GtkTermAction action) {
    g_return_if_fail (GTKTERM_IS_SHORTCUTS (self));
    g_return_if_fail (action >= 0 && action < GTKTERM_ACTION_COUNT);

    g_free (self->accelerators[action]);
    self->accelerators[action] = NULL;

    apply_accelerators (self);
}

/**
 * @brief Reset all to defaults
 */
void gtkterm_shortcuts_reset_all (GtkTermShortcuts *self) {
    g_return_if_fail (GTKTERM_IS_SHORTCUTS (self));

    for (int i = 0; i < GTKTERM_ACTION_COUNT; i++) {
        g_free (self->accelerators[i]);
        self->accelerators[i] = NULL;
    }

    apply_accelerators (self);
}

/**
 * @brief Load shortcuts from configuration
 */
gboolean gtkterm_shortcuts_load (GtkTermShortcuts *self,
                                  const char *config_file,
                                  GError **error) {
    g_return_val_if_fail (GTKTERM_IS_SHORTCUTS (self), FALSE);
    g_return_val_if_fail (config_file != NULL, FALSE);

    GKeyFile *keyfile = g_key_file_new ();

    if (!g_key_file_load_from_file (keyfile, config_file, G_KEY_FILE_NONE, error)) {
        g_key_file_free (keyfile);
        return FALSE;
    }

    // Load custom accelerators
    for (int i = 0; i < GTKTERM_ACTION_COUNT; i++) {
        char *accel = g_key_file_get_string (keyfile, "shortcuts",
                                              shortcut_infos[i].name, NULL);
        if (accel) {
            g_free (self->accelerators[i]);
            self->accelerators[i] = accel;
        }
    }

    g_key_file_free (keyfile);
    apply_accelerators (self);

    return TRUE;
}

/**
 * @brief Save shortcuts to configuration
 */
gboolean gtkterm_shortcuts_save (GtkTermShortcuts *self,
                                  const char *config_file,
                                  GError **error) {
    g_return_val_if_fail (GTKTERM_IS_SHORTCUTS (self), FALSE);
    g_return_val_if_fail (config_file != NULL, FALSE);

    GKeyFile *keyfile = g_key_file_new ();

    // Save custom accelerators
    for (int i = 0; i < GTKTERM_ACTION_COUNT; i++) {
        if (self->accelerators[i]) {
            g_key_file_set_string (keyfile, "shortcuts",
                                   shortcut_infos[i].name,
                                   self->accelerators[i]);
        }
    }

    // Write to file
    gboolean success = g_key_file_save_to_file (keyfile, config_file, error);
    g_key_file_free (keyfile);

    return success;
}

/**
 * @brief Create shortcut row
 */
static GtkWidget *create_shortcut_row (const char *description, const char *accel) {
    GtkWidget *box = gtk_box_new (GTK_ORIENTATION_HORIZONTAL, 12);
    gtk_widget_add_css_class (box, "shortcut-row");
    gtk_widget_set_margin_start (box, 12);
    gtk_widget_set_margin_end (box, 12);
    gtk_widget_set_margin_top (box, 6);
    gtk_widget_set_margin_bottom (box, 6);

    GtkWidget *label = gtk_label_new (description);
    gtk_widget_set_hexpand (label, TRUE);
    gtk_widget_set_halign (label, GTK_ALIGN_START);
    gtk_box_append (GTK_BOX (box), label);

    GtkWidget *accel_label = gtk_label_new (accel);
    gtk_widget_add_css_class (accel_label, "accelerator");
    gtk_widget_set_halign (accel_label, GTK_ALIGN_END);
    gtk_box_append (GTK_BOX (box), accel_label);

    return box;
}

/**
 * @brief Create shortcuts category
 */
static GtkWidget *create_shortcuts_category (const char *category) {
    GtkWidget *frame = gtk_frame_new (category);
    gtk_widget_set_margin_start (frame, 12);
    gtk_widget_set_margin_end (frame, 12);
    gtk_widget_set_margin_top (frame, 12);
    gtk_widget_set_margin_bottom (frame, 12);

    GtkWidget *list_box = gtk_list_box_new ();
    gtk_widget_add_css_class (list_box, "shortcuts-list");

    for (int i = 0; i < GTKTERM_ACTION_COUNT; i++) {
        if (g_strcmp0 (shortcut_infos[i].category, category) == 0 &&
            shortcut_infos[i].default_accel) {
            GtkWidget *row = create_shortcut_row (shortcut_infos[i].description,
                                                    shortcut_infos[i].default_accel);
            gtk_list_box_append (GTK_LIST_BOX (list_box), row);
        }
    }

    gtk_frame_set_child (GTK_FRAME (frame), list_box);

    return frame;
}

/**
 * @brief Show shortcuts window
 */
void gtkterm_shortcuts_show_window (GtkTermShortcuts *self,
                                     GtkWindow *parent) {
    g_return_if_fail (GTKTERM_IS_SHORTCUTS (self));

    // Create window using modern GTK4 approach
    GtkWidget *window = gtk_window_new ();
    gtk_window_set_title (GTK_WINDOW (window), "Keyboard Shortcuts");
    gtk_window_set_default_size (GTK_WINDOW (window), 600, 700);
    gtk_window_set_transient_for (GTK_WINDOW (window), parent);
    gtk_window_set_modal (GTK_WINDOW (window), TRUE);
    gtk_window_set_destroy_with_parent (GTK_WINDOW (window), TRUE);

    // Create header bar with close button
    GtkWidget *header = gtk_header_bar_new ();
    gtk_window_set_titlebar (GTK_WINDOW (window), header);

    // Create main content box
    GtkWidget *content = gtk_box_new (GTK_ORIENTATION_VERTICAL, 0);
    gtk_window_set_child (GTK_WINDOW (window), content);

    GtkWidget *scrolled = gtk_scrolled_window_new ();
    gtk_widget_set_vexpand (scrolled, TRUE);
    gtk_scrolled_window_set_policy (GTK_SCROLLED_WINDOW (scrolled),
                                     GTK_POLICY_NEVER,
                                     GTK_POLICY_AUTOMATIC);

    GtkWidget *box = gtk_box_new (GTK_ORIENTATION_VERTICAL, 0);

    // Add categories
    const char *categories[] = {
        "Application", "Tabs", "Connection", "File Transfer",
        "Recording", "Editing", "Serial Control", "Automation", "View",
        NULL
    };

    for (int i = 0; categories[i] != NULL; i++) {
        GtkWidget *category = create_shortcuts_category (categories[i]);
        gtk_box_append (GTK_BOX (box), category);
    }

    gtk_scrolled_window_set_child (GTK_SCROLLED_WINDOW (scrolled), box);
    gtk_box_append (GTK_BOX (content), scrolled);

    gtk_window_present (GTK_WINDOW (window));
}

/**
 * @brief Dispose
 */
static void gtkterm_shortcuts_dispose (GObject *object) {
    GtkTermShortcuts *self = GTKTERM_SHORTCUTS (object);

    for (int i = 0; i < GTKTERM_ACTION_COUNT; i++) {
        g_clear_pointer (&self->accelerators[i], g_free);
    }

    g_clear_object (&self->app);

    G_OBJECT_CLASS (gtkterm_shortcuts_parent_class)->dispose (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_shortcuts_class_init (GtkTermShortcutsClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_shortcuts_dispose;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_shortcuts_init (GtkTermShortcuts *self) {
    // Initialize accelerators array
    for (int i = 0; i < GTKTERM_ACTION_COUNT; i++) {
        self->accelerators[i] = NULL;
    }
}

/**
 * @brief Create shortcuts manager
 */
GtkTermShortcuts *gtkterm_shortcuts_new (GtkApplication *app) {
    g_return_val_if_fail (GTK_IS_APPLICATION (app), NULL);

    GtkTermShortcuts *self = g_object_new (GTKTERM_TYPE_SHORTCUTS, NULL);
    self->app = g_object_ref (app);

    // Apply default accelerators
    apply_accelerators (self);

    return self;
}
