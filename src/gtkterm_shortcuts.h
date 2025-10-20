/************************************************************************/
/* gtkterm_shortcuts.h                                                  */
/* -------------------                                                  */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Keyboard shortcuts management                                  */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_SHORTCUTS_H
#define GTKTERM_SHORTCUTS_H

#include <gtk/gtk.h>

G_BEGIN_DECLS

#define GTKTERM_TYPE_SHORTCUTS gtkterm_shortcuts_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermShortcuts, gtkterm_shortcuts, GTKTERM, SHORTCUTS, GObject)

/**
 * @brief Standard application actions
 */
typedef enum {
    GTKTERM_ACTION_QUIT,
    GTKTERM_ACTION_NEW_TAB,
    GTKTERM_ACTION_CLOSE_TAB,
    GTKTERM_ACTION_NEXT_TAB,
    GTKTERM_ACTION_PREV_TAB,
    GTKTERM_ACTION_CONNECT,
    GTKTERM_ACTION_DISCONNECT,
    GTKTERM_ACTION_RECONNECT,
    GTKTERM_ACTION_SEND_FILE,
    GTKTERM_ACTION_RECEIVE_FILE,
    GTKTERM_ACTION_RECORD_START,
    GTKTERM_ACTION_RECORD_STOP,
    GTKTERM_ACTION_CLEAR_BUFFER,
    GTKTERM_ACTION_COPY,
    GTKTERM_ACTION_PASTE,
    GTKTERM_ACTION_SELECT_ALL,
    GTKTERM_ACTION_FIND,
    GTKTERM_ACTION_PREFERENCES,
    GTKTERM_ACTION_SEND_BREAK,
    GTKTERM_ACTION_TOGGLE_DTR,
    GTKTERM_ACTION_TOGGLE_RTS,
    GTKTERM_ACTION_RUN_SCRIPT,
    GTKTERM_ACTION_TOGGLE_HEX,
    GTKTERM_ACTION_TOGGLE_TIMESTAMP,
    GTKTERM_ACTION_INCREASE_FONT,
    GTKTERM_ACTION_DECREASE_FONT,
    GTKTERM_ACTION_RESET_FONT,
    GTKTERM_ACTION_FULLSCREEN,
    GTKTERM_ACTION_HELP,
    GTKTERM_ACTION_ABOUT,
    GTKTERM_ACTION_COUNT  /* Keep last */
} GtkTermAction;

/**
 * @brief Shortcut information
 */
typedef struct {
    GtkTermAction action;
    const char *name;
    const char *description;
    const char *default_accel;
    const char *category;
} GtkTermShortcutInfo;

/**
 * @brief Create shortcuts manager
 */
GtkTermShortcuts *gtkterm_shortcuts_new (GtkApplication *app);

/**
 * @brief Get shortcut info
 */
const GtkTermShortcutInfo *gtkterm_shortcuts_get_info (GtkTermAction action);

/**
 * @brief Get all shortcut infos
 */
const GtkTermShortcutInfo *gtkterm_shortcuts_get_all_infos (void);

/**
 * @brief Set custom accelerator for action
 */
gboolean gtkterm_shortcuts_set_accelerator (GtkTermShortcuts *self,
                                             GtkTermAction action,
                                             const char *accel,
                                             GError **error);

/**
 * @brief Get accelerator for action
 */
const char *gtkterm_shortcuts_get_accelerator (GtkTermShortcuts *self,
                                                 GtkTermAction action);

/**
 * @brief Reset action to default accelerator
 */
void gtkterm_shortcuts_reset_to_default (GtkTermShortcuts *self,
                                          GtkTermAction action);

/**
 * @brief Reset all to defaults
 */
void gtkterm_shortcuts_reset_all (GtkTermShortcuts *self);

/**
 * @brief Load shortcuts from configuration
 */
gboolean gtkterm_shortcuts_load (GtkTermShortcuts *self,
                                  const char *config_file,
                                  GError **error);

/**
 * @brief Save shortcuts to configuration
 */
gboolean gtkterm_shortcuts_save (GtkTermShortcuts *self,
                                  const char *config_file,
                                  GError **error);

/**
 * @brief Show shortcuts window
 */
void gtkterm_shortcuts_show_window (GtkTermShortcuts *self,
                                     GtkWindow *parent);

G_END_DECLS

#endif // GTKTERM_SHORTCUTS_H
