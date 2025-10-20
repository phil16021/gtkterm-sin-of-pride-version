/************************************************************************/
/* gtkterm_logging.c                                                    */
/* -----------------                                                    */
/*           GTKTerm Software                                           */
/*                      (c) Julien Schmitt                              */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Logging functionality for GTKTerm                               */
/*                                                                      */
/* This GtkTerm is free software: you can redistribute it and/or modify*/
/* it under the terms of the GNU  General Public License as published  */
/* by the Free Software Foundation, either version 3 of the License,   */
/* or (at your option) any later version.                              */
/*                                                                      */
/* GtkTerm is distributed in the hope that it will be useful, but      */
/* WITHOUT ANY WARRANTY; without even the implied warranty of          */
/* MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.                */
/* See the GNU General Public License for more details.                */
/*                                                                      */
/* You should have received a copy of the GNU General Public License   */
/* along with GtkTerm If not, see <https://www.gnu.org/licenses/>.     */
/*                                                                      */
/************************************************************************/

#include <stdio.h>
#include <string.h>
#include <glib.h>
#include <gio/gio.h>

#include "gtkterm_logging.h"
#include "gtkterm_data_chunk.h"

struct _GtkTermLogging {
    GObject parent_instance;
};

typedef struct {
    GtkTermLoggingState state;
    char *filename;
    GFile *file;
    GFileOutputStream *stream;
    GtkTermLoggingFormat format;
    gint64 last_timestamp_us;  /**< For calculating deltas */
} GtkTermLoggingPrivate;

G_DEFINE_TYPE_WITH_PRIVATE (GtkTermLogging, gtkterm_logging, G_TYPE_OBJECT)

/**
 * @brief Create a new logging object
 */
GtkTermLogging *gtkterm_logging_new (void) {
    return g_object_new (GTKTERM_TYPE_LOGGING, NULL);
}

/**
 * @brief Start logging to a file
 *
 * @param self The logging object
 * @param filename The file to log to
 * @param error Error return location
 *
 * @return TRUE on success, FALSE on error
 */
gboolean gtkterm_logging_start (GtkTermLogging *self, const char *filename, GError **error) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    // Stop any existing logging
    if (priv->state != GTKTERM_LOGGING_STOPPED) {
        gtkterm_logging_stop (self);
    }

    // Create or append to file
    priv->file = g_file_new_for_path (filename);
    priv->stream = g_file_append_to (priv->file, G_FILE_CREATE_NONE, NULL, error);

    if (!priv->stream) {
        g_object_unref (priv->file);
        priv->file = NULL;
        return FALSE;
    }

    priv->filename = g_strdup (filename);
    priv->state = GTKTERM_LOGGING_ACTIVE;

    return TRUE;
}

/**
 * @brief Stop logging and close the file
 *
 * @param self The logging object
 */
void gtkterm_logging_stop (GtkTermLogging *self) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    if (priv->stream) {
        g_output_stream_close (G_OUTPUT_STREAM (priv->stream), NULL, NULL);
        g_object_unref (priv->stream);
        priv->stream = NULL;
    }

    if (priv->file) {
        g_object_unref (priv->file);
        priv->file = NULL;
    }

    if (priv->filename) {
        g_free (priv->filename);
        priv->filename = NULL;
    }

    priv->state = GTKTERM_LOGGING_STOPPED;
}

/**
 * @brief Pause logging (keep file open but don't write)
 *
 * @param self The logging object
 */
void gtkterm_logging_pause (GtkTermLogging *self) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    if (priv->state == GTKTERM_LOGGING_ACTIVE) {
        priv->state = GTKTERM_LOGGING_PAUSED;
    }
}

/**
 * @brief Resume logging after pause
 *
 * @param self The logging object
 */
void gtkterm_logging_resume (GtkTermLogging *self) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    if (priv->state == GTKTERM_LOGGING_PAUSED && priv->stream) {
        priv->state = GTKTERM_LOGGING_ACTIVE;
    }
}

/**
 * @brief Clear the log file
 *
 * @param self The logging object
 */
void gtkterm_logging_clear (GtkTermLogging *self) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    if (priv->filename) {
        char *filename = g_strdup (priv->filename);
        GtkTermLoggingState old_state = priv->state;

        // Stop logging
        gtkterm_logging_stop (self);

        // Delete the file
        GFile *file = g_file_new_for_path (filename);
        g_file_delete (file, NULL, NULL);
        g_object_unref (file);

        // Restart logging if it was active
        if (old_state == GTKTERM_LOGGING_ACTIVE || old_state == GTKTERM_LOGGING_PAUSED) {
            gtkterm_logging_start (self, filename, NULL);
            if (old_state == GTKTERM_LOGGING_PAUSED) {
                gtkterm_logging_pause (self);
            }
        }

        g_free (filename);
    }
}

/**
 * @brief Write data to the log file
 *
 * @param self The logging object
 * @param data The data to write
 * @param length The length of the data
 */
void gtkterm_logging_write (GtkTermLogging *self, const char *data, gsize length) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    if (priv->state == GTKTERM_LOGGING_ACTIVE && priv->stream) {
        g_output_stream_write (G_OUTPUT_STREAM (priv->stream), data, length, NULL, NULL);
        g_output_stream_flush (G_OUTPUT_STREAM (priv->stream), NULL, NULL);
    }
}

/**
 * @brief Get the current logging state
 *
 * @param self The logging object
 *
 * @return The current state
 */
GtkTermLoggingState gtkterm_logging_get_state (GtkTermLogging *self) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);
    return priv->state;
}

/**
 * @brief Get the current log filename
 *
 * @param self The logging object
 *
 * @return The filename or NULL
 */
const char *gtkterm_logging_get_filename (GtkTermLogging *self) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);
    return priv->filename;
}

/**
 * @brief Dispose of the logging object
 */
static void gtkterm_logging_dispose (GObject *object) {
    GtkTermLogging *self = GTKTERM_LOGGING (object);

    gtkterm_logging_stop (self);

    G_OBJECT_CLASS (gtkterm_logging_parent_class)->dispose (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_logging_class_init (GtkTermLoggingClass *class) {
    GObjectClass *object_class = G_OBJECT_CLASS (class);

    object_class->dispose = gtkterm_logging_dispose;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_logging_init (GtkTermLogging *self) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    priv->state = GTKTERM_LOGGING_STOPPED;
    priv->filename = NULL;
    priv->file = NULL;
    priv->stream = NULL;
    priv->format = GTKTERM_LOGGING_FORMAT_RAW;
    priv->last_timestamp_us = 0;
}

/**
 * @brief Start session-based logging with auto-generated filename
 *
 * @param self The logging object
 * @param directory Directory to store log files
 * @param port_name Name of the serial port
 * @param profile_name Configuration profile name
 * @param format Logging format
 * @param error Error return location
 *
 * @return TRUE on success, FALSE on error
 */
gboolean gtkterm_logging_start_session (GtkTermLogging *self,
                                        const char *directory,
                                        const char *port_name,
                                        const char *profile_name,
                                        GtkTermLoggingFormat format,
                                        GError **error) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    // Generate filename with timestamp
    GDateTime *now = g_date_time_new_now_local ();
    char *timestamp = g_date_time_format (now, "%Y%m%d_%H%M%S");
    g_date_time_unref (now);

    // Sanitize port name (remove /dev/ prefix and slashes)
    char *port_safe = g_strdup (port_name);
    if (g_str_has_prefix (port_safe, "/dev/")) {
        char *temp = g_strdup (port_safe + 5);
        g_free (port_safe);
        port_safe = temp;
    }
    g_strdelimit (port_safe, "/", '_');

    // Choose extension based on format
    const char *ext = NULL;
    switch (format) {
        case GTKTERM_LOGGING_FORMAT_RAW:
            ext = "log";
            break;
        case GTKTERM_LOGGING_FORMAT_TEXT:
            ext = "txt";
            break;
        case GTKTERM_LOGGING_FORMAT_CSV:
            ext = "csv";
            break;
        case GTKTERM_LOGGING_FORMAT_JSON:
            ext = "json";
            break;
        default:
            ext = "log";
    }

    // Build filename: YYYYMMDD_HHMMSS_port_profile.ext
    char *filename = g_strdup_printf ("%s/%s_%s_%s.%s",
                                      directory, timestamp, port_safe,
                                      profile_name, ext);

    g_free (timestamp);
    g_free (port_safe);

    // Start logging
    gboolean result = gtkterm_logging_start (self, filename, error);
    g_free (filename);

    if (result) {
        priv->format = format;
        priv->last_timestamp_us = 0;

        // Write format-specific headers
        if (format == GTKTERM_LOGGING_FORMAT_CSV) {
            const char *header = "Timestamp,Direction,Length,Data_Hex,Data_ASCII\n";
            g_output_stream_write (G_OUTPUT_STREAM (priv->stream),
                                   header, strlen (header), NULL, NULL);
        } else if (format == GTKTERM_LOGGING_FORMAT_JSON) {
            const char *header = "{\n  \"session\": {\n";
            g_output_stream_write (G_OUTPUT_STREAM (priv->stream),
                                   header, strlen (header), NULL, NULL);

            // Write metadata
            GDateTime *dt = g_date_time_new_now_local ();
            char *start_time = g_date_time_format (dt, "%Y-%m-%d %H:%M:%S");
            char *metadata = g_strdup_printf ("    \"start_time\": \"%s\",\n"
                                              "    \"port\": \"%s\",\n"
                                              "    \"profile\": \"%s\"\n"
                                              "  },\n  \"data\": [\n",
                                              start_time, port_name, profile_name);
            g_date_time_unref (dt);
            g_free (start_time);

            g_output_stream_write (G_OUTPUT_STREAM (priv->stream),
                                   metadata, strlen (metadata), NULL, NULL);
            g_free (metadata);
        }
    }

    return result;
}

/**
 * @brief Write metadata key-value pair
 *
 * @param self The logging object
 * @param key Metadata key
 * @param value Metadata value
 */
void gtkterm_logging_write_metadata (GtkTermLogging *self,
                                     const char *key,
                                     const char *value) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    if (priv->state != GTKTERM_LOGGING_ACTIVE || !priv->stream)
        return;

    char *line = g_strdup_printf ("# %s: %s\n", key, value);
    g_output_stream_write (G_OUTPUT_STREAM (priv->stream),
                           line, strlen (line), NULL, NULL);
    g_free (line);
}

/**
 * @brief Write a data chunk to the log
 *
 * @param self The logging object
 * @param chunk The data chunk to write
 * @param session_start_time Session start timestamp for relative times
 */
void gtkterm_logging_write_chunk (GtkTermLogging *self,
                                  const GtkTermDataChunk *chunk,
                                  gint64 session_start_time) {
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (self);

    if (priv->state != GTKTERM_LOGGING_ACTIVE || !priv->stream || !chunk)
        return;

    GString *output = g_string_new (NULL);

    switch (priv->format) {
        case GTKTERM_LOGGING_FORMAT_RAW:
            // Just write raw bytes
            g_output_stream_write (G_OUTPUT_STREAM (priv->stream),
                                   chunk->data, chunk->length, NULL, NULL);
            break;

        case GTKTERM_LOGGING_FORMAT_TEXT: {
            // Timestamp + direction + data
            char *ts = gtkterm_data_chunk_get_timestamp_string (chunk, "absolute", 0);
            char dir = (chunk->direction == GTKTERM_DATA_DIRECTION_RX) ? 'R' : 'T';

            g_string_printf (output, "[%s] %c: ", ts, dir);
            g_free (ts);

            // Append data (escape non-printable chars)
            for (gsize i = 0; i < chunk->length; i++) {
                if (chunk->data[i] >= 0x20 && chunk->data[i] <= 0x7E) {
                    g_string_append_c (output, chunk->data[i]);
                } else {
                    g_string_append_printf (output, "<%02X>", chunk->data[i]);
                }
            }
            g_string_append_c (output, '\n');
            break;
        }

        case GTKTERM_LOGGING_FORMAT_CSV: {
            // CSV: Timestamp,Direction,Length,Data_Hex,Data_ASCII
            char *ts = gtkterm_data_chunk_get_timestamp_string (chunk, "absolute", 0);
            const char *dir = (chunk->direction == GTKTERM_DATA_DIRECTION_RX) ? "RX" : "TX";

            g_string_printf (output, "%s,%s,%zu,", ts, dir, chunk->length);
            g_free (ts);

            // Hex data
            for (gsize i = 0; i < chunk->length; i++) {
                g_string_append_printf (output, "%02X ", chunk->data[i]);
            }
            g_string_append_c (output, ',');

            // ASCII data (quoted and escaped)
            g_string_append_c (output, '"');
            for (gsize i = 0; i < chunk->length; i++) {
                if (chunk->data[i] >= 0x20 && chunk->data[i] <= 0x7E) {
                    if (chunk->data[i] == '"')
                        g_string_append (output, "\"\"");  // CSV escape
                    else
                        g_string_append_c (output, chunk->data[i]);
                } else {
                    g_string_append_c (output, '.');
                }
            }
            g_string_append (output, "\"\n");
            break;
        }

        case GTKTERM_LOGGING_FORMAT_JSON: {
            // JSON object for this chunk
            char *ts = gtkterm_data_chunk_get_timestamp_string (chunk, "absolute", 0);
            const char *dir = (chunk->direction == GTKTERM_DATA_DIRECTION_RX) ? "rx" : "tx";

            if (priv->last_timestamp_us != 0) {
                g_string_append (output, ",\n");
            }

            g_string_append (output, "    {");
            g_string_append_printf (output, "\"timestamp\": \"%s\", ", ts);
            g_string_append_printf (output, "\"direction\": \"%s\", ", dir);
            g_string_append_printf (output, "\"length\": %zu, ", chunk->length);

            // Hex array
            g_string_append (output, "\"hex\": [");
            for (gsize i = 0; i < chunk->length; i++) {
                if (i > 0) g_string_append (output, ", ");
                g_string_append_printf (output, "%d", chunk->data[i]);
            }
            g_string_append (output, "]}");

            g_free (ts);
            break;
        }

        case GTKTERM_LOGGING_FORMAT_XML: {
            // XML element for this chunk
            char *ts = gtkterm_data_chunk_get_timestamp_string (chunk, "absolute", 0);
            const char *dir = (chunk->direction == GTKTERM_DATA_DIRECTION_RX) ? "rx" : "tx";

            g_string_append (output, "    <packet");
            g_string_append_printf (output, " timestamp=\"%s\"", ts);
            g_string_append_printf (output, " direction=\"%s\"", dir);
            g_string_append_printf (output, " length=\"%zu\">\n", chunk->length);

            // Hex data
            g_string_append (output, "      <hex>");
            for (gsize i = 0; i < chunk->length; i++) {
                if (i > 0) g_string_append_c (output, ' ');
                g_string_append_printf (output, "%02X", chunk->data[i]);
            }
            g_string_append (output, "</hex>\n");

            // ASCII data (XML-escaped)
            g_string_append (output, "      <ascii><![CDATA[");
            for (gsize i = 0; i < chunk->length; i++) {
                if (chunk->data[i] >= 0x20 && chunk->data[i] <= 0x7E) {
                    g_string_append_c (output, chunk->data[i]);
                } else {
                    g_string_append_c (output, '.');
                }
            }
            g_string_append (output, "]]></ascii>\n");
            g_string_append (output, "    </packet>\n");

            g_free (ts);
            break;
        }
    }

    if (output->len > 0) {
        g_output_stream_write (G_OUTPUT_STREAM (priv->stream),
                               output->str, output->len, NULL, NULL);
        g_output_stream_flush (G_OUTPUT_STREAM (priv->stream), NULL, NULL);
    }

    priv->last_timestamp_us = chunk->timestamp_us;
    g_string_free (output, TRUE);
}

/**
 * @brief Export data chunks to a file
 *
 * @param filename Output filename
 * @param chunks Array of GtkTermDataChunk
 * @param format Export format
 * @param session_start_time Session start time for relative timestamps
 * @param error Error return location
 *
 * @return TRUE on success, FALSE on error
 */
gboolean gtkterm_logging_export_chunks (const char *filename,
                                        GPtrArray *chunks,
                                        GtkTermLoggingFormat format,
                                        gint64 session_start_time,
                                        GError **error) {
    GFile *file = g_file_new_for_path (filename);
    GFileOutputStream *stream = g_file_replace (file, NULL, FALSE,
                                                G_FILE_CREATE_NONE, NULL, error);

    if (!stream) {
        g_object_unref (file);
        return FALSE;
    }

    // Write headers
    if (format == GTKTERM_LOGGING_FORMAT_CSV) {
        const char *header = "Timestamp,Direction,Length,Data_Hex,Data_ASCII\n";
        g_output_stream_write (G_OUTPUT_STREAM (stream), header, strlen (header), NULL, NULL);
    } else if (format == GTKTERM_LOGGING_FORMAT_JSON) {
        const char *header = "{\n  \"data\": [\n";
        g_output_stream_write (G_OUTPUT_STREAM (stream), header, strlen (header), NULL, NULL);
    } else if (format == GTKTERM_LOGGING_FORMAT_XML) {
        const char *header = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                             "<gtkterm_log version=\"1.0\">\n"
                             "  <packets>\n";
        g_output_stream_write (G_OUTPUT_STREAM (stream), header, strlen (header), NULL, NULL);
    }

    // Create temporary logging object to reuse write_chunk logic
    GtkTermLogging *temp_log = gtkterm_logging_new ();
    GtkTermLoggingPrivate *priv = gtkterm_logging_get_instance_private (temp_log);
    priv->stream = stream;
    priv->state = GTKTERM_LOGGING_ACTIVE;
    priv->format = format;
    priv->last_timestamp_us = 0;

    // Write all chunks
    for (guint i = 0; i < chunks->len; i++) {
        GtkTermDataChunk *chunk = g_ptr_array_index (chunks, i);
        gtkterm_logging_write_chunk (temp_log, chunk, session_start_time);
    }

    // Write footers
    if (format == GTKTERM_LOGGING_FORMAT_JSON) {
        const char *footer = "\n  ]\n}\n";
        g_output_stream_write (G_OUTPUT_STREAM (stream), footer, strlen (footer), NULL, NULL);
    } else if (format == GTKTERM_LOGGING_FORMAT_XML) {
        const char *footer = "  </packets>\n</gtkterm_log>\n";
        g_output_stream_write (G_OUTPUT_STREAM (stream), footer, strlen (footer), NULL, NULL);
    }

    g_output_stream_close (G_OUTPUT_STREAM (stream), NULL, NULL);
    g_object_unref (stream);
    g_object_unref (file);
    g_object_unref (temp_log);

    return TRUE;
}
