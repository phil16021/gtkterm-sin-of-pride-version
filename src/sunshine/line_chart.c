/*
 * Sunshine Line Chart Widget
 *
 * This is the main chart type I implemented. It draws line graphs using Cairo.
 * Learning Cairo was fun but took some trial and error to get the scaling right!
 */

#include "line_chart.h"
#include <cairo.h>
#include <math.h>

#define MAX_DATA_POINTS 1000   // Don't want to store infinite data
#define MARGIN 60.0             // Space for labels around the edges
#define GRID_DIVISIONS 10       // How many grid lines to draw

struct _GtkTermSunshineLineChart {
    GtkWidget parent_instance;

    // Identification
    gchar *widget_id;           // Packet ID this chart belongs to
    gchar *title;               // What to show at the top

    // All the data series we're tracking
    GList *series_list;         // List of GtkTermSunshineChartSeries*

    // Chart boundaries (calculated automatically from the data)
    gdouble x_min, x_max;
    gdouble y_min, y_max;

    // Counter for time-series data (when we don't have explicit X values)
    guint64 time_counter;
};

G_DEFINE_TYPE(GtkTermSunshineLineChart, gtkterm_sunshine_line_chart, GTK_TYPE_WIDGET)

// All the drawing helper functions I wrote
static void gtkterm_sunshine_line_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot);
static void calculate_axis_ranges(GtkTermSunshineLineChart *self);
static void draw_chart(GtkTermSunshineLineChart *self, cairo_t *cr, int width, int height);
static void draw_grid_and_axes(cairo_t *cr, int width, int height,
                               gdouble x_min, gdouble x_max,
                               gdouble y_min, gdouble y_max);
static void draw_series(cairo_t *cr, GtkTermSunshineChartSeries *series,
                       int width, int height,
                       gdouble x_min, gdouble x_max,
                       gdouble y_min, gdouble y_max,
                       gboolean is_time_series);
static void draw_legend(GtkTermSunshineLineChart *self, cairo_t *cr, int width, int height);

static void gtkterm_sunshine_line_chart_dispose(GObject *object) {
    GtkTermSunshineLineChart *self = GTKTERM_SUNSHINE_LINE_CHART(object);

    g_free(self->widget_id);
    g_free(self->title);

    if (self->series_list) {
        g_list_free_full(self->series_list, (GDestroyNotify)gtkterm_sunshine_chart_series_free);
        self->series_list = NULL;
    }

    G_OBJECT_CLASS(gtkterm_sunshine_line_chart_parent_class)->dispose(object);
}

static void gtkterm_sunshine_line_chart_class_init(GtkTermSunshineLineChartClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    GtkWidgetClass *widget_class = GTK_WIDGET_CLASS(klass);

    object_class->dispose = gtkterm_sunshine_line_chart_dispose;
    widget_class->snapshot = gtkterm_sunshine_line_chart_snapshot;
}

static void gtkterm_sunshine_line_chart_init(GtkTermSunshineLineChart *self) {
    self->widget_id = NULL;
    self->title = g_strdup("Line Chart");
    self->series_list = NULL;
    self->x_min = 0.0;
    self->x_max = 100.0;       // Default range, gets recalculated when data arrives
    self->y_min = 0.0;
    self->y_max = 100.0;
    self->time_counter = 0;

    gtk_widget_set_size_request(GTK_WIDGET(self), 400, 300);  // Minimum size
}

GtkTermSunshineLineChart *gtkterm_sunshine_line_chart_new(void) {
    return g_object_new(GTKTERM_TYPE_SUNSHINE_LINE_CHART, NULL);
}

void gtkterm_sunshine_line_chart_set_title(GtkTermSunshineLineChart *self, const gchar *title) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));

    g_free(self->title);
    self->title = g_strdup(title);
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

void gtkterm_sunshine_line_chart_set_widget_id(GtkTermSunshineLineChart *self, const gchar *id) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));

    g_free(self->widget_id);
    self->widget_id = g_strdup(id);
}

const gchar *gtkterm_sunshine_line_chart_get_widget_id(GtkTermSunshineLineChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self), NULL);
    return self->widget_id;
}

const gchar *gtkterm_sunshine_line_chart_get_title(GtkTermSunshineLineChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self), NULL);
    return self->title;
}

// Functions for managing data series

GtkTermSunshineChartSeries *gtkterm_sunshine_chart_series_new(const gchar *name, GtkTermSunshineColor color) {
    GtkTermSunshineChartSeries *series = g_new0(GtkTermSunshineChartSeries, 1);
    series->name = g_strdup(name);
    series->color = color;
    series->x_values = g_array_new(FALSE, FALSE, sizeof(gdouble));
    series->y_values = g_array_new(FALSE, FALSE, sizeof(gdouble));
    series->visible = TRUE;
    return series;
}

void gtkterm_sunshine_chart_series_free(GtkTermSunshineChartSeries *series) {
    if (series == NULL) {
        return;
    }

    g_free(series->name);
    if (series->x_values) {
        g_array_free(series->x_values, TRUE);
    }
    if (series->y_values) {
        g_array_free(series->y_values, TRUE);
    }
    g_free(series);
}

void gtkterm_sunshine_line_chart_add_series(GtkTermSunshineLineChart *self,
                                     const gchar *name,
                                     GtkTermSunshineColor color) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));

    GtkTermSunshineChartSeries *series = gtkterm_sunshine_chart_series_new(name, color);
    self->series_list = g_list_append(self->series_list, series);
}

static GtkTermSunshineChartSeries *find_series_by_name(GtkTermSunshineLineChart *self, const gchar *name) {
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineChartSeries *series = (GtkTermSunshineChartSeries *)l->data;
        if (g_strcmp0(series->name, name) == 0) {
            return series;
        }
    }
    return NULL;
}

void gtkterm_sunshine_line_chart_add_data_point(GtkTermSunshineLineChart *self,
                                         const gchar *series_name,
                                         gdouble x,
                                         gdouble y) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));

    GtkTermSunshineChartSeries *series = find_series_by_name(self, series_name);
    if (series == NULL) {
        return;
    }

    // Keep the chart from getting too cluttered by limiting points
    if (series->x_values->len >= MAX_DATA_POINTS) {
        g_array_remove_index(series->x_values, 0);  // Remove oldest
        g_array_remove_index(series->y_values, 0);
    }

    g_array_append_val(series->x_values, x);
    g_array_append_val(series->y_values, y);

    gtk_widget_queue_draw(GTK_WIDGET(self));  // Tell GTK to redraw the widget
}

void gtkterm_sunshine_line_chart_add_data_time_series(GtkTermSunshineLineChart *self,
                                               const gchar *series_name,
                                               gdouble y) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));

    gdouble x = (gdouble)self->time_counter++;
    gtkterm_sunshine_line_chart_add_data_point(self, series_name, x, y);
}

void gtkterm_sunshine_line_chart_clear_series(GtkTermSunshineLineChart *self,
                                       const gchar *series_name) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));

    GtkTermSunshineChartSeries *series = find_series_by_name(self, series_name);
    if (series == NULL) {
        return;
    }

    g_array_set_size(series->x_values, 0);
    g_array_set_size(series->y_values, 0);

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

void gtkterm_sunshine_line_chart_clear_all(GtkTermSunshineLineChart *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineChartSeries *series = (GtkTermSunshineChartSeries *)l->data;
        g_array_set_size(series->x_values, 0);
        g_array_set_size(series->y_values, 0);
    }

    self->time_counter = 0;
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Process incoming data packets from the serial port

void gtkterm_sunshine_line_chart_handle_packet(GtkTermSunshineLineChart *self,
                                        GtkTermSunshineDataPacket *packet) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_LINE_CHART(self));
    g_return_if_fail(packet != NULL);

    // Make sure this packet is actually for this chart
    if (self->widget_id == NULL || g_strcmp0(self->widget_id, packet->id) != 0) {
        return;
    }

    // Go through each data series in the packet
    for (GList *l = packet->series; l != NULL; l = l->next) {
        GtkTermSunshineDataSeries *data_series = (GtkTermSunshineDataSeries *)l->data;

        // Find the chart series or create it if this is the first time we've seen it
        GtkTermSunshineChartSeries *chart_series = find_series_by_name(self, data_series->name);
        if (chart_series == NULL) {
            // Auto-create a new series with the color from the packet (or gray if none)
            GtkTermSunshineColor color = data_series->color.has_color ?
                                  data_series->color :
                                  (GtkTermSunshineColor){128, 128, 128, FALSE};
            gtkterm_sunshine_line_chart_add_series(self, data_series->name, color);
            chart_series = find_series_by_name(self, data_series->name);
        }

        if (chart_series == NULL) {
            continue;  // Something went wrong
        }

        // Add the data to the chart
        if (data_series->values->len == 1) {
            // Just one value - treat as time series (auto-increment X)
            gdouble y = g_array_index(data_series->values, gdouble, 0);
            gtkterm_sunshine_line_chart_add_data_time_series(self, data_series->name, y);
        } else if (data_series->values->len >= 2) {
            // Multiple values - assume they're X,Y pairs
            for (guint i = 0; i + 1 < data_series->values->len; i += 2) {
                gdouble x = g_array_index(data_series->values, gdouble, i);
                gdouble y = g_array_index(data_series->values, gdouble, i + 1);
                gtkterm_sunshine_line_chart_add_data_point(self, data_series->name, x, y);
            }
        }
    }
}

// All the Cairo drawing code below

static void gtkterm_sunshine_line_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot) {
    GtkTermSunshineLineChart *self = GTKTERM_SUNSHINE_LINE_CHART(widget);
    int width = gtk_widget_get_width(widget);
    int height = gtk_widget_get_height(widget);

    if (width <= 0 || height <= 0) {
        return;
    }

    // Create Cairo context
    cairo_t *cr = gtk_snapshot_append_cairo(snapshot, &GRAPHENE_RECT_INIT(0, 0, width, height));

    // Draw the chart
    draw_chart(self, cr, width, height);

    cairo_destroy(cr);
}

// Figure out the min/max values so we can scale the chart properly
static void calculate_axis_ranges(GtkTermSunshineLineChart *self) {
    gboolean first = TRUE;

    // Look through all the data to find the extremes
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineChartSeries *series = (GtkTermSunshineChartSeries *)l->data;

        if (!series->visible || series->y_values->len == 0) {
            continue;
        }

        // Find Y range
        for (guint i = 0; i < series->y_values->len; i++) {
            gdouble y = g_array_index(series->y_values, gdouble, i);

            if (first) {
                self->y_min = self->y_max = y;
                first = FALSE;
            } else {
                if (y < self->y_min) self->y_min = y;
                if (y > self->y_max) self->y_max = y;
            }
        }

        // Find X range
        for (guint i = 0; i < series->x_values->len; i++) {
            gdouble x = g_array_index(series->x_values, gdouble, i);

            if (i == 0 && l == self->series_list) {
                self->x_min = self->x_max = x;
            } else {
                if (x < self->x_min) self->x_min = x;
                if (x > self->x_max) self->x_max = x;
            }
        }
    }

    // Add a bit of padding so the data doesn't touch the edges
    gdouble y_range = self->y_max - self->y_min;
    if (y_range < 0.01) y_range = 1.0;  // Handle flat lines
    self->y_min -= y_range * 0.1;
    self->y_max += y_range * 0.1;

    gdouble x_range = self->x_max - self->x_min;
    if (x_range < 0.01) x_range = 1.0;
    self->x_min -= x_range * 0.05;
    self->x_max += x_range * 0.05;
}

static void draw_chart(GtkTermSunshineLineChart *self, cairo_t *cr, int width, int height) {
    // Start with a clean white background
    cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
    cairo_paint(cr);

    // Figure out what range we're drawing
    calculate_axis_ranges(self);

    // Draw the grid lines and axes
    draw_grid_and_axes(cr, width, height, self->x_min, self->x_max, self->y_min, self->y_max);

    // Draw the title at the top
    if (self->title) {
        cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
        cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL, CAIRO_FONT_WEIGHT_BOLD);
        cairo_set_font_size(cr, 16.0);

        cairo_text_extents_t extents;
        cairo_text_extents(cr, self->title, &extents);
        cairo_move_to(cr, (width - extents.width) / 2.0, 25.0);  // Center it
        cairo_show_text(cr, self->title);
    }

    // Draw all the data lines
    gboolean is_time_series = TRUE;
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineChartSeries *series = (GtkTermSunshineChartSeries *)l->data;
        if (series->visible && series->y_values->len > 0) {
            draw_series(cr, series, width, height,
                       self->x_min, self->x_max, self->y_min, self->y_max,
                       is_time_series);
        }
    }

    // Draw the legend in the corner
    draw_legend(self, cr, width, height);
}

static void draw_grid_and_axes(cairo_t *cr, int width, int height,
                               gdouble x_min, gdouble x_max,
                               gdouble y_min, gdouble y_max) {
    // Grid
    cairo_set_source_rgb(cr, 0.9, 0.9, 0.9);
    cairo_set_line_width(cr, 1.0);

    for (int i = 0; i <= GRID_DIVISIONS; i++) {
        gdouble x = MARGIN + (width - 2 * MARGIN) * i / GRID_DIVISIONS;
        cairo_move_to(cr, x, MARGIN);
        cairo_line_to(cr, x, height - MARGIN);
        cairo_stroke(cr);

        gdouble y = MARGIN + (height - 2 * MARGIN) * i / GRID_DIVISIONS;
        cairo_move_to(cr, MARGIN, y);
        cairo_line_to(cr, width - MARGIN, y);
        cairo_stroke(cr);
    }

    // Axes
    cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
    cairo_set_line_width(cr, 2.0);

    cairo_move_to(cr, MARGIN, MARGIN);
    cairo_line_to(cr, MARGIN, height - MARGIN);
    cairo_line_to(cr, width - MARGIN, height - MARGIN);
    cairo_stroke(cr);

    // Axis labels
    cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL, CAIRO_FONT_WEIGHT_NORMAL);
    cairo_set_font_size(cr, 10.0);

    for (int i = 0; i <= GRID_DIVISIONS; i += 2) {
        // Y axis labels
        gdouble y_pos = MARGIN + (height - 2 * MARGIN) * i / GRID_DIVISIONS;
        gdouble y_val = y_max - (y_max - y_min) * i / GRID_DIVISIONS;
        gchar *label = g_strdup_printf("%.1f", y_val);

        cairo_text_extents_t extents;
        cairo_text_extents(cr, label, &extents);
        cairo_move_to(cr, MARGIN - extents.width - 5, y_pos + extents.height / 2);
        cairo_show_text(cr, label);
        g_free(label);

        // X axis labels
        gdouble x_pos = MARGIN + (width - 2 * MARGIN) * i / GRID_DIVISIONS;
        gdouble x_val = x_min + (x_max - x_min) * i / GRID_DIVISIONS;
        label = g_strdup_printf("%.1f", x_val);

        cairo_text_extents(cr, label, &extents);
        cairo_move_to(cr, x_pos - extents.width / 2, height - MARGIN + 15);
        cairo_show_text(cr, label);
        g_free(label);
    }
}

static void draw_series(cairo_t *cr, GtkTermSunshineChartSeries *series,
                       int width, int height,
                       gdouble x_min, gdouble x_max,
                       gdouble y_min, gdouble y_max,
                       gboolean is_time_series) {
    if (series->y_values->len == 0) {
        return;
    }

    // Set color
    if (series->color.has_color) {
        cairo_set_source_rgb(cr,
                            series->color.r / 255.0,
                            series->color.g / 255.0,
                            series->color.b / 255.0);
    } else {
        cairo_set_source_rgb(cr, 0.0, 0.5, 1.0);
    }

    cairo_set_line_width(cr, 2.0);

    // Draw line
    gboolean first = TRUE;
    for (guint i = 0; i < series->y_values->len; i++) {
        gdouble x_val = (series->x_values->len > i) ?
                       g_array_index(series->x_values, gdouble, i) : (gdouble)i;
        gdouble y_val = g_array_index(series->y_values, gdouble, i);

        gdouble x_pos = MARGIN + (width - 2 * MARGIN) * (x_val - x_min) / (x_max - x_min);
        gdouble y_pos = height - MARGIN - (height - 2 * MARGIN) * (y_val - y_min) / (y_max - y_min);

        if (first) {
            cairo_move_to(cr, x_pos, y_pos);
            first = FALSE;
        } else {
            cairo_line_to(cr, x_pos, y_pos);
        }
    }

    cairo_stroke(cr);
}

static void draw_legend(GtkTermSunshineLineChart *self, cairo_t *cr, int width, int height) {
    // Only draw legend if there are multiple series or if the series has a name
    guint visible_series_count = 0;
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineChartSeries *series = (GtkTermSunshineChartSeries *)l->data;
        if (series->visible && series->y_values->len > 0) {
            visible_series_count++;
        }
    }

    if (visible_series_count == 0) {
        return;
    }

    // Legend position: top-right corner
    gdouble legend_x = width - MARGIN - 150;
    gdouble legend_y = MARGIN + 10;
    gdouble legend_width = 140;
    gdouble legend_height = 20 + visible_series_count * 20;

    // Draw legend background
    cairo_set_source_rgba(cr, 1.0, 1.0, 1.0, 0.9);
    cairo_rectangle(cr, legend_x, legend_y, legend_width, legend_height);
    cairo_fill(cr);

    // Draw legend border
    cairo_set_source_rgb(cr, 0.5, 0.5, 0.5);
    cairo_set_line_width(cr, 1.0);
    cairo_rectangle(cr, legend_x, legend_y, legend_width, legend_height);
    cairo_stroke(cr);

    // Draw legend items
    cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL, CAIRO_FONT_WEIGHT_NORMAL);
    cairo_set_font_size(cr, 11.0);

    gdouble item_y = legend_y + 20;
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineChartSeries *series = (GtkTermSunshineChartSeries *)l->data;
        if (!series->visible || series->y_values->len == 0) {
            continue;
        }

        // Draw color box
        if (series->color.has_color) {
            cairo_set_source_rgb(cr,
                                series->color.r / 255.0,
                                series->color.g / 255.0,
                                series->color.b / 255.0);
        } else {
            cairo_set_source_rgb(cr, 0.0, 0.5, 1.0);
        }

        cairo_rectangle(cr, legend_x + 10, item_y - 8, 12, 12);
        cairo_fill(cr);

        // Draw series name
        cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
        cairo_move_to(cr, legend_x + 28, item_y);
        cairo_show_text(cr, series->name ? series->name : "Series");

        item_y += 20;
    }
}
