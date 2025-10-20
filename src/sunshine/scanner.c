/*
 * Sunshine Buffer Scanner
 *
 * This module scans through the serial data buffer to find Sunshine packets.
 * It basically watches the incoming data and picks out the packets I need.
 */

#include <stdbool.h>
#include <string.h>
#include "scanner.h"
#include "data_parser.h"
#include "../gtkterm_defaults.h"
#include "../gtkterm_serial_port.h"
#include "../gtkterm_terminal.h"
#include "../gtkterm_buffer.h"
#include "../gtkterm_data_chunk.h"

struct _GtkTermSunshineScanner {
    GObject parent_instance;

    // Parsing stuff
    GtkTermSunshineParser *parser;              // The parser that actually decodes packets

    // Buffer for building up complete lines
    GString *line_buffer;                       // We need this because data comes in chunks

    // Callback function to notify when we find packets
    GtkTermSunshinePacketCallback packet_callback;
    gpointer callback_user_data;

    // Just some counters for debugging
    guint packets_found;
    guint lines_scanned;
};

G_DEFINE_TYPE(GtkTermSunshineScanner, gtkterm_sunshine_scanner, G_TYPE_OBJECT)

// Function declarations so the compiler doesn't complain
static void process_line(GtkTermSunshineScanner *self, const gchar *line);
static void reconstruct_lines_from_chunk(GtkTermSunshineScanner *self,
                                         const guint8 *data,
                                         gsize length);

static void gtkterm_sunshine_scanner_dispose(GObject *object) {
    GtkTermSunshineScanner *self = GTKTERM_SUNSHINE_SCANNER(object);

    if (self->parser) {
        g_object_unref(self->parser);
        self->parser = NULL;
    }

    if (self->line_buffer) {
        g_string_free(self->line_buffer, TRUE);
        self->line_buffer = NULL;
    }

    G_OBJECT_CLASS(gtkterm_sunshine_scanner_parent_class)->dispose(object);
}

static void gtkterm_sunshine_scanner_class_init(GtkTermSunshineScannerClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = gtkterm_sunshine_scanner_dispose;
}

static void gtkterm_sunshine_scanner_init(GtkTermSunshineScanner *self) {
    self->parser = gtkterm_sunshine_parser_new();
    self->line_buffer = g_string_new(NULL);
    self->packet_callback = NULL;
    self->callback_user_data = NULL;
    self->packets_found = 0;
    self->lines_scanned = 0;
}

GtkTermSunshineScanner *gtkterm_sunshine_scanner_new(void) {
    return g_object_new(GTKTERM_TYPE_SUNSHINE_SCANNER, NULL);
}

void gtkterm_sunshine_scanner_set_callback(GtkTermSunshineScanner *self,
                                            GtkTermSunshinePacketCallback callback,
                                            gpointer user_data) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCANNER(self));

    self->packet_callback = callback;
    self->callback_user_data = user_data;
}

void gtkterm_sunshine_scanner_reset(GtkTermSunshineScanner *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCANNER(self));

    g_string_truncate(self->line_buffer, 0);
    self->packets_found = 0;
    self->lines_scanned = 0;
}

void gtkterm_sunshine_scanner_get_stats(GtkTermSunshineScanner *self,
                                         guint *packets_found,
                                         guint *lines_scanned) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCANNER(self));

    if (packets_found) {
        *packets_found = self->packets_found;
    }
    if (lines_scanned) {
        *lines_scanned = self->lines_scanned;
    }
}

guint gtkterm_sunshine_scanner_scan_buffer(GtkTermSunshineScanner *self,
                                            GtkTermBuffer *buffer) {
    return gtkterm_sunshine_scanner_scan_from_index(self, buffer, 0);
}

guint gtkterm_sunshine_scanner_scan_from_index(GtkTermSunshineScanner *self,
                                                GtkTermBuffer *buffer,
                                                guint start_chunk_index) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_SCANNER(self), 0);
    g_return_val_if_fail(GTKTERM_IS_BUFFER(buffer), 0);

    GPtrArray *data_chunks = gtkterm_buffer_get_data_chunks(buffer);
    if (!data_chunks) {
        return 0;
    }

    guint initial_packets = self->packets_found;

    // Go through all the data chunks starting from where we left off
    for (guint i = start_chunk_index; i < data_chunks->len; i++) {
        GtkTermDataChunk *chunk = g_ptr_array_index(data_chunks, i);

        // We only care about incoming data, not what we sent
        if (chunk->direction == GTKTERM_DATA_DIRECTION_RX) {
            reconstruct_lines_from_chunk(self, chunk->data, chunk->length);
        }
    }

    // Don't forget to process any leftover data in the buffer
    if (self->line_buffer->len > 0) {
        process_line(self, self->line_buffer->str);
        g_string_truncate(self->line_buffer, 0);
    }

    return self->packets_found - initial_packets;
}

// This function pieces together complete lines from the raw bytes
// Had to deal with all the different line ending formats (LF, CR, CRLF) - annoying but works!
static void reconstruct_lines_from_chunk(GtkTermSunshineScanner *self,
                                         const guint8 *data,
                                         gsize length) {
    for (gsize i = 0; i < length; i++) {
        guint8 byte = data[i];

        if (byte == '\n' || byte == '\r') {
            // Found end of line! Process what we've accumulated so far
            if (self->line_buffer->len > 0) {
                process_line(self, self->line_buffer->str);
                g_string_truncate(self->line_buffer, 0);
            }

            // Handle Windows-style CRLF (skip the LF if we just saw a CR)
            if (byte == '\r' && i + 1 < length && data[i + 1] == '\n') {
                i++; // Skip the next byte
            }
        } else {
            // Keep building up the line, but only with printable characters
            if (byte >= 32 || byte == '\t') {
                g_string_append_c(self->line_buffer, (gchar)byte);
            }
        }
    }
}

// Try to parse a complete line as a Sunshine packet
// Most lines won't be packets, so we just ignore those
static void process_line(GtkTermSunshineScanner *self, const gchar *line) {
    if (!line || strlen(line) == 0) {
        return;
    }

    self->lines_scanned++;

    // See if this line contains a Sunshine packet
    GtkTermSunshineDataPacket *packet = NULL;
    GError *error = NULL;

    if (gtkterm_sunshine_parser_parse_line(self->parser, line, &packet, &error)) {
        // Nice! Found a valid packet
        self->packets_found++;

        // Tell whoever's interested about this packet
        if (self->packet_callback) {
            self->packet_callback(packet, self->callback_user_data);
        } else {
            // No one to send it to, so clean it up
            gtkterm_sunshine_packet_free(packet);
        }
    } else {
        // Not a packet, or malformed - just ignore it
        if (error) {
            g_error_free(error);
        }
    }
}
