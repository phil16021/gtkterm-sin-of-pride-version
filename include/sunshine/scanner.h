/*
 * Sunshine Scanner Header
 *
 * The scanner goes through the serial data buffer and finds Sunshine packets.
 * It handles line reconstruction and all that messy stuff.
 */

#ifndef GTKTERM_SUNSHINE_SCANNER_H
#define GTKTERM_SUNSHINE_SCANNER_H

#include <glib-object.h>
#include "data_parser.h"

G_BEGIN_DECLS

// Defined elsewhere
typedef struct _GtkTermBuffer GtkTermBuffer;

#define GTKTERM_TYPE_SUNSHINE_SCANNER (gtkterm_sunshine_scanner_get_type())
G_DECLARE_FINAL_TYPE(GtkTermSunshineScanner, gtkterm_sunshine_scanner, GTKTERM, SUNSHINE_SCANNER, GObject)

// Callback that fires every time we find a packet
typedef void (*GtkTermSunshinePacketCallback)(GtkTermSunshineDataPacket *packet, gpointer user_data);

// Create a new scanner
GtkTermSunshineScanner *gtkterm_sunshine_scanner_new(void);

// Set up a callback to get notified about found packets
void gtkterm_sunshine_scanner_set_callback(GtkTermSunshineScanner *self,
                                            GtkTermSunshinePacketCallback callback,
                                            gpointer user_data);

// Scan the entire buffer from the beginning - returns how many packets we found
guint gtkterm_sunshine_scanner_scan_buffer(GtkTermSunshineScanner *self,
                                            GtkTermBuffer *buffer);

// Scan from a specific point (good for incremental updates)
guint gtkterm_sunshine_scanner_scan_from_index(GtkTermSunshineScanner *self,
                                                GtkTermBuffer *buffer,
                                                guint start_chunk_index);

// Reset the scanner state
void gtkterm_sunshine_scanner_reset(GtkTermSunshineScanner *self);

// Get some stats about what we've scanned
void gtkterm_sunshine_scanner_get_stats(GtkTermSunshineScanner *self,
                                         guint *packets_found,
                                         guint *lines_scanned);

G_END_DECLS

#endif // GTKTERM_SUNSHINE_SCANNER_H
