/*
 * Sunshine Manager Header
 *
 * This is the main controller that ties everything together.
 * It manages all the charts and routes data to the right places.
 */

#ifndef GTKTERM_SUNSHINE_MANAGER_H
#define GTKTERM_SUNSHINE_MANAGER_H

#include <glib-object.h>
#include <gtk/gtk.h>
#include "scanner.h"
#include "data_parser.h"

G_BEGIN_DECLS

// We need this but it's defined elsewhere
typedef struct _GtkTermBuffer GtkTermBuffer;

#define GTKTERM_TYPE_SUNSHINE_MANAGER (gtkterm_sunshine_manager_get_type())
G_DECLARE_FINAL_TYPE(GtkTermSunshineManager, gtkterm_sunshine_manager, GTKTERM, SUNSHINE_MANAGER, GObject)

// Create a new manager - pass it the notebook widget and the serial buffer to watch
GtkTermSunshineManager *gtkterm_sunshine_manager_new(GtkWidget *notebook,
                                                        GtkTermBuffer *buffer);

// Scan through all the existing data in the buffer to find packets
void gtkterm_sunshine_manager_scan_history(GtkTermSunshineManager *self);

// Clear all the charts (useful when you want to start fresh)
void gtkterm_sunshine_manager_clear_all(GtkTermSunshineManager *self);

// Reset from the current point (clears data but keeps watching from here)
void gtkterm_sunshine_manager_reset(GtkTermSunshineManager *self);

// Get some stats about how many packets we've seen and charts we have
void gtkterm_sunshine_manager_get_stats(GtkTermSunshineManager *self,
                                         guint *total_packets,
                                         guint *total_charts);

G_END_DECLS

#endif // GTKTERM_SUNSHINE_MANAGER_H
