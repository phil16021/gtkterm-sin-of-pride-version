/*
 * Sunshine Bar Chart Header
 *
 * Bar chart widget for comparing values side-by-side.
 * Haven't fully implemented this one yet!
 */

#ifndef GTKTERM_SUNSHINE_BAR_CHART_H
#define GTKTERM_SUNSHINE_BAR_CHART_H

#include <gtk/gtk.h>
#include "data_parser.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_SUNSHINE_BAR_CHART (gtkterm_sunshine_bar_chart_get_type())
G_DECLARE_FINAL_TYPE(GtkTermSunshineBarChart, gtkterm_sunshine_bar_chart, GTKTERM, SUNSHINE_BAR_CHART, GtkWidget)

// Chart series data structure
typedef struct {
    gchar *name;
    GtkTermSunshineColor color;
    GArray *values;        // Array of gdouble (bar heights)
    gboolean visible;
} GtkTermSunshineBarSeries;

// Constructor
GtkTermSunshineBarChart *gtkterm_sunshine_bar_chart_new(void);

// Widget ID for packet routing
void gtkterm_sunshine_bar_chart_set_widget_id(GtkTermSunshineBarChart *self, const gchar *widget_id);
const gchar *gtkterm_sunshine_bar_chart_get_widget_id(GtkTermSunshineBarChart *self);

// Title
void gtkterm_sunshine_bar_chart_set_title(GtkTermSunshineBarChart *self, const gchar *title);
const gchar *gtkterm_sunshine_bar_chart_get_title(GtkTermSunshineBarChart *self);

// Data management
void gtkterm_sunshine_bar_chart_add_data_point(GtkTermSunshineBarChart *self,
                                                const gchar *series_name,
                                                gdouble value);

void gtkterm_sunshine_bar_chart_add_series(GtkTermSunshineBarChart *self,
                                            const gchar *series_name,
                                            GtkTermSunshineColor color);

void gtkterm_sunshine_bar_chart_clear_series(GtkTermSunshineBarChart *self,
                                              const gchar *series_name);

void gtkterm_sunshine_bar_chart_clear_all(GtkTermSunshineBarChart *self);

// Series visibility
void gtkterm_sunshine_bar_chart_set_series_visible(GtkTermSunshineBarChart *self,
                                                    const gchar *series_name,
                                                    gboolean visible);

// Packet handling
void gtkterm_sunshine_bar_chart_handle_packet(GtkTermSunshineBarChart *self,
                                               GtkTermSunshineDataPacket *packet);

// Category labels
void gtkterm_sunshine_bar_chart_set_categories(GtkTermSunshineBarChart *self,
                                                const gchar **categories,
                                                gsize n_categories);

G_END_DECLS

#endif // GTKTERM_SUNSHINE_BAR_CHART_H
