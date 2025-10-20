/*
 * Sunshine Line Chart Header
 *
 * My main chart widget - draws nice line graphs using Cairo.
 * This is the one I spent the most time on!
 */

#ifndef GTKTERM_SUNSHINE_LINE_CHART_H
#define GTKTERM_SUNSHINE_LINE_CHART_H

#include <gtk/gtk.h>
#include "data_parser.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_SUNSHINE_LINE_CHART (gtkterm_sunshine_line_chart_get_type())
G_DECLARE_FINAL_TYPE(GtkTermSunshineLineChart, gtkterm_sunshine_line_chart, GTKTERM, SUNSHINE_LINE_CHART, GtkWidget)

// Structure for storing a data series (like "Temperature", "Humidity", etc)
typedef struct {
    gchar *name;
    GtkTermSunshineColor color;
    GArray *x_values;  // X coordinates (can be NULL for time series)
    GArray *y_values;  // Y coordinates (actual data values)
    gboolean visible;  // Whether to show this series or not
} GtkTermSunshineChartSeries;

// Create a new line chart widget
GtkTermSunshineLineChart *gtkterm_sunshine_line_chart_new(void);

// Set the chart title and ID
void gtkterm_sunshine_line_chart_set_title(GtkTermSunshineLineChart *self, const gchar *title);
void gtkterm_sunshine_line_chart_set_widget_id(GtkTermSunshineLineChart *self, const gchar *id);

// Add a new data series to the chart
void gtkterm_sunshine_line_chart_add_series(GtkTermSunshineLineChart *self,
                                             const gchar *name,
                                             GtkTermSunshineColor color);

// Add a single X,Y point to a series
void gtkterm_sunshine_line_chart_add_data_point(GtkTermSunshineLineChart *self,
                                                 const gchar *series_name,
                                                 gdouble x,
                                                 gdouble y);

// Add a Y value (X gets auto-incremented - good for time series)
void gtkterm_sunshine_line_chart_add_data_time_series(GtkTermSunshineLineChart *self,
                                                       const gchar *series_name,
                                                       gdouble y);

// Clear data from a specific series
void gtkterm_sunshine_line_chart_clear_series(GtkTermSunshineLineChart *self,
                                               const gchar *series_name);

void gtkterm_sunshine_line_chart_clear_all(GtkTermSunshineLineChart *self);

// Handle incoming data packets
void gtkterm_sunshine_line_chart_handle_packet(GtkTermSunshineLineChart *self,
                                                GtkTermSunshineDataPacket *packet);

// Properties
const gchar *gtkterm_sunshine_line_chart_get_widget_id(GtkTermSunshineLineChart *self);
const gchar *gtkterm_sunshine_line_chart_get_title(GtkTermSunshineLineChart *self);

// Chart series management
GtkTermSunshineChartSeries *gtkterm_sunshine_chart_series_new(const gchar *name, GtkTermSunshineColor color);
void gtkterm_sunshine_chart_series_free(GtkTermSunshineChartSeries *series);

G_END_DECLS

#endif // GTKTERM_SUNSHINE_LINE_CHART_H
