/************************************************************************/
/* gtkterm_automation.h                                                 */
/* --------------------                                                 */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Automation and scripting framework                             */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_AUTOMATION_H
#define GTKTERM_AUTOMATION_H

#include <glib-object.h>
#include <gio/gio.h>

G_BEGIN_DECLS

/**
 * @brief Script action types
 */
typedef enum {
    GTKTERM_ACTION_SEND,           /**< Send data */
    GTKTERM_ACTION_EXPECT,         /**< Wait for expected data */
    GTKTERM_ACTION_DELAY,          /**< Delay execution */
    GTKTERM_ACTION_SET_DTR,        /**< Set DTR signal */
    GTKTERM_ACTION_SET_RTS,        /**< Set RTS signal */
    GTKTERM_ACTION_BREAK,          /**< Send break signal */
    GTKTERM_ACTION_LOG,            /**< Log a message */
    GTKTERM_ACTION_EXEC,           /**< Execute external command */
    GTKTERM_ACTION_CONDITIONAL     /**< Conditional branch */
} GtkTermActionType;

/**
 * @brief Script execution status
 */
typedef enum {
    GTKTERM_SCRIPT_STATUS_IDLE,
    GTKTERM_SCRIPT_STATUS_RUNNING,
    GTKTERM_SCRIPT_STATUS_WAITING,
    GTKTERM_SCRIPT_STATUS_COMPLETED,
    GTKTERM_SCRIPT_STATUS_ERROR,
    GTKTERM_SCRIPT_STATUS_CANCELLED
} GtkTermScriptStatus;

/**
 * @brief Script action
 */
typedef struct {
    GtkTermActionType type;
    char *parameter;
    gint timeout_ms;
    GRegex *expect_regex;
} GtkTermScriptAction;

#define GTKTERM_TYPE_AUTOMATION gtkterm_automation_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermAutomation, gtkterm_automation, GTKTERM, AUTOMATION, GObject)

/**
 * @brief Create a new automation object
 */
GtkTermAutomation *gtkterm_automation_new (void);

/**
 * @brief Load script from file
 */
gboolean gtkterm_automation_load_script (GtkTermAutomation *self,
                                          const char *file_path,
                                          GError **error);

/**
 * @brief Add a script action
 */
void gtkterm_automation_add_action (GtkTermAutomation *self,
                                     GtkTermActionType type,
                                     const char *parameter,
                                     gint timeout_ms);

/**
 * @brief Start script execution
 */
gboolean gtkterm_automation_start (GtkTermAutomation *self,
                                    GError **error);

/**
 * @brief Stop script execution
 */
void gtkterm_automation_stop (GtkTermAutomation *self);

/**
 * @brief Process received data (for expect matching)
 */
void gtkterm_automation_process_data (GtkTermAutomation *self,
                                       const guint8 *data,
                                       gsize length);

/**
 * @brief Get current script status
 */
GtkTermScriptStatus gtkterm_automation_get_status (GtkTermAutomation *self);

/**
 * @brief Get current line number
 */
guint gtkterm_automation_get_current_line (GtkTermAutomation *self);

/**
 * @brief Set write function for sending data
 */
void gtkterm_automation_set_write_func (GtkTermAutomation *self,
                                         gboolean (*write_func)(const guint8 *, gsize, gpointer),
                                         gpointer user_data);

/**
 * @brief Set control signals function
 */
void gtkterm_automation_set_control_func (GtkTermAutomation *self,
                                           gboolean (*control_func)(const char *, gboolean, gpointer),
                                           gpointer user_data);

/* Helper functions for creating actions */
GtkTermScriptAction *gtkterm_script_action_new (GtkTermActionType type,
                                                 const char *parameter,
                                                 gint timeout_ms);

void gtkterm_script_action_free (GtkTermScriptAction *action);

G_END_DECLS

#endif // GTKTERM_AUTOMATION_H
