/************************************************************************/
/* gtkterm_macro.c                                                      */
/* ---------------                                                      */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Enhanced macro system implementation                            */
/*                                                                      */
/************************************************************************/

#include <glib.h>
#include <gio/gio.h>
#include <json-glib/json-glib.h>
#include <string.h>
#include "gtkterm_macro.h"

/* Macro Structure */
struct _GtkTermMacro {
    GObject parent_instance;
    char *name;
    char *shortcut;
    GPtrArray *commands;  /**< Array of GtkTermMacroCommand */
};

/* Macro Manager Structure */
struct _GtkTermMacroManager {
    GObject parent_instance;
    GHashTable *macros;        /**< Hash table of name -> GtkTermMacro */
    GHashTable *shortcuts;     /**< Hash table of shortcut -> GtkTermMacro */

    /* Recording state */
    gboolean is_recording;
    GtkTermMacro *recording_macro;
    gint64 last_command_time;

    /* Playback state */
    gboolean is_playing;
    guint playback_source_id;
    gpointer playback_ctx;  /**< Current playback context */
};

G_DEFINE_TYPE (GtkTermMacro, gtkterm_macro, G_TYPE_OBJECT)
G_DEFINE_TYPE (GtkTermMacroManager, gtkterm_macro_manager, G_TYPE_OBJECT)

/**
 * @brief Free a macro command
 */
static void gtkterm_macro_command_free (GtkTermMacroCommand *cmd) {
    if (cmd) {
        g_free (cmd->data);
        g_free (cmd);
    }
}

/**
 * @brief Macro class initialization
 */
static void gtkterm_macro_class_init (GtkTermMacroClass *klass) {
    (void)klass;  // Unused parameter
    // Add finalize if needed
}

/**
 * @brief Macro instance initialization
 */
static void gtkterm_macro_init (GtkTermMacro *macro) {
    macro->name = NULL;
    macro->shortcut = NULL;
    macro->commands = g_ptr_array_new_with_free_func ((GDestroyNotify)gtkterm_macro_command_free);
}

/**
 * @brief Create a new macro
 */
GtkTermMacro *gtkterm_macro_new (const char *name, const char *shortcut) {
    GtkTermMacro *macro = g_object_new (GTKTERM_TYPE_MACRO, NULL);
    macro->name = g_strdup (name);
    macro->shortcut = g_strdup (shortcut);
    return macro;
}

/**
 * @brief Free a macro
 */
void gtkterm_macro_free (GtkTermMacro *macro) {
    if (macro) {
        g_free (macro->name);
        g_free (macro->shortcut);
        g_ptr_array_free (macro->commands, TRUE);
        g_object_unref (macro);
    }
}

/* Getters/Setters */
const char *gtkterm_macro_get_name (GtkTermMacro *macro) {
    return macro->name;
}

void gtkterm_macro_set_name (GtkTermMacro *macro, const char *name) {
    g_free (macro->name);
    macro->name = g_strdup (name);
}

const char *gtkterm_macro_get_shortcut (GtkTermMacro *macro) {
    return macro->shortcut;
}

void gtkterm_macro_set_shortcut (GtkTermMacro *macro, const char *shortcut) {
    g_free (macro->shortcut);
    macro->shortcut = g_strdup (shortcut);
}

/**
 * @brief Add a command to the macro
 */
void gtkterm_macro_add_command (GtkTermMacro *macro,
                                GtkTermMacroCommandType type,
                                const char *data,
                                guint delay_ms) {
    GtkTermMacroCommand *cmd = g_new0 (GtkTermMacroCommand, 1);
    cmd->type = type;
    cmd->data = g_strdup (data);
    cmd->delay_ms = delay_ms;
    g_ptr_array_add (macro->commands, cmd);
}

/**
 * @brief Clear all commands
 */
void gtkterm_macro_clear_commands (GtkTermMacro *macro) {
    g_ptr_array_remove_range (macro->commands, 0, macro->commands->len);
}

/**
 * @brief Get commands array
 */
GPtrArray *gtkterm_macro_get_commands (GtkTermMacro *macro) {
    return macro->commands;
}

/* ===== Macro Manager ===== */

/**
 * @brief Macro Manager dispose
 */
static void gtkterm_macro_manager_dispose (GObject *object) {
    GtkTermMacroManager *manager = GTKTERM_MACRO_MANAGER (object);

    /* Stop any active playback */
    if (manager->is_playing) {
        gtkterm_macro_manager_stop_playback (manager);
    }

    /* Clean up hash tables */
    if (manager->macros) {
        g_hash_table_destroy (manager->macros);
        manager->macros = NULL;
    }
    if (manager->shortcuts) {
        g_hash_table_destroy (manager->shortcuts);
        manager->shortcuts = NULL;
    }

    G_OBJECT_CLASS (gtkterm_macro_manager_parent_class)->dispose (object);
}

/**
 * @brief Macro Manager class initialization
 */
static void gtkterm_macro_manager_class_init (GtkTermMacroManagerClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);
    object_class->dispose = gtkterm_macro_manager_dispose;
}

/**
 * @brief Macro Manager instance initialization
 */
static void gtkterm_macro_manager_init (GtkTermMacroManager *manager) {
    manager->macros = g_hash_table_new_full (g_str_hash, g_str_equal,
                                             g_free, (GDestroyNotify)gtkterm_macro_free);
    manager->shortcuts = g_hash_table_new (g_str_hash, g_str_equal);
    manager->is_recording = FALSE;
    manager->recording_macro = NULL;
    manager->last_command_time = 0;
    manager->is_playing = FALSE;
    manager->playback_source_id = 0;
    manager->playback_ctx = NULL;
}

/**
 * @brief Create a new macro manager
 */
GtkTermMacroManager *gtkterm_macro_manager_new (void) {
    return g_object_new (GTKTERM_TYPE_MACRO_MANAGER, NULL);
}

/**
 * @brief Add a macro to the manager
 */
void gtkterm_macro_manager_add_macro (GtkTermMacroManager *manager, GtkTermMacro *macro) {
    const char *name = gtkterm_macro_get_name (macro);
    const char *shortcut = gtkterm_macro_get_shortcut (macro);

    g_hash_table_insert (manager->macros, g_strdup (name), macro);

    if (shortcut) {
        g_hash_table_insert (manager->shortcuts, (gpointer)shortcut, macro);
    }
}

/**
 * @brief Remove a macro
 */
void gtkterm_macro_manager_remove_macro (GtkTermMacroManager *manager, const char *name) {
    GtkTermMacro *macro = g_hash_table_lookup (manager->macros, name);
    if (macro) {
        const char *shortcut = gtkterm_macro_get_shortcut (macro);
        if (shortcut) {
            g_hash_table_remove (manager->shortcuts, shortcut);
        }
        g_hash_table_remove (manager->macros, name);
    }
}

/**
 * @brief Get macro by name
 */
GtkTermMacro *gtkterm_macro_manager_get_macro (GtkTermMacroManager *manager, const char *name) {
    return g_hash_table_lookup (manager->macros, name);
}

/**
 * @brief Get macro by shortcut
 */
GtkTermMacro *gtkterm_macro_manager_get_macro_by_shortcut (GtkTermMacroManager *manager, const char *shortcut) {
    return g_hash_table_lookup (manager->shortcuts, shortcut);
}

/**
 * @brief Get all macros
 */
GPtrArray *gtkterm_macro_manager_get_all_macros (GtkTermMacroManager *manager) {
    GPtrArray *macros = g_ptr_array_new ();
    GHashTableIter iter;
    gpointer key, value;

    g_hash_table_iter_init (&iter, manager->macros);
    while (g_hash_table_iter_next (&iter, &key, &value)) {
        g_ptr_array_add (macros, value);
    }

    return macros;
}

/* ===== Recording ===== */

/**
 * @brief Start recording a macro
 */
void gtkterm_macro_manager_start_recording (GtkTermMacroManager *manager, const char *name) {
    if (manager->is_recording) {
        gtkterm_macro_manager_stop_recording (manager);
    }

    manager->recording_macro = gtkterm_macro_new (name, NULL);
    manager->is_recording = TRUE;
    manager->last_command_time = g_get_monotonic_time ();
}

/**
 * @brief Stop recording and save macro
 */
void gtkterm_macro_manager_stop_recording (GtkTermMacroManager *manager) {
    if (manager->is_recording && manager->recording_macro) {
        gtkterm_macro_manager_add_macro (manager, manager->recording_macro);
        manager->recording_macro = NULL;
    }
    manager->is_recording = FALSE;
}

/**
 * @brief Check if recording
 */
gboolean gtkterm_macro_manager_is_recording (GtkTermMacroManager *manager) {
    return manager->is_recording;
}

/**
 * @brief Record a command
 */
void gtkterm_macro_manager_record_command (GtkTermMacroManager *manager,
                                           const char *data,
                                           guint delay_ms) {
    if (!manager->is_recording || !manager->recording_macro) {
        return;
    }

    // Calculate delay since last command
    gint64 now = g_get_monotonic_time ();
    guint actual_delay = (now - manager->last_command_time) / 1000; // Convert to ms
    manager->last_command_time = now;

    gtkterm_macro_add_command (manager->recording_macro,
                               GTKTERM_MACRO_CMD_SEND,
                               data,
                               actual_delay);
}

/* ===== Playback ===== */

typedef struct {
    GtkTermMacroManager *manager;
    GtkTermMacro *macro;
    guint current_index;
    GtkTermMacroSendFunc send_func;
    gpointer user_data;
} PlaybackContext;

static gboolean playback_next_command (gpointer data) {
    PlaybackContext *ctx = (PlaybackContext *)data;
    GPtrArray *commands = gtkterm_macro_get_commands (ctx->macro);

    if (ctx->current_index >= commands->len) {
        // Playback complete
        ctx->manager->is_playing = FALSE;
        ctx->manager->playback_source_id = 0;
        ctx->manager->playback_ctx = NULL;
        g_free (ctx);
        return G_SOURCE_REMOVE;
    }

    GtkTermMacroCommand *cmd = g_ptr_array_index (commands, ctx->current_index);

    if (cmd->type == GTKTERM_MACRO_CMD_SEND) {
        // Send data
        ctx->send_func (cmd->data, strlen (cmd->data), ctx->user_data);
    }

    ctx->current_index++;

    // Schedule next command
    if (ctx->current_index < commands->len) {
        GtkTermMacroCommand *next_cmd = g_ptr_array_index (commands, ctx->current_index);
        ctx->manager->playback_source_id = g_timeout_add (next_cmd->delay_ms,
                                                          playback_next_command,
                                                          ctx);
        return G_SOURCE_REMOVE;
    }

    return G_SOURCE_REMOVE;
}

/**
 * @brief Play a macro
 */
void gtkterm_macro_manager_play_macro (GtkTermMacroManager *manager,
                                       const char *name,
                                       GtkTermMacroSendFunc send_func,
                                       gpointer user_data) {
    GtkTermMacro *macro = gtkterm_macro_manager_get_macro (manager, name);
    if (!macro || manager->is_playing) {
        return;
    }

    PlaybackContext *ctx = g_new0 (PlaybackContext, 1);
    ctx->manager = manager;
    ctx->macro = macro;
    ctx->current_index = 0;
    ctx->send_func = send_func;
    ctx->user_data = user_data;

    manager->is_playing = TRUE;
    manager->playback_ctx = ctx;

    // Start playback
    playback_next_command (ctx);
}

/**
 * @brief Stop playback
 */
void gtkterm_macro_manager_stop_playback (GtkTermMacroManager *manager) {
    if (manager->playback_source_id > 0) {
        g_source_remove (manager->playback_source_id);
        manager->playback_source_id = 0;
    }
    if (manager->playback_ctx) {
        g_free (manager->playback_ctx);
        manager->playback_ctx = NULL;
    }
    manager->is_playing = FALSE;
}

/**
 * @brief Check if playing
 */
gboolean gtkterm_macro_manager_is_playing (GtkTermMacroManager *manager) {
    return manager->is_playing;
}

/* ===== Save/Load ===== */

/**
 * @brief Save macros to file (JSON format)
 */
gboolean gtkterm_macro_manager_save_to_file (GtkTermMacroManager *manager,
                                             const char *filename,
                                             GError **error) {
    GString *json = g_string_new ("{\n  \"macros\": [\n");

    GPtrArray *macros = gtkterm_macro_manager_get_all_macros (manager);

    for (guint i = 0; i < macros->len; i++) {
        GtkTermMacro *macro = g_ptr_array_index (macros, i);
        const char *name = gtkterm_macro_get_name (macro);
        const char *shortcut = gtkterm_macro_get_shortcut (macro);
        GPtrArray *commands = gtkterm_macro_get_commands (macro);

        g_string_append_printf (json, "    {\n");
        g_string_append_printf (json, "      \"name\": \"%s\",\n", name);
        g_string_append_printf (json, "      \"shortcut\": \"%s\",\n", shortcut ? shortcut : "");
        g_string_append_printf (json, "      \"commands\": [\n");

        for (guint j = 0; j < commands->len; j++) {
            GtkTermMacroCommand *cmd = g_ptr_array_index (commands, j);

            // Escape data string for JSON
            char *escaped = g_strescape (cmd->data, NULL);

            g_string_append_printf (json, "        {\"type\": %d, \"data\": \"%s\", \"delay\": %u}",
                                   cmd->type, escaped, cmd->delay_ms);

            if (j < commands->len - 1) {
                g_string_append (json, ",\n");
            } else {
                g_string_append (json, "\n");
            }

            g_free (escaped);
        }

        g_string_append_printf (json, "      ]\n");
        g_string_append_printf (json, "    }");

        if (i < macros->len - 1) {
            g_string_append (json, ",\n");
        } else {
            g_string_append (json, "\n");
        }
    }

    g_string_append (json, "  ]\n}\n");

    g_ptr_array_free (macros, TRUE);

    // Write to file
    gboolean result = g_file_set_contents (filename, json->str, json->len, error);
    g_string_free (json, TRUE);

    return result;
}

/**
 * @brief Load macros from file (JSON format)
 */
gboolean gtkterm_macro_manager_load_from_file (GtkTermMacroManager *manager,
                                               const char *filename,
                                               GError **error) {
    JsonParser *parser = NULL;
    JsonNode *root = NULL;
    JsonObject *root_obj = NULL;
    JsonArray *macros_array = NULL;
    gboolean success = FALSE;

    /* Create JSON parser */
    parser = json_parser_new ();

    /* Load and parse the JSON file */
    if (!json_parser_load_from_file (parser, filename, error)) {
        g_object_unref (parser);
        return FALSE;
    }

    /* Get the root node */
    root = json_parser_get_root (parser);
    if (!JSON_NODE_HOLDS_OBJECT (root)) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                     "Invalid JSON format: root is not an object");
        goto cleanup;
    }

    root_obj = json_node_get_object (root);
    if (!json_object_has_member (root_obj, "macros")) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                     "Invalid JSON format: missing 'macros' array");
        goto cleanup;
    }

    macros_array = json_object_get_array_member (root_obj, "macros");
    if (!macros_array) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                     "Invalid JSON format: 'macros' is not an array");
        goto cleanup;
    }

    /* Clear existing macros */
    g_hash_table_remove_all (manager->macros);
    g_hash_table_remove_all (manager->shortcuts);

    /* Parse each macro */
    guint n_macros = json_array_get_length (macros_array);
    for (guint i = 0; i < n_macros; i++) {
        JsonNode *macro_node = json_array_get_element (macros_array, i);
        if (!JSON_NODE_HOLDS_OBJECT (macro_node)) {
            continue;
        }

        JsonObject *macro_obj = json_node_get_object (macro_node);

        /* Extract macro properties */
        const char *name = json_object_get_string_member (macro_obj, "name");
        const char *shortcut = json_object_get_string_member (macro_obj, "shortcut");

        if (!name || strlen (name) == 0) {
            continue;
        }

        /* Create new macro */
        GtkTermMacro *macro = gtkterm_macro_new (name, shortcut);

        /* Parse commands array */
        if (json_object_has_member (macro_obj, "commands")) {
            JsonArray *commands_array = json_object_get_array_member (macro_obj, "commands");
            guint n_commands = json_array_get_length (commands_array);

            for (guint j = 0; j < n_commands; j++) {
                JsonNode *cmd_node = json_array_get_element (commands_array, j);
                if (!JSON_NODE_HOLDS_OBJECT (cmd_node)) {
                    continue;
                }

                JsonObject *cmd_obj = json_node_get_object (cmd_node);

                /* Extract command properties */
                gint64 type = json_object_get_int_member (cmd_obj, "type");
                const char *data = json_object_get_string_member (cmd_obj, "data");
                gint64 delay = json_object_get_int_member (cmd_obj, "delay");

                /* Unescape the data string */
                char *unescaped = g_strcompress (data);

                /* Add command to macro */
                gtkterm_macro_add_command (macro,
                                          (GtkTermMacroCommandType)type,
                                          unescaped,
                                          (guint)delay);

                g_free (unescaped);
            }
        }

        /* Add macro to manager */
        gtkterm_macro_manager_add_macro (manager, macro);
    }

    success = TRUE;

cleanup:
    if (parser) {
        g_object_unref (parser);
    }

    return success;
}
