/************************************************************************/
/* gtkterm_dataview.c                                                   */
/* ------------------                                                   */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Custom data view widget implementation                         */
/*                                                                      */
/************************************************************************/

#include <gtk/gtk.h>
#include <glib.h>
#include <string.h>
#include <ctype.h>
#include "gtkterm_dataview.h"
#include "gtkterm_data_chunk.h"

struct _GtkTermDataView {
    GtkTextView parent_instance;
};

typedef struct {
    GtkTextBuffer *buffer;
    GtkTextMark *end_mark;
    GtkTermTerminalView view_mode;
    GtkTermDataViewOptions options;
    GtkTermDataDirection filter_direction;
    gboolean filter_enabled;
    gint64 last_timestamp_us;  /* For delta calculations */
    guint64 byte_offset;       /* Current byte offset for indexing */

    /* Text tags for formatting */
    GtkTextTag *tag_timestamp;
    GtkTextTag *tag_direction_rx;
    GtkTextTag *tag_direction_tx;
    GtkTextTag *tag_index;
    GtkTextTag *tag_control_char;
    GtkTextTag *tag_printable;
    GtkTextTag *tag_extended;
} GtkTermDataViewPrivate;

G_DEFINE_TYPE_WITH_PRIVATE (GtkTermDataView, gtkterm_dataview, GTK_TYPE_TEXT_VIEW)

/* Forward declarations */
static void gtkterm_dataview_init_text_tags (GtkTermDataView *self);
static void gtkterm_dataview_format_ascii (GtkTermDataView *self, const guint8 *data, gsize length);
static void gtkterm_dataview_format_hex (GtkTermDataView *self, const guint8 *data, gsize length);
static void gtkterm_dataview_format_binary (GtkTermDataView *self, const guint8 *data, gsize length);
static void gtkterm_dataview_format_decimal (GtkTermDataView *self, const guint8 *data, gsize length);
static void gtkterm_dataview_format_mixed (GtkTermDataView *self, const guint8 *data, gsize length);

/**
 * @brief Class initialization
 */
static void gtkterm_dataview_class_init (GtkTermDataViewClass *klass) {
    /* No special class init needed for now */
}

/**
 * @brief Instance initialization
 */
static void gtkterm_dataview_init (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);

    /* Create text buffer */
    priv->buffer = gtk_text_view_get_buffer (GTK_TEXT_VIEW (self));

    /* Create end mark for appending */
    GtkTextIter iter;
    gtk_text_buffer_get_end_iter (priv->buffer, &iter);
    priv->end_mark = gtk_text_buffer_create_mark (priv->buffer, "end", &iter, FALSE);

    /* Initialize options with defaults */
    priv->options.show_timestamps = FALSE;
    priv->options.show_direction = FALSE;
    priv->options.show_index = FALSE;
    priv->options.hex_bytes_per_line = 16;
    priv->options.auto_scroll = TRUE;
    priv->options.session_start_time = 0;
    priv->options.timestamp_format = "absolute";

    /* Initialize state */
    priv->view_mode = GTKTERM_TERMINAL_VIEW_ASCII;
    priv->filter_enabled = FALSE;
    priv->last_timestamp_us = 0;
    priv->byte_offset = 0;

    /* Set widget properties */
    gtk_text_view_set_editable (GTK_TEXT_VIEW (self), FALSE);
    gtk_text_view_set_cursor_visible (GTK_TEXT_VIEW (self), TRUE);
    gtk_text_view_set_monospace (GTK_TEXT_VIEW (self), TRUE);
    gtk_text_view_set_wrap_mode (GTK_TEXT_VIEW (self), GTK_WRAP_WORD_CHAR);

    /* Initialize text tags for formatting */
    gtkterm_dataview_init_text_tags (self);
}

/**
 * @brief Initialize text tags for formatting
 */
static void gtkterm_dataview_init_text_tags (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);

    /* Timestamp tag - gray italic */
    priv->tag_timestamp = gtk_text_buffer_create_tag (priv->buffer, "timestamp",
        "foreground", "#808080",
        "style", PANGO_STYLE_ITALIC,
        NULL);

    /* Direction RX - blue */
    priv->tag_direction_rx = gtk_text_buffer_create_tag (priv->buffer, "dir-rx",
        "foreground", "#0000FF",
        "weight", PANGO_WEIGHT_BOLD,
        NULL);

    /* Direction TX - green */
    priv->tag_direction_tx = gtk_text_buffer_create_tag (priv->buffer, "dir-tx",
        "foreground", "#008000",
        "weight", PANGO_WEIGHT_BOLD,
        NULL);

    /* Index/offset - gray */
    priv->tag_index = gtk_text_buffer_create_tag (priv->buffer, "index",
        "foreground", "#606060",
        NULL);

    /* Control characters (0x00-0x1F) - blue background */
    priv->tag_control_char = gtk_text_buffer_create_tag (priv->buffer, "control",
        "foreground", "#0000FF",
        "background", "#E0E0FF",
        NULL);

    /* Printable ASCII (0x20-0x7E) - default */
    priv->tag_printable = gtk_text_buffer_create_tag (priv->buffer, "printable",
        NULL);

    /* Extended ASCII (0x80-0xFF) - orange/red */
    priv->tag_extended = gtk_text_buffer_create_tag (priv->buffer, "extended",
        "foreground", "#FF4500",
        NULL);
}

/**
 * @brief Create a new data view widget
 */
GtkTermDataView *gtkterm_dataview_new (void) {
    return g_object_new (GTKTERM_TYPE_DATAVIEW, NULL);
}

/**
 * @brief Set the view mode
 */
void gtkterm_dataview_set_view_mode (GtkTermDataView *self, GtkTermTerminalView mode) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    g_print("DEBUG: gtkterm_dataview_set_view_mode() called - old_mode=%d, new_mode=%d\n",
            priv->view_mode, mode);
    priv->view_mode = mode;
}

/**
 * @brief Get the view mode
 */
GtkTermTerminalView gtkterm_dataview_get_view_mode (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    return priv->view_mode;
}

/**
 * @brief Set display options
 */
void gtkterm_dataview_set_options (GtkTermDataView *self, const GtkTermDataViewOptions *options) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    priv->options = *options;
}

/**
 * @brief Get display options
 */
void gtkterm_dataview_get_options (GtkTermDataView *self, GtkTermDataViewOptions *options) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    *options = priv->options;
}

/**
 * @brief Set timestamp display
 */
void gtkterm_dataview_set_show_timestamps (GtkTermDataView *self, gboolean show) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    priv->options.show_timestamps = show;
}

/**
 * @brief Get timestamp display setting
 */
gboolean gtkterm_dataview_get_show_timestamps (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    return priv->options.show_timestamps;
}

/**
 * @brief Set hex bytes per line
 */
void gtkterm_dataview_set_hex_bytes_per_line (GtkTermDataView *self, guint bytes) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    priv->options.hex_bytes_per_line = bytes;
}

/**
 * @brief Get hex bytes per line
 */
guint gtkterm_dataview_get_hex_bytes_per_line (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    return priv->options.hex_bytes_per_line;
}

/**
 * @brief Append a data chunk to the view
 */
void gtkterm_dataview_append_chunk (GtkTermDataView *self, const GtkTermDataChunk *chunk) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);

    if (chunk == NULL || chunk->length == 0)
        return;

    /* Check filter */
    if (priv->filter_enabled && chunk->direction != priv->filter_direction)
        return;

    GtkTextIter iter;
    gtk_text_buffer_get_iter_at_mark (priv->buffer, &iter, priv->end_mark);

    /* Add timestamp if enabled */
    if (priv->options.show_timestamps) {
        gint64 ref_time = priv->last_timestamp_us;
        if (g_strcmp0 (priv->options.timestamp_format, "relative") == 0) {
            ref_time = priv->options.session_start_time;
        }

        char *timestamp = gtkterm_data_chunk_get_timestamp_string (chunk,
                                                                    priv->options.timestamp_format,
                                                                    ref_time);
        gtk_text_buffer_insert_with_tags (priv->buffer, &iter, timestamp, -1,
                                          priv->tag_timestamp, NULL);
        gtk_text_buffer_insert (priv->buffer, &iter, " ", -1);
        g_free (timestamp);
    }

    /* Add direction indicator if enabled */
    if (priv->options.show_direction) {
        const char *dir_str = (chunk->direction == GTKTERM_DATA_DIRECTION_RX) ? "RX" : "TX";
        GtkTextTag *dir_tag = (chunk->direction == GTKTERM_DATA_DIRECTION_RX) ?
                              priv->tag_direction_rx : priv->tag_direction_tx;

        gtk_text_buffer_insert_with_tags (priv->buffer, &iter, dir_str, -1,
                                          dir_tag, NULL);
        gtk_text_buffer_insert (priv->buffer, &iter, " ", -1);
    }

    /* Format data based on view mode */
    g_print("DEBUG: gtkterm_dataview_append_chunk() - view_mode=%d, length=%zu\n",
            priv->view_mode, chunk->length);

    switch (priv->view_mode) {
        case GTKTERM_TERMINAL_VIEW_ASCII:
            g_print("DEBUG: Calling format_ascii()\n");
            gtkterm_dataview_format_ascii (self, chunk->data, chunk->length);
            break;
        case GTKTERM_TERMINAL_VIEW_HEX:
            g_print("DEBUG: Calling format_hex()\n");
            gtkterm_dataview_format_hex (self, chunk->data, chunk->length);
            break;
        case GTKTERM_TERMINAL_VIEW_BINARY:
            g_print("DEBUG: Calling format_binary()\n");
            gtkterm_dataview_format_binary (self, chunk->data, chunk->length);
            break;
        case GTKTERM_TERMINAL_VIEW_DECIMAL:
            g_print("DEBUG: Calling format_decimal()\n");
            gtkterm_dataview_format_decimal (self, chunk->data, chunk->length);
            break;
        case GTKTERM_TERMINAL_VIEW_MIXED:
            g_print("DEBUG: Calling format_mixed()\n");
            gtkterm_dataview_format_mixed (self, chunk->data, chunk->length);
            break;
        default:
            g_print("DEBUG: Calling format_ascii() [default case]\n");
            gtkterm_dataview_format_ascii (self, chunk->data, chunk->length);
            break;
    }

    /* Update state */
    priv->last_timestamp_us = chunk->timestamp_us;
    priv->byte_offset += chunk->length;

    /* Auto-scroll if enabled */
    if (priv->options.auto_scroll) {
        gtkterm_dataview_scroll_to_bottom (self);
    }
}

/**
 * @brief Append data directly (creates chunk internally)
 */
void gtkterm_dataview_append_data (GtkTermDataView *self,
                                   const guint8 *data,
                                   gsize length,
                                   GtkTermDataDirection direction) {
    GtkTermDataChunk *chunk = gtkterm_data_chunk_new (data, length, direction, 0);
    gtkterm_dataview_append_chunk (self, chunk);
    gtkterm_data_chunk_free (chunk);
}

/**
 * @brief Format data as ASCII
 */
static void gtkterm_dataview_format_ascii (GtkTermDataView *self, const guint8 *data, gsize length) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter iter;
    gtk_text_buffer_get_iter_at_mark (priv->buffer, &iter, priv->end_mark);

    for (gsize i = 0; i < length; i++) {
        guint8 byte = data[i];
        GtkTextTag *tag = NULL;
        char buf[2] = {0};

        /* Determine tag based on byte value */
        if (byte < 0x20) {
            /* Control character */
            tag = priv->tag_control_char;
            /* Show printable representation for common control chars */
            if (byte == '\n' || byte == '\r' || byte == '\t') {
                buf[0] = byte;
            } else {
                /* Show as dot or space for other control chars */
                buf[0] = '.';
            }
        } else if (byte >= 0x20 && byte <= 0x7E) {
            /* Printable ASCII */
            tag = priv->tag_printable;
            buf[0] = byte;
        } else {
            /* Extended ASCII */
            tag = priv->tag_extended;
            buf[0] = '.';  /* Or byte if you want to show extended ASCII */
        }

        if (tag) {
            gtk_text_buffer_insert_with_tags (priv->buffer, &iter, buf, -1, tag, NULL);
        } else {
            gtk_text_buffer_insert (priv->buffer, &iter, buf, -1);
        }
    }
}

/**
 * @brief Format data as hexadecimal
 */
static void gtkterm_dataview_format_hex (GtkTermDataView *self, const guint8 *data, gsize length) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter iter;
    gtk_text_buffer_get_iter_at_mark (priv->buffer, &iter, priv->end_mark);

    guint bytes_per_line = priv->options.hex_bytes_per_line;

    for (gsize i = 0; i < length; i++) {
        /* Add index/offset at start of line */
        if (priv->options.show_index && (i % bytes_per_line) == 0) {
            char offset_buf[32];
            g_snprintf (offset_buf, sizeof(offset_buf), "%08lX: ",
                       (unsigned long)(priv->byte_offset + i - length));
            gtk_text_buffer_insert_with_tags (priv->buffer, &iter, offset_buf, -1,
                                              priv->tag_index, NULL);
        }

        /* Insert hex byte */
        char hex_buf[4];
        g_snprintf (hex_buf, sizeof(hex_buf), "%02X ", data[i]);

        /* Color code based on byte type */
        GtkTextTag *tag = NULL;
        if (data[i] < 0x20) {
            tag = priv->tag_control_char;
        } else if (data[i] <= 0x7E) {
            tag = priv->tag_printable;
        } else {
            tag = priv->tag_extended;
        }

        if (tag) {
            gtk_text_buffer_insert_with_tags (priv->buffer, &iter, hex_buf, -1, tag, NULL);
        } else {
            gtk_text_buffer_insert (priv->buffer, &iter, hex_buf, -1);
        }

        /* Newline at end of line */
        if ((i + 1) % bytes_per_line == 0 && i + 1 < length) {
            gtk_text_buffer_insert (priv->buffer, &iter, "\n", -1);
        }
    }

    gtk_text_buffer_insert (priv->buffer, &iter, "\n", -1);
}

/**
 * @brief Format data as binary
 */
static void gtkterm_dataview_format_binary (GtkTermDataView *self, const guint8 *data, gsize length) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter iter;
    gtk_text_buffer_get_iter_at_mark (priv->buffer, &iter, priv->end_mark);

    for (gsize i = 0; i < length; i++) {
        char bin_buf[16];
        guint8 byte = data[i];

        /* Convert byte to binary string */
        g_snprintf (bin_buf, sizeof(bin_buf), "%c%c%c%c%c%c%c%c ",
                   (byte & 0x80) ? '1' : '0',
                   (byte & 0x40) ? '1' : '0',
                   (byte & 0x20) ? '1' : '0',
                   (byte & 0x10) ? '1' : '0',
                   (byte & 0x08) ? '1' : '0',
                   (byte & 0x04) ? '1' : '0',
                   (byte & 0x02) ? '1' : '0',
                   (byte & 0x01) ? '1' : '0');

        gtk_text_buffer_insert (priv->buffer, &iter, bin_buf, -1);

        if ((i + 1) % 4 == 0) {
            gtk_text_buffer_insert (priv->buffer, &iter, "\n", -1);
        }
    }

    gtk_text_buffer_insert (priv->buffer, &iter, "\n", -1);
}

/**
 * @brief Format data as decimal
 */
static void gtkterm_dataview_format_decimal (GtkTermDataView *self, const guint8 *data, gsize length) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter iter;
    gtk_text_buffer_get_iter_at_mark (priv->buffer, &iter, priv->end_mark);

    for (gsize i = 0; i < length; i++) {
        char dec_buf[8];
        g_snprintf (dec_buf, sizeof(dec_buf), "%3d ", data[i]);
        gtk_text_buffer_insert (priv->buffer, &iter, dec_buf, -1);

        if ((i + 1) % 16 == 0) {
            gtk_text_buffer_insert (priv->buffer, &iter, "\n", -1);
        }
    }

    gtk_text_buffer_insert (priv->buffer, &iter, "\n", -1);
}

/**
 * @brief Format data as mixed (hex + ASCII side-by-side, like hex editors)
 */
static void gtkterm_dataview_format_mixed (GtkTermDataView *self, const guint8 *data, gsize length) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter iter;
    gtk_text_buffer_get_iter_at_mark (priv->buffer, &iter, priv->end_mark);

    guint bytes_per_line = priv->options.hex_bytes_per_line;
    GString *hex_part = g_string_new (NULL);
    GString *ascii_part = g_string_new (NULL);

    for (gsize i = 0; i < length; i++) {
        guint8 byte = data[i];

        /* Start of line: add offset if enabled */
        if ((i % bytes_per_line) == 0) {
            if (priv->options.show_index) {
                char offset_buf[32];
                g_snprintf (offset_buf, sizeof(offset_buf), "%08lX  ",
                           (unsigned long)(priv->byte_offset + i - length));
                gtk_text_buffer_insert_with_tags (priv->buffer, &iter, offset_buf, -1,
                                                  priv->tag_index, NULL);
            }
            g_string_truncate (hex_part, 0);
            g_string_truncate (ascii_part, 0);
        }

        /* Build hex representation */
        g_string_append_printf (hex_part, "%02X ", byte);

        /* Build ASCII representation */
        if (byte >= 0x20 && byte <= 0x7E) {
            /* Printable ASCII */
            g_string_append_c (ascii_part, byte);
        } else {
            /* Non-printable - show as dot */
            g_string_append_c (ascii_part, '.');
        }

        /* End of line or end of data: print the line */
        if ((i + 1) % bytes_per_line == 0 || i + 1 == length) {
            /* Pad hex part if this is the last line and not full */
            if (i + 1 == length && (i + 1) % bytes_per_line != 0) {
                gsize bytes_in_line = (i + 1) % bytes_per_line;
                gsize padding = bytes_per_line - bytes_in_line;
                for (gsize p = 0; p < padding; p++) {
                    g_string_append (hex_part, "   ");  /* 3 spaces for each missing byte */
                }
            }

            /* Insert hex part */
            gtk_text_buffer_insert (priv->buffer, &iter, hex_part->str, -1);

            /* Insert separator */
            gtk_text_buffer_insert (priv->buffer, &iter, " | ", -1);

            /* Insert ASCII part */
            gtk_text_buffer_insert (priv->buffer, &iter, ascii_part->str, -1);

            /* Newline */
            gtk_text_buffer_insert (priv->buffer, &iter, "\n", -1);
        }
    }

    g_string_free (hex_part, TRUE);
    g_string_free (ascii_part, TRUE);
}

/**
 * @brief Clear all data from view
 */
void gtkterm_dataview_clear (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter start, end;

    gtk_text_buffer_get_bounds (priv->buffer, &start, &end);
    gtk_text_buffer_delete (priv->buffer, &start, &end);

    priv->byte_offset = 0;
    priv->last_timestamp_us = 0;
}

/**
 * @brief Set filter by direction
 */
void gtkterm_dataview_set_filter_direction (GtkTermDataView *self, GtkTermDataDirection direction) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    priv->filter_direction = direction;
    priv->filter_enabled = TRUE;
}

/**
 * @brief Clear filter
 */
void gtkterm_dataview_clear_filter (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    priv->filter_enabled = FALSE;
}

/**
 * @brief Scroll to bottom of view
 */
void gtkterm_dataview_scroll_to_bottom (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter iter;

    gtk_text_buffer_get_iter_at_mark (priv->buffer, &iter, priv->end_mark);
    gtk_text_view_scroll_to_iter (GTK_TEXT_VIEW (self), &iter, 0.0, FALSE, 0.0, 1.0);
}

/**
 * @brief Get all text from view
 */
gchar *gtkterm_dataview_get_all_text (GtkTermDataView *self) {
    GtkTermDataViewPrivate *priv = gtkterm_dataview_get_instance_private (self);
    GtkTextIter start, end;

    gtk_text_buffer_get_bounds (priv->buffer, &start, &end);
    return gtk_text_buffer_get_text (priv->buffer, &start, &end, FALSE);
}
