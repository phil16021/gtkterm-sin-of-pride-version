/************************************************************************/
/* gtkterm_macro.h                                                      */
/* ---------------                                                      */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Enhanced macro system for GTKTerm with recording/playback      */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_MACRO_H
#define GTKTERM_MACRO_H

#include <glib-object.h>
#include <gtk/gtk.h>

G_BEGIN_DECLS

#define GTKTERM_TYPE_MACRO gtkterm_macro_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermMacro, gtkterm_macro, GTKTERM, MACRO, GObject)

#define GTKTERM_TYPE_MACRO_MANAGER gtkterm_macro_manager_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermMacroManager, gtkterm_macro_manager, GTKTERM, MACRO_MANAGER, GObject)

/**
 * @brief Macro command types
 */
typedef enum {
    GTKTERM_MACRO_CMD_SEND,      /**< Send data to serial port */
    GTKTERM_MACRO_CMD_WAIT,      /**< Wait for milliseconds */
    GTKTERM_MACRO_CMD_EXPECT     /**< Wait for expected response (optional) */
} GtkTermMacroCommandType;

/**
 * @brief Single macro command
 */
typedef struct _GtkTermMacroCommand {
    GtkTermMacroCommandType type;
    char *data;                   /**< Data to send or pattern to expect */
    guint delay_ms;               /**< Delay in milliseconds */
} GtkTermMacroCommand;

/* Macro Management */
GtkTermMacro *gtkterm_macro_new (const char *name, const char *shortcut);
void gtkterm_macro_free (GtkTermMacro *macro);

const char *gtkterm_macro_get_name (GtkTermMacro *macro);
void gtkterm_macro_set_name (GtkTermMacro *macro, const char *name);

const char *gtkterm_macro_get_shortcut (GtkTermMacro *macro);
void gtkterm_macro_set_shortcut (GtkTermMacro *macro, const char *shortcut);

void gtkterm_macro_add_command (GtkTermMacro *macro,
                                GtkTermMacroCommandType type,
                                const char *data,
                                guint delay_ms);

void gtkterm_macro_clear_commands (GtkTermMacro *macro);
GPtrArray *gtkterm_macro_get_commands (GtkTermMacro *macro);

/* Macro Manager */
GtkTermMacroManager *gtkterm_macro_manager_new (void);

void gtkterm_macro_manager_add_macro (GtkTermMacroManager *manager, GtkTermMacro *macro);
void gtkterm_macro_manager_remove_macro (GtkTermMacroManager *manager, const char *name);
GtkTermMacro *gtkterm_macro_manager_get_macro (GtkTermMacroManager *manager, const char *name);
GtkTermMacro *gtkterm_macro_manager_get_macro_by_shortcut (GtkTermMacroManager *manager, const char *shortcut);
GPtrArray *gtkterm_macro_manager_get_all_macros (GtkTermMacroManager *manager);

/* Recording */
void gtkterm_macro_manager_start_recording (GtkTermMacroManager *manager, const char *name);
void gtkterm_macro_manager_stop_recording (GtkTermMacroManager *manager);
gboolean gtkterm_macro_manager_is_recording (GtkTermMacroManager *manager);
void gtkterm_macro_manager_record_command (GtkTermMacroManager *manager,
                                           const char *data,
                                           guint delay_ms);

/* Playback */
typedef void (*GtkTermMacroSendFunc) (const char *data, gsize length, gpointer user_data);

void gtkterm_macro_manager_play_macro (GtkTermMacroManager *manager,
                                       const char *name,
                                       GtkTermMacroSendFunc send_func,
                                       gpointer user_data);

void gtkterm_macro_manager_stop_playback (GtkTermMacroManager *manager);
gboolean gtkterm_macro_manager_is_playing (GtkTermMacroManager *manager);

/* Save/Load */
gboolean gtkterm_macro_manager_save_to_file (GtkTermMacroManager *manager,
                                             const char *filename,
                                             GError **error);

gboolean gtkterm_macro_manager_load_from_file (GtkTermMacroManager *manager,
                                               const char *filename,
                                               GError **error);

G_END_DECLS

#endif // GTKTERM_MACRO_H
