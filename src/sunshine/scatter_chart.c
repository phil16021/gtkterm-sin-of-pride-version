/*
 * Sunshine Scatter Chart Widget
 *
 * Scatter plots for showing correlations between variables.
 * Basically just dots on a chart - simpler than line charts!
 */

#include "scatter_chart.h"
#include <cairo.h>
#include <math.h>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

// Keep memory usage reasonable
#define MAX_DATA_POINTS 1000

struct _GtkTermSunshineScatterChart {
    GtkWidget parent_instance;

    gchar *widget_id;
    gchar *title;
    GList *series_list;        // All the scatter series we're tracking

    gdouble x_min, x_max;      // Chart boundaries
    gdouble y_min, y_max;

    gdouble point_size;        // How big to draw each dot
    guint64 time_counter;      // For time series without explicit X values
};

G_DEFINE_TYPE(GtkTermSunshineScatterChart, gtkterm_sunshine_scatter_chart, GTK_TYPE_WIDGET)

// Helper functions
static void gtkterm_sunshine_scatter_chart_dispose(GObject *object);
static void gtkterm_sunshine_scatter_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot);
static void draw_chart(GtkTermSunshineScatterChart *self, cairo_t *cr, int width, int height);
static GtkTermSunshineScatterSeries *find_series(GtkTermSunshineScatterChart *self, const gchar *name);

// GObject setup
static void gtkterm_sunshine_scatter_chart_class_init(GtkTermSunshineScatterChartClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    GtkWidgetClass *widget_class = GTK_WIDGET_CLASS(klass);

    object_class->dispose = gtkterm_sunshine_scatter_chart_dispose;
    widget_class->snapshot = gtkterm_sunshine_scatter_chart_snapshot;
}

// Initialize a new scatter chart
static void gtkterm_sunshine_scatter_chart_init(GtkTermSunshineScatterChart *self) {
    self->widget_id = NULL;
    self->title = g_strdup("Scatter Chart");
    self->series_list = NULL;
    self->x_min = 0.0;
    self->x_max = 100.0;         // Default range
    self->y_min = 0.0;
    self->y_max = 100.0;
    self->point_size = 3.0;      // Nice small dots
    self->time_counter = 0;
}

// Clean up when destroyed
static void gtkterm_sunshine_scatter_chart_dispose(GObject *object) {
    GtkTermSunshineScatterChart *self = GTKTERM_SUNSHINE_SCATTER_CHART(object);

    g_free(self->widget_id);
    self->widget_id = NULL;

    g_free(self->title);
    self->title = NULL;

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineScatterSeries *series = (GtkTermSunshineScatterSeries *)l->data;
        g_free(series->name);
        g_array_free(series->x_values, TRUE);
        g_array_free(series->y_values, TRUE);
        g_free(series);
    }
    g_list_free(self->series_list);
    self->series_list = NULL;

    G_OBJECT_CLASS(gtkterm_sunshine_scatter_chart_parent_class)->dispose(object);
}

// Create a new scatter chart widget
GtkTermSunshineScatterChart *gtkterm_sunshine_scatter_chart_new(void) {
    return g_object_new(GTKTERM_TYPE_SUNSHINE_SCATTER_CHART, NULL);
}

// Widget ID
void gtkterm_sunshine_scatter_chart_set_widget_id(GtkTermSunshineScatterChart *self, const gchar *widget_id) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));
    g_free(self->widget_id);
    self->widget_id = g_strdup(widget_id);
}

const gchar *gtkterm_sunshine_scatter_chart_get_widget_id(GtkTermSunshineScatterChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self), NULL);
    return self->widget_id;
}

// Title
void gtkterm_sunshine_scatter_chart_set_title(GtkTermSunshineScatterChart *self, const gchar *title) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));
    g_free(self->title);
    self->title = g_strdup(title);
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

const gchar *gtkterm_sunshine_scatter_chart_get_title(GtkTermSunshineScatterChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self), NULL);
    return self->title;
}

// Point size
void gtkterm_sunshine_scatter_chart_set_point_size(GtkTermSunshineScatterChart *self, gdouble size) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));
    self->point_size = size;
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Find series by name
static GtkTermSunshineScatterSeries *find_series(GtkTermSunshineScatterChart *self, const gchar *name) {
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineScatterSeries *series = (GtkTermSunshineScatterSeries *)l->data;
        if (g_strcmp0(series->name, name) == 0) {
            return series;
        }
    }
    return NULL;
}

// Add series
void gtkterm_sunshine_scatter_chart_add_series(GtkTermSunshineScatterChart *self,
                                        const gchar *series_name,
                                        GtkTermSunshineColor color) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));
    g_return_if_fail(series_name != NULL);

    if (find_series(self, series_name) != NULL) {
        return;  // Series already exists
    }

    GtkTermSunshineScatterSeries *series = g_new0(GtkTermSunshineScatterSeries, 1);
    series->name = g_strdup(series_name);
    series->color = color;
    series->x_values = g_array_new(FALSE, FALSE, sizeof(gdouble));
    series->y_values = g_array_new(FALSE, FALSE, sizeof(gdouble));
    series->visible = TRUE;

    self->series_list = g_list_append(self->series_list, series);
}

// Add data point
void gtkterm_sunshine_scatter_chart_add_data_point(GtkTermSunshineScatterChart *self,
                                            const gchar *series_name,
                                            gdouble x,
                                            gdouble y) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));
    g_return_if_fail(series_name != NULL);

    GtkTermSunshineScatterSeries *series = find_series(self, series_name);
    if (series == NULL) {
        g_warning("Series '%s' not found", series_name);
        return;
    }

    g_array_append_val(series->x_values, x);
    g_array_append_val(series->y_values, y);

    // Limit data points
    if (series->x_values->len > MAX_DATA_POINTS) {
        g_array_remove_index(series->x_values, 0);
        g_array_remove_index(series->y_values, 0);
    }

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Clear series
void gtkterm_sunshine_scatter_chart_clear_series(GtkTermSunshineScatterChart *self,
                                          const gchar *series_name) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));

    GtkTermSunshineScatterSeries *series = find_series(self, series_name);
    if (series != NULL) {
        g_array_remove_range(series->x_values, 0, series->x_values->len);
        g_array_remove_range(series->y_values, 0, series->y_values->len);
        gtk_widget_queue_draw(GTK_WIDGET(self));
    }
}

// Clear all
void gtkterm_sunshine_scatter_chart_clear_all(GtkTermSunshineScatterChart *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineScatterSeries *series = (GtkTermSunshineScatterSeries *)l->data;
        g_array_remove_range(series->x_values, 0, series->x_values->len);
        g_array_remove_range(series->y_values, 0, series->y_values->len);
    }

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Set series visibility
void gtkterm_sunshine_scatter_chart_set_series_visible(GtkTermSunshineScatterChart *self,
                                                const gchar *series_name,
                                                gboolean visible) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));

    GtkTermSunshineScatterSeries *series = find_series(self, series_name);
    if (series != NULL) {
        series->visible = visible;
        gtk_widget_queue_draw(GTK_WIDGET(self));
    }
}

// Handle incoming data packet
void gtkterm_sunshine_scatter_chart_handle_packet(GtkTermSunshineScatterChart *self,
                                           GtkTermSunshineDataPacket *packet) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_SCATTER_CHART(self));
    g_return_if_fail(packet != NULL);

    for (GList *l = packet->series; l != NULL; l = l->next) {
        GtkTermSunshineDataSeries *data_series = (GtkTermSunshineDataSeries *)l->data;

        // Auto-create series if it doesn't exist
        GtkTermSunshineScatterSeries *chart_series = find_series(self, data_series->name);
        if (chart_series == NULL) {
            gtkterm_sunshine_scatter_chart_add_series(self, data_series->name, data_series->color);
            chart_series = find_series(self, data_series->name);
        }

        // Handle multi-value data (X, Y pairs)
        if (data_series->values->len >= 2) {
            // Process pairs of values as (X, Y)
            for (guint i = 0; i + 1 < data_series->values->len; i += 2) {
                gdouble x = g_array_index(data_series->values, gdouble, i);
                gdouble y = g_array_index(data_series->values, gdouble, i + 1);
                gtkterm_sunshine_scatter_chart_add_data_point(self, data_series->name, x, y);
            }
        } else if (data_series->values->len == 1) {
            // Time series mode: use counter as X, value as Y
            gdouble y = g_array_index(data_series->values, gdouble, 0);
            gdouble x = (gdouble)self->time_counter++;
            gtkterm_sunshine_scatter_chart_add_data_point(self, data_series->name, x, y);
        }
    }
}

// Calculate axis ranges
static void calculate_axis_ranges(GtkTermSunshineScatterChart *self) {
    if (self->series_list == NULL) {
        self->x_min = 0.0;
        self->x_max = 100.0;
        self->y_min = 0.0;
        self->y_max = 100.0;
        return;
    }

    gboolean first = TRUE;
    gdouble x_min = 0.0, x_max = 0.0;
    gdouble y_min = 0.0, y_max = 0.0;

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineScatterSeries *series = (GtkTermSunshineScatterSeries *)l->data;

        if (!series->visible || series->x_values->len == 0) {
            continue;
        }

        for (guint i = 0; i < series->x_values->len; i++) {
            gdouble x = g_array_index(series->x_values, gdouble, i);
            gdouble y = g_array_index(series->y_values, gdouble, i);

            if (first) {
                x_min = x_max = x;
                y_min = y_max = y;
                first = FALSE;
            } else {
                if (x < x_min) x_min = x;
                if (x > x_max) x_max = x;
                if (y < y_min) y_min = y;
                if (y > y_max) y_max = y;
            }
        }
    }

    if (first) {
        self->x_min = 0.0;
        self->x_max = 100.0;
        self->y_min = 0.0;
        self->y_max = 100.0;
        return;
    }

    // Add 10% margin
    gdouble x_range = x_max - x_min;
    gdouble y_range = y_max - y_min;

    if (x_range < 0.001) {
        x_range = x_max * 0.1;
        if (x_range < 0.001) x_range = 1.0;
    }
    if (y_range < 0.001) {
        y_range = y_max * 0.1;
        if (y_range < 0.001) y_range = 1.0;
    }

    self->x_min = x_min - x_range * 0.1;
    self->x_max = x_max + x_range * 0.1;
    self->y_min = y_min - y_range * 0.1;
    self->y_max = y_max + y_range * 0.1;
}

// Draw grid and axes
static void draw_grid_and_axes(cairo_t *cr, int width, int height,
                               gdouble x_min, gdouble x_max,
                               gdouble y_min, gdouble y_max) {
    const int margin = 60;
    const int plot_width = width - 2 * margin;
    const int plot_height = height - 2 * margin;

    // Grid
    cairo_set_source_rgb(cr, 0.9, 0.9, 0.9);
    cairo_set_line_width(cr, 1.0);

    for (int i = 0; i <= 10; i++) {
        double x = margin + (plot_width * i / 10.0);
        cairo_move_to(cr, x, margin);
        cairo_line_to(cr, x, height - margin);

        double y = margin + (plot_height * i / 10.0);
        cairo_move_to(cr, margin, y);
        cairo_line_to(cr, width - margin, y);
    }
    cairo_stroke(cr);

    // Axes
    cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
    cairo_set_line_width(cr, 2.0);

    cairo_move_to(cr, margin, margin);
    cairo_line_to(cr, margin, height - margin);
    cairo_line_to(cr, width - margin, height - margin);
    cairo_stroke(cr);

    // Labels
    cairo_set_font_size(cr, 10.0);
    cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL, CAIRO_FONT_WEIGHT_NORMAL);

    // Y axis labels
    for (int i = 0; i <= 10; i++) {
        double y = margin + (plot_height * (10 - i) / 10.0);
        double value = y_min + (y_max - y_min) * i / 10.0;

        gchar *label = g_strdup_printf("%.1f", value);
        cairo_text_extents_t extents;
        cairo_text_extents(cr, label, &extents);

        cairo_move_to(cr, margin - extents.width - 10, y + extents.height / 2);
        cairo_show_text(cr, label);

        g_free(label);
    }

    // X axis labels
    for (int i = 0; i <= 10; i++) {
        double x = margin + (plot_width * i / 10.0);
        double value = x_min + (x_max - x_min) * i / 10.0;

        gchar *label = g_strdup_printf("%.1f", value);
        cairo_text_extents_t extents;
        cairo_text_extents(cr, label, &extents);

        cairo_move_to(cr, x - extents.width / 2, height - margin + 20);
        cairo_show_text(cr, label);

        g_free(label);
    }
}

// Draw scatter points
static void draw_series(cairo_t *cr, GtkTermSunshineScatterSeries *series,
                       int width, int height,
                       gdouble x_min, gdouble x_max,
                       gdouble y_min, gdouble y_max,
                       gdouble point_size) {
    const int margin = 60;
    const int plot_width = width - 2 * margin;
    const int plot_height = height - 2 * margin;

    if (series->x_values->len == 0) {
        return;
    }

    gdouble x_range = x_max - x_min;
    gdouble y_range = y_max - y_min;

    if (x_range < 0.001 || y_range < 0.001) {
        return;
    }

    // Set color
    if (series->color.has_color) {
        cairo_set_source_rgb(cr, series->color.r / 255.0,
                           series->color.g / 255.0,
                           series->color.b / 255.0);
    } else {
        cairo_set_source_rgb(cr, 0.0, 0.0, 1.0);  // Default blue
    }

    // Draw points
    for (guint i = 0; i < series->x_values->len; i++) {
        gdouble x_val = g_array_index(series->x_values, gdouble, i);
        gdouble y_val = g_array_index(series->y_values, gdouble, i);

        double x = margin + ((x_val - x_min) / x_range) * plot_width;
        double y = height - margin - ((y_val - y_min) / y_range) * plot_height;

        cairo_arc(cr, x, y, point_size, 0, 2 * M_PI);
        cairo_fill(cr);

        // Optional: add border to points
        cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
        cairo_arc(cr, x, y, point_size, 0, 2 * M_PI);
        cairo_stroke(cr);

        // Restore color
        if (series->color.has_color) {
            cairo_set_source_rgb(cr, series->color.r / 255.0,
                               series->color.g / 255.0,
                               series->color.b / 255.0);
        } else {
            cairo_set_source_rgb(cr, 0.0, 0.0, 1.0);
        }
    }
}

// Main drawing function
static void draw_chart(GtkTermSunshineScatterChart *self, cairo_t *cr, int width, int height) {
    // Background
    cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
    cairo_paint(cr);

    // Calculate ranges
    calculate_axis_ranges(self);

    // Draw grid and axes
    draw_grid_and_axes(cr, width, height, self->x_min, self->x_max,
                      self->y_min, self->y_max);

    // Draw title
    if (self->title) {
        cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
        cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL,
                             CAIRO_FONT_WEIGHT_BOLD);
        cairo_set_font_size(cr, 16.0);

        cairo_text_extents_t extents;
        cairo_text_extents(cr, self->title, &extents);

        cairo_move_to(cr, (width - extents.width) / 2, 30);
        cairo_show_text(cr, self->title);
    }

    // Draw series
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineScatterSeries *series = (GtkTermSunshineScatterSeries *)l->data;
        if (series->visible) {
            draw_series(cr, series, width, height,
                       self->x_min, self->x_max,
                       self->y_min, self->y_max,
                       self->point_size);
        }
    }
}

// GTK4 snapshot method for rendering
static void gtkterm_sunshine_scatter_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot) {
    GtkTermSunshineScatterChart *self = GTKTERM_SUNSHINE_SCATTER_CHART(widget);

    int width = gtk_widget_get_width(widget);
    int height = gtk_widget_get_height(widget);

    if (width <= 0 || height <= 0) {
        return;
    }

    graphene_rect_t bounds = GRAPHENE_RECT_INIT(0, 0, width, height);
    cairo_t *cr = gtk_snapshot_append_cairo(snapshot, &bounds);

    draw_chart(self, cr, width, height);

    cairo_destroy(cr);
}
