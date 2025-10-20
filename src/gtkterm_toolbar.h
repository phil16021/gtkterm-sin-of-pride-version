/************************************************************************/
/* gtkterm_toolbar.h                                                    */
/* -----------------                                                    */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Quick access toolbar for common actions                        */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_TOOLBAR_H
#define GTKTERM_TOOLBAR_H

#include <gtk/gtk.h>

G_BEGIN_DECLS

#define GTKTERM_TYPE_TOOLBAR gtkterm_toolbar_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermToolbar, gtkterm_toolbar, GTKTERM, TOOLBAR, GtkBox)

/**
 * @brief Toolbar action signals
 */
typedef enum {
    GTKTERM_TOOLBAR_ACTION_CONNECT,
    GTKTERM_TOOLBAR_ACTION_DISCONNECT,
    GTKTERM_TOOLBAR_ACTION_SEND_FILE,
    GTKTERM_TOOLBAR_ACTION_RECEIVE_FILE,
    GTKTERM_TOOLBAR_ACTION_RECORD,
    GTKTERM_TOOLBAR_ACTION_CLEAR,
    GTKTERM_TOOLBAR_ACTION_SEND_BREAK,
    GTKTERM_TOOLBAR_ACTION_TOGGLE_DTR,
    GTKTERM_TOOLBAR_ACTION_TOGGLE_RTS,
    GTKTERM_TOOLBAR_ACTION_RUN_SCRIPT,
    GTKTERM_TOOLBAR_ACTION_CONFIGURE
} GtkTermToolbarAction;

/**
 * @brief Create a new toolbar
 */
GtkWidget *gtkterm_toolbar_new (void);

/**
 * @brief Set connection state
 */
void gtkterm_toolbar_set_connected (GtkTermToolbar *self, gboolean connected);

/**
 * @brief Set recording state
 */
void gtkterm_toolbar_set_recording (GtkTermToolbar *self, gboolean recording);

/**
 * @brief Set DTR state
 */
void gtkterm_toolbar_set_dtr (GtkTermToolbar *self, gboolean active);

/**
 * @brief Set RTS state
 */
void gtkterm_toolbar_set_rts (GtkTermToolbar *self, gboolean active);

/**
 * @brief Enable/disable toolbar buttons
 */
void gtkterm_toolbar_set_sensitive (GtkTermToolbar *self, gboolean sensitive);

/**
 * @brief Set available protocols for decoder selection
 */
void gtkterm_toolbar_set_protocols (GtkTermToolbar *self, const char **protocol_names);

/**
 * @brief Get selected protocol
 */
const char *gtkterm_toolbar_get_selected_protocol (GtkTermToolbar *self);

G_END_DECLS

#endif // GTKTERM_TOOLBAR_H
