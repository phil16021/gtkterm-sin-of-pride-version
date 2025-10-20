/*
 * Sunshine Pie Chart Header
 *
 * Pie chart widget for showing proportions and percentages.
 * The trigonometry for slices was interesting to figure out!
 */

#ifndef GTKTERM_SUNSHINE_PIE_CHART_H
#define GTKTERM_SUNSHINE_PIE_CHART_H

#include <gtk/gtk.h>
#include "data_parser.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_SUNSHINE_PIE_CHART (gtkterm_sunshine_pie_chart_get_type())
G_DECLARE_FINAL_TYPE(GtkTermSunshinePieChart, gtkterm_sunshine_pie_chart, GTKTERM, SUNSHINE_PIE_CHART, GtkWidget)

// Pie slice data structure
typedef struct {
    gchar *name;
    gdouble value;
    GtkTermSunshineColor color;
    gboolean visible;
} GtkTermSunshinePieSlice;

// Constructor
GtkTermSunshinePieChart *gtkterm_sunshine_pie_chart_new(void);

// Widget ID for packet routing
void gtkterm_sunshine_pie_chart_set_widget_id(GtkTermSunshinePieChart *self, const gchar *widget_id);
const gchar *gtkterm_sunshine_pie_chart_get_widget_id(GtkTermSunshinePieChart *self);

// Title
void gtkterm_sunshine_pie_chart_set_title(GtkTermSunshinePieChart *self, const gchar *title);
const gchar *gtkterm_sunshine_pie_chart_get_title(GtkTermSunshinePieChart *self);

// Data management
void gtkterm_sunshine_pie_chart_set_slice(GtkTermSunshinePieChart *self,
                                           const gchar *slice_name,
                                           gdouble value,
                                           GtkTermSunshineColor color);

void gtkterm_sunshine_pie_chart_remove_slice(GtkTermSunshinePieChart *self,
                                              const gchar *slice_name);

void gtkterm_sunshine_pie_chart_clear_all(GtkTermSunshinePieChart *self);

// Slice visibility
void gtkterm_sunshine_pie_chart_set_slice_visible(GtkTermSunshinePieChart *self,
                                                   const gchar *slice_name,
                                                   gboolean visible);

// Packet handling
void gtkterm_sunshine_pie_chart_handle_packet(GtkTermSunshinePieChart *self,
                                               GtkTermSunshineDataPacket *packet);

// Display options
void gtkterm_sunshine_pie_chart_set_show_labels(GtkTermSunshinePieChart *self, gboolean show);
void gtkterm_sunshine_pie_chart_set_show_percentages(GtkTermSunshinePieChart *self, gboolean show);

G_END_DECLS

#endif // GTKTERM_SUNSHINE_PIE_CHART_H
