/*
 * Sunshine Scatter Chart Header
 *
 * Scatter plot widget for finding correlations.
 * Just draws dots at X,Y coordinates - simpler than line charts!
 */

#ifndef GTKTERM_SUNSHINE_SCATTER_CHART_H
#define GTKTERM_SUNSHINE_SCATTER_CHART_H

#include <gtk/gtk.h>
#include "data_parser.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_SUNSHINE_SCATTER_CHART (gtkterm_sunshine_scatter_chart_get_type())
G_DECLARE_FINAL_TYPE(GtkTermSunshineScatterChart, gtkterm_sunshine_scatter_chart, GTKTERM, SUNSHINE_SCATTER_CHART, GtkWidget)

// Scatter series data structure
typedef struct {
    gchar *name;
    GtkTermSunshineColor color;
    GArray *x_values;      // Array of gdouble
    GArray *y_values;      // Array of gdouble
    gboolean visible;
} GtkTermSunshineScatterSeries;

// Constructor
GtkTermSunshineScatterChart *gtkterm_sunshine_scatter_chart_new(void);

// Widget ID for packet routing
void gtkterm_sunshine_scatter_chart_set_widget_id(GtkTermSunshineScatterChart *self, const gchar *widget_id);
const gchar *gtkterm_sunshine_scatter_chart_get_widget_id(GtkTermSunshineScatterChart *self);

// Title
void gtkterm_sunshine_scatter_chart_set_title(GtkTermSunshineScatterChart *self, const gchar *title);
const gchar *gtkterm_sunshine_scatter_chart_get_title(GtkTermSunshineScatterChart *self);

// Data management
void gtkterm_sunshine_scatter_chart_add_data_point(GtkTermSunshineScatterChart *self,
                                                    const gchar *series_name,
                                                    gdouble x,
                                                    gdouble y);

void gtkterm_sunshine_scatter_chart_add_series(GtkTermSunshineScatterChart *self,
                                                const gchar *series_name,
                                                GtkTermSunshineColor color);

void gtkterm_sunshine_scatter_chart_clear_series(GtkTermSunshineScatterChart *self,
                                                  const gchar *series_name);

void gtkterm_sunshine_scatter_chart_clear_all(GtkTermSunshineScatterChart *self);

// Series visibility
void gtkterm_sunshine_scatter_chart_set_series_visible(GtkTermSunshineScatterChart *self,
                                                        const gchar *series_name,
                                                        gboolean visible);

// Packet handling
void gtkterm_sunshine_scatter_chart_handle_packet(GtkTermSunshineScatterChart *self,
                                                   GtkTermSunshineDataPacket *packet);

// Point size
void gtkterm_sunshine_scatter_chart_set_point_size(GtkTermSunshineScatterChart *self, gdouble size);

G_END_DECLS

#endif // GTKTERM_SUNSHINE_SCATTER_CHART_H
