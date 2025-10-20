/************************************************************************/
/* gtkterm_data_chunk.c                                                	*/
/* --------------------                                                	*/
/*           GTKTerm Software                                          	*/
/*                      (c) 2025 Contributors                           	*/
/*                                                                     	*/
/* ------------------------------------------------------------------- 	*/
/*                                                                     	*/
/*   Purpose                                                           	*/
/*      Data chunk implementation for storing serial data with metadata	*/
/*                                                                     	*/
/************************************************************************/

#include <glib.h>
#include <string.h>
#include "gtkterm_data_chunk.h"

/**
 * @brief Get current time in microseconds
 */
gint64 gtkterm_data_chunk_get_current_time_us (void) {
    return g_get_real_time();
}

/**
 * @brief Create a new data chunk
 */
GtkTermDataChunk *gtkterm_data_chunk_new (const guint8 *data,
                                          gsize length,
                                          GtkTermDataDirection direction,
                                          gint64 timestamp_us) {
    // BUG FIX #2: Validate NULL pointer with non-zero length
    // This is undefined behavior and must be prevented
    if (data == NULL && length > 0) {
        // Silently reject - logging may be fatal in test mode
        return NULL;
    }

    GtkTermDataChunk *chunk = g_new0(GtkTermDataChunk, 1);

    chunk->timestamp_us = (timestamp_us == 0) ? gtkterm_data_chunk_get_current_time_us() : timestamp_us;
    chunk->direction = direction;
    chunk->length = length;

    // Copy data (safe now - data is guaranteed non-NULL if length > 0)
    if (length > 0) {
        chunk->data = g_malloc(length);
        memcpy(chunk->data, data, length);
    } else {
        chunk->data = NULL;
    }

    return chunk;
}

/**
 * @brief Free a data chunk
 */
void gtkterm_data_chunk_free (GtkTermDataChunk *chunk) {
    if (chunk == NULL)
        return;

    g_free(chunk->data);
    g_free(chunk);
}

/**
 * @brief Get human-readable timestamp string
 */
char *gtkterm_data_chunk_get_timestamp_string (const GtkTermDataChunk *chunk,
                                                const char *format,
                                                gint64 reference_time) {
    if (chunk == NULL)
        return g_strdup("00:00:00.000");

    gint64 timestamp = chunk->timestamp_us;

    if (g_strcmp0(format, "absolute") == 0) {
        // Absolute time: HH:MM:SS.mmm
        GDateTime *dt = g_date_time_new_from_unix_local(timestamp / 1000000);
        if (dt == NULL)
            return g_strdup("00:00:00.000");

        int ms = (timestamp % 1000000) / 1000;
        char *result = g_date_time_format(dt, "%H:%M:%S");
        char *final = g_strdup_printf("%s.%03d", result, ms);

        g_free(result);
        g_date_time_unref(dt);
        return final;

    } else if (g_strcmp0(format, "relative") == 0) {
        // Relative to reference time: +SS.mmm
        gint64 delta = timestamp - reference_time;
        int seconds = delta / 1000000;
        int ms = (delta % 1000000) / 1000;

        return g_strdup_printf("+%d.%03d", seconds, ms);

    } else if (g_strcmp0(format, "delta") == 0) {
        // Delta from previous: +SS.mmm
        // Note: caller must track previous timestamp
        gint64 delta = timestamp - reference_time;
        int ms_total = delta / 1000;

        if (ms_total < 1000) {
            return g_strdup_printf("+%dms", ms_total);
        } else {
            int seconds = ms_total / 1000;
            int ms = ms_total % 1000;
            return g_strdup_printf("+%d.%03ds", seconds, ms);
        }

    } else {
        // Default: milliseconds since epoch
        return g_strdup_printf("%lld", (long long)(timestamp / 1000));
    }
}
