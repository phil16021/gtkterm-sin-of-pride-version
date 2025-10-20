/*
 * Sunshine Data Parser
 *
 * This is the part that actually decodes the Sunshine packet format.
 * Took me a while to get the parsing logic right - lots of edge cases!
 */

#include "data_parser.h"
#include <gio/gio.h>
#include <string.h>
#include <stdlib.h>

struct _GtkTermSunshineParser {
    GObject parent_instance;

    // Keep track of how many packets worked vs failed
    guint64 packets_received;
    guint64 packets_dropped;

    // Callback for when we successfully parse a packet
    GtkTermSunshineDataCallback data_callback;
    gpointer callback_user_data;

    // Buffer for dealing with incomplete packets
    GString *line_buffer;
};

G_DEFINE_TYPE(GtkTermSunshineParser, gtkterm_sunshine_parser, G_TYPE_OBJECT)

// Helper functions declared here so I can use them below
static gboolean parse_packet_content(const gchar *content, GtkTermSunshineDataPacket *packet, GError **error);
static gboolean parse_color(const gchar *color_str, GtkTermSunshineColor *color);
static gboolean is_number(const gchar *str);

static void gtkterm_sunshine_parser_finalize(GObject *object) {
    GtkTermSunshineParser *self = GTKTERM_SUNSHINE_PARSER(object);

    if (self->line_buffer) {
        g_string_free(self->line_buffer, TRUE);
    }

    G_OBJECT_CLASS(gtkterm_sunshine_parser_parent_class)->finalize(object);
}

static void gtkterm_sunshine_parser_class_init(GtkTermSunshineParserClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->finalize = gtkterm_sunshine_parser_finalize;
}

static void gtkterm_sunshine_parser_init(GtkTermSunshineParser *self) {
    self->packets_received = 0;
    self->packets_dropped = 0;
    self->data_callback = NULL;
    self->callback_user_data = NULL;
    self->line_buffer = g_string_new(NULL);
}

GtkTermSunshineParser *gtkterm_sunshine_parser_new(void) {
    return g_object_new(GTKTERM_TYPE_SUNSHINE_PARSER, NULL);
}

void gtkterm_sunshine_parser_set_callback(GtkTermSunshineParser *self,
                                        GtkTermSunshineDataCallback callback,
                                        gpointer user_data) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PARSER(self));

    self->data_callback = callback;
    self->callback_user_data = user_data;
}

// Parse a line of data to see if it's a Sunshine packet
// Format I came up with: {ID|Name1|R,G,B|value1|Name2|R,G,B|value2|...}
// The color part is optional, so {ID|Name1|value1|Name2|value2|...} also works
gboolean gtkterm_sunshine_parser_parse_line(GtkTermSunshineParser *self,
                                          const gchar *line,
                                          GtkTermSunshineDataPacket **packet,
                                          GError **error) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_PARSER(self), FALSE);
    g_return_val_if_fail(line != NULL, FALSE);

    // Clean up any spaces at the start/end
    gchar *trimmed = g_strstrip(g_strdup(line));
    gsize len = strlen(trimmed);

    if (len < 3) {
        self->packets_dropped++;
        g_free(trimmed);
        return FALSE;
    }

    // Sunshine packets always start with { and end with }
    if (trimmed[0] != '{') {
        self->packets_dropped++;
        g_free(trimmed);
        return FALSE;
    }

    gchar *end_brace = strrchr(trimmed, '}');
    if (end_brace == NULL) {
        self->packets_dropped++;
        g_free(trimmed);
        return FALSE;
    }

    // Get everything between the braces
    *end_brace = '\0';
    gchar *content = trimmed + 1;

    // Make a new packet and timestamp it
    GtkTermSunshineDataPacket *pkt = gtkterm_sunshine_packet_new();
    pkt->timestamp_ms = g_get_real_time() / 1000;  // convert microseconds to milliseconds

    // Try to parse what's inside the braces
    gboolean success = parse_packet_content(content, pkt, error);

    if (success) {
        self->packets_received++;
        if (packet) {
            *packet = pkt;
        } else {
            // If caller doesn't want the packet back, send to callback
            if (self->data_callback) {
                self->data_callback(pkt, self->callback_user_data);
            }
            gtkterm_sunshine_packet_free(pkt);
        }
    } else {
        self->packets_dropped++;
        gtkterm_sunshine_packet_free(pkt);
    }

    g_free(trimmed);
    return success;
}

// This function parses the actual content inside the braces
// Format: ID|Name1|R,G,B|value1|Name2|R,G,B|value2|...
// The color (R,G,B) is optional for each series
static gboolean parse_packet_content(const gchar *content, GtkTermSunshineDataPacket *packet, GError **error) {
    if (content == NULL || strlen(content) == 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                   "Empty packet content");
        return FALSE;
    }

    // Split by pipe character (|)
    gchar **fields = g_strsplit(content, "|", -1);
    gint field_count = g_strv_length(fields);

    if (field_count < 2) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                   "Packet must have at least ID and one field");
        g_strfreev(fields);
        return FALSE;
    }

    // First field is always the packet ID
    packet->id = g_strdup(g_strstrip(fields[0]));

    // Go through the rest of the fields and build up data series
    // This part was tricky because color is optional
    gint i = 1;
    while (i < field_count) {
        if (strlen(g_strstrip(fields[i])) == 0) {
            i++;
            continue;
        }

        GtkTermSunshineDataSeries *series = gtkterm_sunshine_data_series_new();

        // Current field is the series name
        series->name = g_strdup(g_strstrip(fields[i]));
        i++;

        if (i >= field_count) {
            // Ran out of fields - malformed packet
            gtkterm_sunshine_data_series_free(series);
            break;
        }

        // Check if next field has commas (meaning it's a color) or not (meaning it's a value)
        gchar *next_field = g_strstrip(fields[i]);

        if (strchr(next_field, ',') != NULL) {
            // Found a color! Parse it
            if (parse_color(next_field, &series->color)) {
                series->color.has_color = TRUE;
            }
            i++;

            if (i >= field_count) {
                // Color but no value - that's wrong
                gtkterm_sunshine_data_series_free(series);
                break;
            }

            next_field = g_strstrip(fields[i]);
        } else {
            // No color, just default it
            series->color.has_color = FALSE;
        }

        // Parse the value(s)
        // Can be a single number or multiple numbers separated by spaces (for scatter plots)
        gchar **values = g_strsplit(next_field, " ", -1);
        for (gint j = 0; values[j] != NULL; j++) {
            gchar *value_str = g_strstrip(values[j]);
            if (strlen(value_str) > 0 && is_number(value_str)) {
                gdouble value = g_ascii_strtod(value_str, NULL);
                g_array_append_val(series->values, value);
            }
        }
        g_strfreev(values);

        // Add this series to the packet
        packet->series = g_list_append(packet->series, series);

        i++;
    }

    g_strfreev(fields);

    // A packet needs at least one series to be valid
    if (packet->series == NULL) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                   "Packet has no valid data series");
        return FALSE;
    }

    return TRUE;
}

// Parse a color string like "255,0,0" into RGB values
// Pretty straightforward - just split by commas and validate
static gboolean parse_color(const gchar *color_str, GtkTermSunshineColor *color) {
    if (color_str == NULL) {
        return FALSE;
    }

    gchar **components = g_strsplit(color_str, ",", 3);
    if (g_strv_length(components) != 3) {
        g_strfreev(components);
        return FALSE;
    }

    gint64 r = g_ascii_strtoll(components[0], NULL, 10);
    gint64 g = g_ascii_strtoll(components[1], NULL, 10);
    gint64 b = g_ascii_strtoll(components[2], NULL, 10);

    g_strfreev(components);

    // Make sure values are in valid range (0-255)
    if (r < 0 || r > 255 || g < 0 || g > 255 || b < 0 || b > 255) {
        return FALSE;
    }

    color->r = (guint8)r;
    color->g = (guint8)g;
    color->b = (guint8)b;

    return TRUE;
}

// Helper to check if a string is a valid number
// Used this to filter out junk data
static gboolean is_number(const gchar *str) {
    if (str == NULL || strlen(str) == 0) {
        return FALSE;
    }

    gchar *endptr = NULL;
    g_ascii_strtod(str, &endptr);

    // If parsing consumed the whole string, it's a valid number
    return (endptr != NULL && *endptr == '\0');
}

// Packet memory management functions

GtkTermSunshineDataPacket *gtkterm_sunshine_packet_new(void) {
    GtkTermSunshineDataPacket *packet = g_new0(GtkTermSunshineDataPacket, 1);
    packet->id = NULL;
    packet->series = NULL;
    packet->timestamp_ms = 0;
    return packet;
}

void gtkterm_sunshine_packet_free(GtkTermSunshineDataPacket *packet) {
    if (packet == NULL) {
        return;
    }

    g_free(packet->id);

    if (packet->series) {
        g_list_free_full(packet->series, (GDestroyNotify)gtkterm_sunshine_data_series_free);
    }

    g_free(packet);
}

GtkTermSunshineDataSeries *gtkterm_sunshine_data_series_new(void) {
    GtkTermSunshineDataSeries *series = g_new0(GtkTermSunshineDataSeries, 1);
    series->name = NULL;
    series->color.r = 0;
    series->color.g = 0;
    series->color.b = 0;
    series->color.has_color = FALSE;
    series->values = g_array_new(FALSE, FALSE, sizeof(gdouble));
    return series;
}

void gtkterm_sunshine_data_series_free(GtkTermSunshineDataSeries *series) {
    if (series == NULL) {
        return;
    }

    g_free(series->name);

    if (series->values) {
        g_array_free(series->values, TRUE);
    }

    g_free(series);
}

// Statistics

guint64 gtkterm_sunshine_parser_get_packets_received(GtkTermSunshineParser *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_PARSER(self), 0);
    return self->packets_received;
}

guint64 gtkterm_sunshine_parser_get_packets_dropped(GtkTermSunshineParser *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_PARSER(self), 0);
    return self->packets_dropped;
}

void gtkterm_sunshine_parser_reset_stats(GtkTermSunshineParser *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PARSER(self));
    self->packets_received = 0;
    self->packets_dropped = 0;
}
