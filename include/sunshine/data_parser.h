/*
 * Sunshine Data Parser Header
 *
 * This defines the packet format I came up with and the parser that decodes it.
 * The packet format is pretty simple: {ID|Name|R,G,B|value|...}
 */

#ifndef GTKTERM_SUNSHINE_DATA_PARSER_H
#define GTKTERM_SUNSHINE_DATA_PARSER_H

#include <glib-object.h>
#include <glib.h>

G_BEGIN_DECLS

// Data structures for storing parsed packet data

// RGB color for a data series
typedef struct {
    guint8 r;
    guint8 g;
    guint8 b;
    gboolean has_color;  // Set to FALSE if no color was specified in the packet
} GtkTermSunshineColor;

// A single data series (like "Temperature" or "Humidity")
typedef struct {
    gchar *name;
    GtkTermSunshineColor color;
    GArray *values;  // Array of gdouble (can have one or more values)
} GtkTermSunshineDataSeries;

// A complete parsed packet - contains multiple series
typedef struct {
    gchar *id;                  // Packet ID like "P0", "P1", etc
    GList *series;              // List of GtkTermSunshineDataSeries*
    guint64 timestamp_ms;       // When we received this packet
} GtkTermSunshineDataPacket;

// The parser object (GObject-based)

#define GTKTERM_TYPE_SUNSHINE_PARSER (gtkterm_sunshine_parser_get_type())
G_DECLARE_FINAL_TYPE(GtkTermSunshineParser, gtkterm_sunshine_parser, GTKTERM, SUNSHINE_PARSER, GObject)

// Callback that gets called when we successfully parse a packet
typedef void (*GtkTermSunshineDataCallback)(GtkTermSunshineDataPacket *packet, gpointer user_data);

// Create a new parser
GtkTermSunshineParser *gtkterm_sunshine_parser_new(void);

// Try to parse a line - returns TRUE if it's a valid Sunshine packet
gboolean gtkterm_sunshine_parser_parse_line(GtkTermSunshineParser *self,
                                             const gchar *line,
                                             GtkTermSunshineDataPacket **packet,
                                             GError **error);

// Set up a callback to be notified when packets are parsed
void gtkterm_sunshine_parser_set_callback(GtkTermSunshineParser *self,
                                           GtkTermSunshineDataCallback callback,
                                           gpointer user_data);

// Functions for creating/destroying packets and series
GtkTermSunshineDataPacket *gtkterm_sunshine_packet_new(void);
void gtkterm_sunshine_packet_free(GtkTermSunshineDataPacket *packet);

GtkTermSunshineDataSeries *gtkterm_sunshine_data_series_new(void);
void gtkterm_sunshine_data_series_free(GtkTermSunshineDataSeries *series);

// Get stats about parsing success/failures
guint64 gtkterm_sunshine_parser_get_packets_received(GtkTermSunshineParser *self);
guint64 gtkterm_sunshine_parser_get_packets_dropped(GtkTermSunshineParser *self);
void gtkterm_sunshine_parser_reset_stats(GtkTermSunshineParser *self);

G_END_DECLS

#endif // GTKTERM_SUNSHINE_DATA_PARSER_H
