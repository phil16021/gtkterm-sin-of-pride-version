/*
 * Sunshine Bar Chart Widget
 *
 * Bar charts are good for comparing values side-by-side.
 * I haven't fully implemented this one yet - mostly focused on line charts!
 */

#include "bar_chart.h"
#include <cairo.h>
#include <math.h>

// Don't want too many bars cluttering the screen
#define MAX_DATA_POINTS 100

struct _GtkTermSunshineBarChart {
    GtkWidget parent_instance;

    gchar *widget_id;
    gchar *title;
    GList *series_list;        // All the bar series we're displaying

    gchar **categories;        // Labels for each bar group
    gsize n_categories;

    gdouble y_min;
    gdouble y_max;

    guint64 value_counter;     // Auto-generate categories if none provided
};

G_DEFINE_TYPE(GtkTermSunshineBarChart, gtkterm_sunshine_bar_chart, GTK_TYPE_WIDGET)

// Helper functions
static void gtkterm_sunshine_bar_chart_dispose(GObject *object);
static void gtkterm_sunshine_bar_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot);
static void draw_chart(GtkTermSunshineBarChart *self, cairo_t *cr, int width, int height);
static GtkTermSunshineBarSeries *find_series(GtkTermSunshineBarChart *self, const gchar *name);

// GObject setup stuff
static void gtkterm_sunshine_bar_chart_class_init(GtkTermSunshineBarChartClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    GtkWidgetClass *widget_class = GTK_WIDGET_CLASS(klass);

    object_class->dispose = gtkterm_sunshine_bar_chart_dispose;
    widget_class->snapshot = gtkterm_sunshine_bar_chart_snapshot;
}

// Initialize a new bar chart
static void gtkterm_sunshine_bar_chart_init(GtkTermSunshineBarChart *self) {
    self->widget_id = NULL;
    self->title = g_strdup("Bar Chart");
    self->series_list = NULL;
    self->categories = NULL;
    self->n_categories = 0;
    self->y_min = 0.0;
    self->y_max = 100.0;         // Default range
    self->value_counter = 0;
}

// Clean up when the widget is destroyed
static void gtkterm_sunshine_bar_chart_dispose(GObject *object) {
    GtkTermSunshineBarChart *self = GTKTERM_SUNSHINE_BAR_CHART(object);

    g_free(self->widget_id);
    self->widget_id = NULL;

    g_free(self->title);
    self->title = NULL;

    if (self->categories) {
        g_strfreev(self->categories);
        self->categories = NULL;
    }

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineBarSeries *series = (GtkTermSunshineBarSeries *)l->data;
        g_free(series->name);
        g_array_free(series->values, TRUE);
        g_free(series);
    }
    g_list_free(self->series_list);
    self->series_list = NULL;

    G_OBJECT_CLASS(gtkterm_sunshine_bar_chart_parent_class)->dispose(object);
}

// Create a new bar chart widget
GtkTermSunshineBarChart *gtkterm_sunshine_bar_chart_new(void) {
    return g_object_new(GTKTERM_TYPE_SUNSHINE_BAR_CHART, NULL);
}

// Widget ID
void gtkterm_sunshine_bar_chart_set_widget_id(GtkTermSunshineBarChart *self, const gchar *widget_id) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));
    g_free(self->widget_id);
    self->widget_id = g_strdup(widget_id);
}

const gchar *gtkterm_sunshine_bar_chart_get_widget_id(GtkTermSunshineBarChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self), NULL);
    return self->widget_id;
}

// Title
void gtkterm_sunshine_bar_chart_set_title(GtkTermSunshineBarChart *self, const gchar *title) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));
    g_free(self->title);
    self->title = g_strdup(title);
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

const gchar *gtkterm_sunshine_bar_chart_get_title(GtkTermSunshineBarChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self), NULL);
    return self->title;
}

// Find series by name
static GtkTermSunshineBarSeries *find_series(GtkTermSunshineBarChart *self, const gchar *name) {
    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineBarSeries *series = (GtkTermSunshineBarSeries *)l->data;
        if (g_strcmp0(series->name, name) == 0) {
            return series;
        }
    }
    return NULL;
}

// Add series
void gtkterm_sunshine_bar_chart_add_series(GtkTermSunshineBarChart *self,
                                    const gchar *series_name,
                                    GtkTermSunshineColor color) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));
    g_return_if_fail(series_name != NULL);

    if (find_series(self, series_name) != NULL) {
        return;  // Series already exists
    }

    GtkTermSunshineBarSeries *series = g_new0(GtkTermSunshineBarSeries, 1);
    series->name = g_strdup(series_name);
    series->color = color;
    series->values = g_array_new(FALSE, FALSE, sizeof(gdouble));
    series->visible = TRUE;

    self->series_list = g_list_append(self->series_list, series);
}

// Add data point
void gtkterm_sunshine_bar_chart_add_data_point(GtkTermSunshineBarChart *self,
                                        const gchar *series_name,
                                        gdouble value) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));
    g_return_if_fail(series_name != NULL);

    GtkTermSunshineBarSeries *series = find_series(self, series_name);
    if (series == NULL) {
        g_warning("Series '%s' not found", series_name);
        return;
    }

    g_array_append_val(series->values, value);

    // Limit data points
    if (series->values->len > MAX_DATA_POINTS) {
        g_array_remove_index(series->values, 0);
    }

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Clear series
void gtkterm_sunshine_bar_chart_clear_series(GtkTermSunshineBarChart *self,
                                      const gchar *series_name) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));

    GtkTermSunshineBarSeries *series = find_series(self, series_name);
    if (series != NULL) {
        g_array_remove_range(series->values, 0, series->values->len);
        gtk_widget_queue_draw(GTK_WIDGET(self));
    }
}

// Clear all
void gtkterm_sunshine_bar_chart_clear_all(GtkTermSunshineBarChart *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineBarSeries *series = (GtkTermSunshineBarSeries *)l->data;
        g_array_remove_range(series->values, 0, series->values->len);
    }

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Set series visibility
void gtkterm_sunshine_bar_chart_set_series_visible(GtkTermSunshineBarChart *self,
                                            const gchar *series_name,
                                            gboolean visible) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));

    GtkTermSunshineBarSeries *series = find_series(self, series_name);
    if (series != NULL) {
        series->visible = visible;
        gtk_widget_queue_draw(GTK_WIDGET(self));
    }
}

// Set categories
void gtkterm_sunshine_bar_chart_set_categories(GtkTermSunshineBarChart *self,
                                        const gchar **categories,
                                        gsize n_categories) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));

    if (self->categories) {
        g_strfreev(self->categories);
    }

    self->categories = g_strdupv((gchar **)categories);
    self->n_categories = n_categories;

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Handle incoming data packet
void gtkterm_sunshine_bar_chart_handle_packet(GtkTermSunshineBarChart *self,
                                       GtkTermSunshineDataPacket *packet) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_BAR_CHART(self));
    g_return_if_fail(packet != NULL);

    for (GList *l = packet->series; l != NULL; l = l->next) {
        GtkTermSunshineDataSeries *data_series = (GtkTermSunshineDataSeries *)l->data;

        // Auto-create series if it doesn't exist
        GtkTermSunshineBarSeries *chart_series = find_series(self, data_series->name);
        if (chart_series == NULL) {
            gtkterm_sunshine_bar_chart_add_series(self, data_series->name, data_series->color);
            chart_series = find_series(self, data_series->name);
        }

        // Add values from packet
        if (data_series->values->len > 0) {
            gdouble value = g_array_index(data_series->values, gdouble, 0);
            gtkterm_sunshine_bar_chart_add_data_point(self, data_series->name, value);
        }
    }
}

// Calculate Y axis range
static void calculate_y_range(GtkTermSunshineBarChart *self) {
    if (self->series_list == NULL) {
        self->y_min = 0.0;
        self->y_max = 100.0;
        return;
    }

    gboolean first = TRUE;
    gdouble min_val = 0.0;
    gdouble max_val = 0.0;

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineBarSeries *series = (GtkTermSunshineBarSeries *)l->data;

        if (!series->visible || series->values->len == 0) {
            continue;
        }

        for (guint i = 0; i < series->values->len; i++) {
            gdouble val = g_array_index(series->values, gdouble, i);

            if (first) {
                min_val = val;
                max_val = val;
                first = FALSE;
            } else {
                if (val < min_val) min_val = val;
                if (val > max_val) max_val = val;
            }
        }
    }

    if (first) {
        self->y_min = 0.0;
        self->y_max = 100.0;
        return;
    }

    // Add margin
    gdouble range = max_val - min_val;
    if (range < 0.001) {
        range = max_val * 0.1;
        if (range < 0.001) {
            range = 1.0;
        }
    }

    self->y_min = min_val - range * 0.1;
    self->y_max = max_val + range * 0.1;

    // Ensure min is at or below 0 for typical bar charts
    if (self->y_min > 0.0) {
        self->y_min = 0.0;
    }
}

// Draw grid and axes
static void draw_grid_and_axes(cairo_t *cr, int width, int height,
                               gdouble y_min, gdouble y_max) {
    const int margin = 60;
    const int plot_width = width - 2 * margin;
    const int plot_height = height - 2 * margin;

    // Grid
    cairo_set_source_rgb(cr, 0.9, 0.9, 0.9);
    cairo_set_line_width(cr, 1.0);

    for (int i = 0; i <= 10; i++) {
        double y = margin + (plot_height * i / 10.0);
        cairo_move_to(cr, margin, y);
        cairo_line_to(cr, width - margin, y);
    }
    cairo_stroke(cr);

    // Axes
    cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
    cairo_set_line_width(cr, 2.0);

    // Y axis
    cairo_move_to(cr, margin, margin);
    cairo_line_to(cr, margin, height - margin);

    // X axis
    cairo_move_to(cr, margin, height - margin);
    cairo_line_to(cr, width - margin, height - margin);

    cairo_stroke(cr);

    // Y axis labels
    cairo_set_font_size(cr, 10.0);
    cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL, CAIRO_FONT_WEIGHT_NORMAL);

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
}

// Draw bars
static void draw_bars(cairo_t *cr, GtkTermSunshineBarChart *self,
                     int width, int height) {
    const int margin = 60;
    const int plot_width = width - 2 * margin;
    const int plot_height = height - 2 * margin;

    if (self->series_list == NULL) {
        return;
    }

    // Count visible series and max data points
    gint n_series = 0;
    guint max_points = 0;

    for (GList *l = self->series_list; l != NULL; l = l->next) {
        GtkTermSunshineBarSeries *series = (GtkTermSunshineBarSeries *)l->data;
        if (series->visible) {
            n_series++;
            if (series->values->len > max_points) {
                max_points = series->values->len;
            }
        }
    }

    if (n_series == 0 || max_points == 0) {
        return;
    }

    // Calculate bar dimensions
    double group_width = (double)plot_width / max_points;
    double bar_width = group_width / (n_series + 1);  // +1 for spacing

    // Draw bars for each data point
    for (guint point_idx = 0; point_idx < max_points; point_idx++) {
        gint series_idx = 0;

        for (GList *l = self->series_list; l != NULL; l = l->next) {
            GtkTermSunshineBarSeries *series = (GtkTermSunshineBarSeries *)l->data;

            if (!series->visible || point_idx >= series->values->len) {
                if (series->visible) {
                    series_idx++;
                }
                continue;
            }

            gdouble value = g_array_index(series->values, gdouble, point_idx);

            // Calculate bar position and height
            double x = margin + point_idx * group_width + series_idx * bar_width;
            double y_range = self->y_max - self->y_min;
            double zero_y = height - margin - ((0.0 - self->y_min) / y_range) * plot_height;
            double bar_height = ((value - self->y_min) / y_range) * plot_height;
            double y = height - margin - bar_height;

            // Set color
            if (series->color.has_color) {
                cairo_set_source_rgb(cr, series->color.r / 255.0,
                                   series->color.g / 255.0,
                                   series->color.b / 255.0);
            } else {
                // Default colors
                double hue = (series_idx * 360.0 / n_series) / 360.0;
                cairo_set_source_rgb(cr, hue, 0.7, 0.7);
            }

            // Draw bar
            cairo_rectangle(cr, x, y, bar_width * 0.9, bar_height);
            cairo_fill(cr);

            // Draw border
            cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
            cairo_set_line_width(cr, 1.0);
            cairo_rectangle(cr, x, y, bar_width * 0.9, bar_height);
            cairo_stroke(cr);

            series_idx++;
        }
    }
}

// Main drawing function
static void draw_chart(GtkTermSunshineBarChart *self, cairo_t *cr, int width, int height) {
    // Background
    cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
    cairo_paint(cr);

    // Calculate ranges
    calculate_y_range(self);

    // Draw grid and axes
    draw_grid_and_axes(cr, width, height, self->y_min, self->y_max);

    // Draw title
    if (self->title) {
        cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
        cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL, CAIRO_FONT_WEIGHT_BOLD);
        cairo_set_font_size(cr, 16.0);

        cairo_text_extents_t extents;
        cairo_text_extents(cr, self->title, &extents);

        cairo_move_to(cr, (width - extents.width) / 2, 30);
        cairo_show_text(cr, self->title);
    }

    // Draw bars
    draw_bars(cr, self, width, height);
}

// GTK4 snapshot method for rendering
static void gtkterm_sunshine_bar_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot) {
    GtkTermSunshineBarChart *self = GTKTERM_SUNSHINE_BAR_CHART(widget);

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
