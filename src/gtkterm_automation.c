/************************************************************************/
/* gtkterm_automation.c                                                 */
/* --------------------                                                 */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Automation and scripting framework implementation              */
/*                                                                      */
/************************************************************************/

#include "gtkterm_automation.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>
#include <ctype.h>

struct _GtkTermAutomation {
    GObject parent_instance;

    /* Script actions */
    GList *actions;
    GList *current_action;

    /* Execution state */
    GtkTermScriptStatus status;
    guint current_line;
    GString *receive_buffer;

    /* Callbacks */
    gboolean (*write_func)(const guint8 *, gsize, gpointer);
    gpointer write_user_data;
    gboolean (*control_func)(const char *, gboolean, gpointer);
    gpointer control_user_data;

    /* Timeout */
    guint timeout_id;
    guint delay_timeout_id;

    /* Variables (for simple state) */
    GHashTable *variables;
};

enum {
    SIGNAL_STARTED,
    SIGNAL_COMPLETED,
    SIGNAL_ERROR,
    SIGNAL_LINE_EXECUTED,
    N_SIGNALS
};

static guint signals[N_SIGNALS] = {0};

G_DEFINE_TYPE (GtkTermAutomation, gtkterm_automation, G_TYPE_OBJECT)

/**
 * @brief Create a new script action
 */
GtkTermScriptAction *gtkterm_script_action_new (GtkTermActionType type,
                                                 const char *parameter,
                                                 gint timeout_ms) {
    GtkTermScriptAction *action = g_new0 (GtkTermScriptAction, 1);
    action->type = type;
    action->parameter = g_strdup (parameter);
    action->timeout_ms = timeout_ms;
    action->expect_regex = NULL;

    return action;
}

/**
 * @brief Free a script action
 */
void gtkterm_script_action_free (GtkTermScriptAction *action) {
    if (!action)
        return;

    g_free (action->parameter);
    if (action->expect_regex)
        g_regex_unref (action->expect_regex);
    g_free (action);
}

/**
 * @brief Execute the next action
 */
static void execute_next_action (GtkTermAutomation *self);

/**
 * @brief Handle timeout
 */
static gboolean on_timeout (gpointer user_data) {
    GtkTermAutomation *self = GTKTERM_AUTOMATION (user_data);

    self->timeout_id = 0;
    self->status = GTKTERM_SCRIPT_STATUS_ERROR;

    g_signal_emit (self, signals[SIGNAL_ERROR], 0,
                   "Timeout waiting for expected data");

    return G_SOURCE_REMOVE;
}

/**
 * @brief Handle delay timeout
 */
static gboolean on_delay_timeout (gpointer user_data) {
    GtkTermAutomation *self = GTKTERM_AUTOMATION (user_data);

    self->delay_timeout_id = 0;

    // Continue with next action
    if (self->current_action) {
        self->current_action = self->current_action->next;
        execute_next_action (self);
    }

    return G_SOURCE_REMOVE;
}

/**
 * @brief Finish script execution
 */
static void finish_execution (GtkTermAutomation *self, gboolean success, const char *message) {
    self->status = success ? GTKTERM_SCRIPT_STATUS_COMPLETED : GTKTERM_SCRIPT_STATUS_ERROR;

    if (self->timeout_id) {
        g_source_remove (self->timeout_id);
        self->timeout_id = 0;
    }

    if (self->delay_timeout_id) {
        g_source_remove (self->delay_timeout_id);
        self->delay_timeout_id = 0;
    }

    if (success) {
        g_signal_emit (self, signals[SIGNAL_COMPLETED], 0);
    } else {
        g_signal_emit (self, signals[SIGNAL_ERROR], 0, message);
    }
}

/**
 * @brief Expand variables in string
 */
static char *expand_variables (GtkTermAutomation *self, const char *str) {
    if (!str || !strchr (str, '$'))
        return g_strdup (str);

    GString *result = g_string_new ("");
    const char *p = str;

    while (*p) {
        if (*p == '$' && *(p + 1) == '{') {
            // Variable reference: ${varname}
            const char *start = p + 2;
            const char *end = strchr (start, '}');

            if (end) {
                char *var_name = g_strndup (start, end - start);
                const char *var_value = g_hash_table_lookup (self->variables, var_name);

                if (var_value) {
                    g_string_append (result, var_value);
                } else {
                    g_string_append_printf (result, "${%s}", var_name);
                }

                g_free (var_name);
                p = end + 1;
                continue;
            }
        }

        g_string_append_c (result, *p);
        p++;
    }

    return g_string_free (result, FALSE);
}

/**
 * @brief Execute current action
 */
static void execute_next_action (GtkTermAutomation *self) {
    if (!self->current_action) {
        finish_execution (self, TRUE, "Script completed successfully");
        return;
    }

    GtkTermScriptAction *action = (GtkTermScriptAction *)self->current_action->data;
    self->current_line++;

    g_signal_emit (self, signals[SIGNAL_LINE_EXECUTED], 0, self->current_line);

    switch (action->type) {
        case GTKTERM_ACTION_SEND: {
            // Send data
            char *expanded = expand_variables (self, action->parameter);

            // Process escape sequences
            GString *data = g_string_new ("");
            for (const char *p = expanded; *p; p++) {
                if (*p == '\\' && *(p + 1)) {
                    p++;
                    switch (*p) {
                        case 'n': g_string_append_c (data, '\n'); break;
                        case 'r': g_string_append_c (data, '\r'); break;
                        case 't': g_string_append_c (data, '\t'); break;
                        case '\\': g_string_append_c (data, '\\'); break;
                        case 'x': {
                            // Hex byte: \xHH
                            if (isxdigit (*(p + 1)) && isxdigit (*(p + 2))) {
                                char hex[3] = {*(p + 1), *(p + 2), 0};
                                guint8 byte = (guint8)strtoul (hex, NULL, 16);
                                g_string_append_c (data, byte);
                                p += 2;
                            }
                            break;
                        }
                        default:
                            g_string_append_c (data, *p);
                            break;
                    }
                } else {
                    g_string_append_c (data, *p);
                }
            }

            if (self->write_func) {
                self->write_func ((const guint8 *)data->str, data->len, self->write_user_data);
            }

            g_string_free (data, TRUE);
            g_free (expanded);

            // Continue with next action
            self->current_action = self->current_action->next;
            execute_next_action (self);
            break;
        }

        case GTKTERM_ACTION_EXPECT: {
            // Wait for expected data
            GError *error = NULL;
            char *expanded = expand_variables (self, action->parameter);

            action->expect_regex = g_regex_new (expanded, 0, 0, &error);

            if (error) {
                finish_execution (self, FALSE, error->message);
                g_error_free (error);
                g_free (expanded);
                return;
            }

            g_free (expanded);

            self->status = GTKTERM_SCRIPT_STATUS_WAITING;

            // Set timeout
            if (action->timeout_ms > 0) {
                self->timeout_id = g_timeout_add (action->timeout_ms, on_timeout, self);
            }

            // Don't advance to next action - wait for data
            break;
        }

        case GTKTERM_ACTION_DELAY: {
            // Delay execution
            gint delay_ms = atoi (action->parameter);

            if (delay_ms > 0) {
                self->delay_timeout_id = g_timeout_add (delay_ms, on_delay_timeout, self);
            } else {
                self->current_action = self->current_action->next;
                execute_next_action (self);
            }
            break;
        }

        case GTKTERM_ACTION_SET_DTR:
        case GTKTERM_ACTION_SET_RTS: {
            // Set control signals
            if (self->control_func) {
                const char *signal_name = (action->type == GTKTERM_ACTION_SET_DTR) ? "DTR" : "RTS";
                gboolean value = (g_strcmp0 (action->parameter, "1") == 0 ||
                                  g_ascii_strcasecmp (action->parameter, "on") == 0 ||
                                  g_ascii_strcasecmp (action->parameter, "true") == 0);

                self->control_func (signal_name, value, self->control_user_data);
            }

            self->current_action = self->current_action->next;
            execute_next_action (self);
            break;
        }

        case GTKTERM_ACTION_BREAK: {
            // Send break signal
            if (self->control_func) {
                gint duration_ms = atoi (action->parameter);
                if (duration_ms <= 0)
                    duration_ms = 250;

                self->control_func ("BREAK", TRUE, self->control_user_data);

                // Schedule break release
                self->delay_timeout_id = g_timeout_add (duration_ms, on_delay_timeout, self);
            } else {
                self->current_action = self->current_action->next;
                execute_next_action (self);
            }
            break;
        }

        case GTKTERM_ACTION_LOG: {
            // Log message
            char *expanded = expand_variables (self, action->parameter);
            g_message ("Script: %s", expanded);
            g_free (expanded);

            self->current_action = self->current_action->next;
            execute_next_action (self);
            break;
        }

        case GTKTERM_ACTION_EXEC: {
            // Execute external command
            char *expanded = expand_variables (self, action->parameter);

            gchar *std_out = NULL;
            gchar *std_err = NULL;
            gint exit_status = 0;
            GError *error = NULL;

            gboolean success = g_spawn_command_line_sync (expanded,
                                                          &std_out,
                                                          &std_err,
                                                          &exit_status,
                                                          &error);

            if (!success) {
                finish_execution (self, FALSE, error->message);
                g_error_free (error);
                g_free (expanded);
                return;
            }

            // Store output in variable
            if (std_out && strlen (std_out) > 0) {
                g_hash_table_insert (self->variables,
                                    g_strdup ("EXEC_OUTPUT"),
                                    g_strdup (std_out));
            }

            g_free (std_out);
            g_free (std_err);
            g_free (expanded);

            self->current_action = self->current_action->next;
            execute_next_action (self);
            break;
        }

        case GTKTERM_ACTION_CONDITIONAL: {
            // Simple conditional: if ${var} == value
            // Format: "var == value"
            // For now, just continue
            self->current_action = self->current_action->next;
            execute_next_action (self);
            break;
        }

        default:
            self->current_action = self->current_action->next;
            execute_next_action (self);
            break;
    }
}

/**
 * @brief Parse script line
 */
static gboolean parse_script_line (GtkTermAutomation *self, const char *line, GError **error) {
    // Skip empty lines and comments
    if (!line || strlen (line) == 0 || line[0] == '#')
        return TRUE;

    // Trim whitespace
    char *trimmed = g_strstrip (g_strdup (line));

    if (strlen (trimmed) == 0) {
        g_free (trimmed);
        return TRUE;
    }

    // Parse command
    char **parts = g_strsplit (trimmed, " ", 2);
    g_free (trimmed);

    if (!parts[0]) {
        g_strfreev (parts);
        return TRUE;
    }

    const char *command = parts[0];
    const char *parameter = parts[1] ? parts[1] : "";

    GtkTermActionType type;
    gint timeout = 0;

    if (g_strcmp0 (command, "send") == 0) {
        type = GTKTERM_ACTION_SEND;
    } else if (g_strcmp0 (command, "expect") == 0) {
        type = GTKTERM_ACTION_EXPECT;
        timeout = 5000; // 5 second default timeout
    } else if (g_strcmp0 (command, "delay") == 0) {
        type = GTKTERM_ACTION_DELAY;
    } else if (g_strcmp0 (command, "set_dtr") == 0) {
        type = GTKTERM_ACTION_SET_DTR;
    } else if (g_strcmp0 (command, "set_rts") == 0) {
        type = GTKTERM_ACTION_SET_RTS;
    } else if (g_strcmp0 (command, "break") == 0) {
        type = GTKTERM_ACTION_BREAK;
    } else if (g_strcmp0 (command, "log") == 0) {
        type = GTKTERM_ACTION_LOG;
    } else if (g_strcmp0 (command, "exec") == 0) {
        type = GTKTERM_ACTION_EXEC;
    } else {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                     "Unknown command: %s", command);
        g_strfreev (parts);
        return FALSE;
    }

    gtkterm_automation_add_action (self, type, parameter, timeout);
    g_strfreev (parts);

    return TRUE;
}

/**
 * @brief Load script from file
 */
gboolean gtkterm_automation_load_script (GtkTermAutomation *self,
                                          const char *file_path,
                                          GError **error) {
    g_return_val_if_fail (GTKTERM_IS_AUTOMATION (self), FALSE);
    g_return_val_if_fail (file_path != NULL, FALSE);

    // Clear existing actions
    g_list_free_full (self->actions, (GDestroyNotify)gtkterm_script_action_free);
    self->actions = NULL;

    // Read file
    char *contents = NULL;
    gsize length = 0;

    if (!g_file_get_contents (file_path, &contents, &length, error)) {
        return FALSE;
    }

    // Parse lines
    char **lines = g_strsplit (contents, "\n", -1);
    g_free (contents);

    for (int i = 0; lines[i] != NULL; i++) {
        if (!parse_script_line (self, lines[i], error)) {
            g_strfreev (lines);
            return FALSE;
        }
    }

    g_strfreev (lines);
    return TRUE;
}

/**
 * @brief Add a script action
 */
void gtkterm_automation_add_action (GtkTermAutomation *self,
                                     GtkTermActionType type,
                                     const char *parameter,
                                     gint timeout_ms) {
    g_return_if_fail (GTKTERM_IS_AUTOMATION (self));

    GtkTermScriptAction *action = gtkterm_script_action_new (type, parameter, timeout_ms);
    self->actions = g_list_append (self->actions, action);
}

/**
 * @brief Start script execution
 */
gboolean gtkterm_automation_start (GtkTermAutomation *self, GError **error) {
    g_return_val_if_fail (GTKTERM_IS_AUTOMATION (self), FALSE);

    if (self->status == GTKTERM_SCRIPT_STATUS_RUNNING) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_BUSY, "Script already running");
        return FALSE;
    }

    if (!self->actions) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "No script loaded");
        return FALSE;
    }

    self->status = GTKTERM_SCRIPT_STATUS_RUNNING;
    self->current_line = 0;
    self->current_action = self->actions;
    g_string_truncate (self->receive_buffer, 0);

    g_signal_emit (self, signals[SIGNAL_STARTED], 0);

    execute_next_action (self);

    return TRUE;
}

/**
 * @brief Stop script execution
 */
void gtkterm_automation_stop (GtkTermAutomation *self) {
    g_return_if_fail (GTKTERM_IS_AUTOMATION (self));

    if (self->status == GTKTERM_SCRIPT_STATUS_RUNNING ||
        self->status == GTKTERM_SCRIPT_STATUS_WAITING) {
        finish_execution (self, FALSE, "Script cancelled by user");
    }
}

/**
 * @brief Process received data
 */
void gtkterm_automation_process_data (GtkTermAutomation *self,
                                       const guint8 *data,
                                       gsize length) {
    g_return_if_fail (GTKTERM_IS_AUTOMATION (self));

    if (self->status != GTKTERM_SCRIPT_STATUS_WAITING)
        return;

    // Append to receive buffer
    g_string_append_len (self->receive_buffer, (const char *)data, length);

    // Check if we're waiting for expected data
    if (!self->current_action)
        return;

    GtkTermScriptAction *action = (GtkTermScriptAction *)self->current_action->data;

    if (action->type == GTKTERM_ACTION_EXPECT && action->expect_regex) {
        // Try to match regex
        GMatchInfo *match_info = NULL;

        if (g_regex_match (action->expect_regex, self->receive_buffer->str, 0, &match_info)) {
            // Match found!
            if (self->timeout_id) {
                g_source_remove (self->timeout_id);
                self->timeout_id = 0;
            }

            // Extract matched groups as variables
            gint match_count = g_match_info_get_match_count (match_info);
            for (gint i = 0; i < match_count; i++) {
                char *match = g_match_info_fetch (match_info, i);
                if (match) {
                    char *var_name = g_strdup_printf ("MATCH_%d", i);
                    g_hash_table_insert (self->variables, var_name, match);
                }
            }

            g_match_info_free (match_info);

            // Clear buffer and continue
            g_string_truncate (self->receive_buffer, 0);
            self->status = GTKTERM_SCRIPT_STATUS_RUNNING;
            self->current_action = self->current_action->next;
            execute_next_action (self);
        } else {
            g_match_info_free (match_info);
        }
    }
}

/**
 * @brief Get current script status
 */
GtkTermScriptStatus gtkterm_automation_get_status (GtkTermAutomation *self) {
    g_return_val_if_fail (GTKTERM_IS_AUTOMATION (self), GTKTERM_SCRIPT_STATUS_IDLE);
    return self->status;
}

/**
 * @brief Get current line number
 */
guint gtkterm_automation_get_current_line (GtkTermAutomation *self) {
    g_return_val_if_fail (GTKTERM_IS_AUTOMATION (self), 0);
    return self->current_line;
}

/**
 * @brief Set write function
 */
void gtkterm_automation_set_write_func (GtkTermAutomation *self,
                                         gboolean (*write_func)(const guint8 *, gsize, gpointer),
                                         gpointer user_data) {
    g_return_if_fail (GTKTERM_IS_AUTOMATION (self));

    self->write_func = write_func;
    self->write_user_data = user_data;
}

/**
 * @brief Set control signals function
 */
void gtkterm_automation_set_control_func (GtkTermAutomation *self,
                                           gboolean (*control_func)(const char *, gboolean, gpointer),
                                           gpointer user_data) {
    g_return_if_fail (GTKTERM_IS_AUTOMATION (self));

    self->control_func = control_func;
    self->control_user_data = user_data;
}

/**
 * @brief Dispose
 */
static void gtkterm_automation_dispose (GObject *object) {
    GtkTermAutomation *self = GTKTERM_AUTOMATION (object);

    if (self->timeout_id) {
        g_source_remove (self->timeout_id);
        self->timeout_id = 0;
    }

    if (self->delay_timeout_id) {
        g_source_remove (self->delay_timeout_id);
        self->delay_timeout_id = 0;
    }

    G_OBJECT_CLASS (gtkterm_automation_parent_class)->dispose (object);
}

/**
 * @brief Finalize
 */
static void gtkterm_automation_finalize (GObject *object) {
    GtkTermAutomation *self = GTKTERM_AUTOMATION (object);

    g_list_free_full (self->actions, (GDestroyNotify)gtkterm_script_action_free);
    g_string_free (self->receive_buffer, TRUE);
    g_hash_table_destroy (self->variables);

    G_OBJECT_CLASS (gtkterm_automation_parent_class)->finalize (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_automation_class_init (GtkTermAutomationClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_automation_dispose;
    object_class->finalize = gtkterm_automation_finalize;

    /**
     * GtkTermAutomation::started:
     */
    signals[SIGNAL_STARTED] = g_signal_new (
        "started",
        G_TYPE_FROM_CLASS (klass),
        G_SIGNAL_RUN_LAST,
        0, NULL, NULL, NULL,
        G_TYPE_NONE, 0
    );

    /**
     * GtkTermAutomation::completed:
     */
    signals[SIGNAL_COMPLETED] = g_signal_new (
        "completed",
        G_TYPE_FROM_CLASS (klass),
        G_SIGNAL_RUN_LAST,
        0, NULL, NULL, NULL,
        G_TYPE_NONE, 0
    );

    /**
     * GtkTermAutomation::error:
     */
    signals[SIGNAL_ERROR] = g_signal_new (
        "error",
        G_TYPE_FROM_CLASS (klass),
        G_SIGNAL_RUN_LAST,
        0, NULL, NULL, NULL,
        G_TYPE_NONE, 1, G_TYPE_STRING
    );

    /**
     * GtkTermAutomation::line-executed:
     */
    signals[SIGNAL_LINE_EXECUTED] = g_signal_new (
        "line-executed",
        G_TYPE_FROM_CLASS (klass),
        G_SIGNAL_RUN_LAST,
        0, NULL, NULL, NULL,
        G_TYPE_NONE, 1, G_TYPE_UINT
    );
}

/**
 * @brief Instance initialization
 */
static void gtkterm_automation_init (GtkTermAutomation *self) {
    self->actions = NULL;
    self->current_action = NULL;
    self->status = GTKTERM_SCRIPT_STATUS_IDLE;
    self->current_line = 0;
    self->receive_buffer = g_string_new ("");
    self->write_func = NULL;
    self->write_user_data = NULL;
    self->control_func = NULL;
    self->control_user_data = NULL;
    self->timeout_id = 0;
    self->delay_timeout_id = 0;
    self->variables = g_hash_table_new_full (g_str_hash, g_str_equal, g_free, g_free);
}

/**
 * @brief Create a new automation object
 */
GtkTermAutomation *gtkterm_automation_new (void) {
    return g_object_new (GTKTERM_TYPE_AUTOMATION, NULL);
}
