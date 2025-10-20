/*
 * Sunshine Pie Chart Widget
 *
 * Pie charts for showing proportions/percentages.
 * The trig for drawing the slices was a bit tricky to get right!
 */

#include "pie_chart.h"
#include <cairo.h>
#include <math.h>

#ifndef M_PI
#define M_PI 3.14159265358979323846  // Just in case it's not defined
#endif

struct _GtkTermSunshinePieChart {
    GtkWidget parent_instance;

    gchar *widget_id;
    gchar *title;
    GList *slices;             // All the pie slices to draw

    gboolean show_labels;      // Whether to show slice labels
    gboolean show_percentages; // Whether to show percentage values
};

G_DEFINE_TYPE(GtkTermSunshinePieChart, gtkterm_sunshine_pie_chart, GTK_TYPE_WIDGET)

// Helper functions
static void gtkterm_sunshine_pie_chart_dispose(GObject *object);
static void gtkterm_sunshine_pie_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot);
static void draw_chart(GtkTermSunshinePieChart *self, cairo_t *cr, int width, int height);
static GtkTermSunshinePieSlice *find_slice(GtkTermSunshinePieChart *self, const gchar *name);

// GObject setup
static void gtkterm_sunshine_pie_chart_class_init(GtkTermSunshinePieChartClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    GtkWidgetClass *widget_class = GTK_WIDGET_CLASS(klass);

    object_class->dispose = gtkterm_sunshine_pie_chart_dispose;
    widget_class->snapshot = gtkterm_sunshine_pie_chart_snapshot;
}

// Initialize a new pie chart
static void gtkterm_sunshine_pie_chart_init(GtkTermSunshinePieChart *self) {
    self->widget_id = NULL;
    self->title = g_strdup("Pie Chart");
    self->slices = NULL;
    self->show_labels = TRUE;       // Show labels by default
    self->show_percentages = TRUE;  // Show percentages by default
}

// Clean up when destroyed
static void gtkterm_sunshine_pie_chart_dispose(GObject *object) {
    GtkTermSunshinePieChart *self = GTKTERM_SUNSHINE_PIE_CHART(object);

    g_free(self->widget_id);
    self->widget_id = NULL;

    g_free(self->title);
    self->title = NULL;

    for (GList *l = self->slices; l != NULL; l = l->next) {
        GtkTermSunshinePieSlice *slice = (GtkTermSunshinePieSlice *)l->data;
        g_free(slice->name);
        g_free(slice);
    }
    g_list_free(self->slices);
    self->slices = NULL;

    G_OBJECT_CLASS(gtkterm_sunshine_pie_chart_parent_class)->dispose(object);
}

// Create a new pie chart widget
GtkTermSunshinePieChart *gtkterm_sunshine_pie_chart_new(void) {
    return g_object_new(GTKTERM_TYPE_SUNSHINE_PIE_CHART, NULL);
}

// Widget ID
void gtkterm_sunshine_pie_chart_set_widget_id(GtkTermSunshinePieChart *self, const gchar *widget_id) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));
    g_free(self->widget_id);
    self->widget_id = g_strdup(widget_id);
}

const gchar *gtkterm_sunshine_pie_chart_get_widget_id(GtkTermSunshinePieChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self), NULL);
    return self->widget_id;
}

// Title
void gtkterm_sunshine_pie_chart_set_title(GtkTermSunshinePieChart *self, const gchar *title) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));
    g_free(self->title);
    self->title = g_strdup(title);
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

const gchar *gtkterm_sunshine_pie_chart_get_title(GtkTermSunshinePieChart *self) {
    g_return_val_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self), NULL);
    return self->title;
}

// Find slice by name
static GtkTermSunshinePieSlice *find_slice(GtkTermSunshinePieChart *self, const gchar *name) {
    for (GList *l = self->slices; l != NULL; l = l->next) {
        GtkTermSunshinePieSlice *slice = (GtkTermSunshinePieSlice *)l->data;
        if (g_strcmp0(slice->name, name) == 0) {
            return slice;
        }
    }
    return NULL;
}

// Set slice data
void gtkterm_sunshine_pie_chart_set_slice(GtkTermSunshinePieChart *self,
                                   const gchar *slice_name,
                                   gdouble value,
                                   GtkTermSunshineColor color) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));
    g_return_if_fail(slice_name != NULL);

    GtkTermSunshinePieSlice *slice = find_slice(self, slice_name);

    if (slice == NULL) {
        // Create new slice
        slice = g_new0(GtkTermSunshinePieSlice, 1);
        slice->name = g_strdup(slice_name);
        slice->visible = TRUE;
        self->slices = g_list_append(self->slices, slice);
    }

    slice->value = value;
    slice->color = color;

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Remove slice
void gtkterm_sunshine_pie_chart_remove_slice(GtkTermSunshinePieChart *self,
                                      const gchar *slice_name) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));

    GtkTermSunshinePieSlice *slice = find_slice(self, slice_name);
    if (slice != NULL) {
        self->slices = g_list_remove(self->slices, slice);
        g_free(slice->name);
        g_free(slice);
        gtk_widget_queue_draw(GTK_WIDGET(self));
    }
}

// Clear all
void gtkterm_sunshine_pie_chart_clear_all(GtkTermSunshinePieChart *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));

    for (GList *l = self->slices; l != NULL; l = l->next) {
        GtkTermSunshinePieSlice *slice = (GtkTermSunshinePieSlice *)l->data;
        g_free(slice->name);
        g_free(slice);
    }

    g_list_free(self->slices);
    self->slices = NULL;

    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Set slice visibility
void gtkterm_sunshine_pie_chart_set_slice_visible(GtkTermSunshinePieChart *self,
                                           const gchar *slice_name,
                                           gboolean visible) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));

    GtkTermSunshinePieSlice *slice = find_slice(self, slice_name);
    if (slice != NULL) {
        slice->visible = visible;
        gtk_widget_queue_draw(GTK_WIDGET(self));
    }
}

// Display options
void gtkterm_sunshine_pie_chart_set_show_labels(GtkTermSunshinePieChart *self, gboolean show) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));
    self->show_labels = show;
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

void gtkterm_sunshine_pie_chart_set_show_percentages(GtkTermSunshinePieChart *self, gboolean show) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));
    self->show_percentages = show;
    gtk_widget_queue_draw(GTK_WIDGET(self));
}

// Handle incoming data packet
void gtkterm_sunshine_pie_chart_handle_packet(GtkTermSunshinePieChart *self,
                                       GtkTermSunshineDataPacket *packet) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_PIE_CHART(self));
    g_return_if_fail(packet != NULL);

    for (GList *l = packet->series; l != NULL; l = l->next) {
        GtkTermSunshineDataSeries *data_series = (GtkTermSunshineDataSeries *)l->data;

        // Get latest value (for pie chart, we use the most recent value)
        if (data_series->values->len > 0) {
            gdouble value = g_array_index(data_series->values, gdouble,
                                         data_series->values->len - 1);

            gtkterm_sunshine_pie_chart_set_slice(self, data_series->name, value,
                                         data_series->color);
        }
    }
}

// Calculate total of visible slices
static gdouble calculate_total(GtkTermSunshinePieChart *self) {
    gdouble total = 0.0;

    for (GList *l = self->slices; l != NULL; l = l->next) {
        GtkTermSunshinePieSlice *slice = (GtkTermSunshinePieSlice *)l->data;
        if (slice->visible && slice->value > 0.0) {
            total += slice->value;
        }
    }

    return total;
}

// Draw pie slices
static void draw_pie_slices(cairo_t *cr, GtkTermSunshinePieChart *self,
                           double center_x, double center_y, double radius) {
    gdouble total = calculate_total(self);

    if (total <= 0.0) {
        // Draw empty circle
        cairo_set_source_rgb(cr, 0.9, 0.9, 0.9);
        cairo_arc(cr, center_x, center_y, radius, 0, 2 * M_PI);
        cairo_fill(cr);

        cairo_set_source_rgb(cr, 0.5, 0.5, 0.5);
        cairo_arc(cr, center_x, center_y, radius, 0, 2 * M_PI);
        cairo_stroke(cr);

        return;
    }

    gdouble current_angle = -M_PI / 2;  // Start at top
    gint slice_idx = 0;

    for (GList *l = self->slices; l != NULL; l = l->next) {
        GtkTermSunshinePieSlice *slice = (GtkTermSunshinePieSlice *)l->data;

        if (!slice->visible || slice->value <= 0.0) {
            continue;
        }

        gdouble slice_angle = (slice->value / total) * 2 * M_PI;
        gdouble end_angle = current_angle + slice_angle;

        // Set color
        if (slice->color.has_color) {
            cairo_set_source_rgb(cr, slice->color.r / 255.0,
                               slice->color.g / 255.0,
                               slice->color.b / 255.0);
        } else {
            // Generate color based on index
            double hue = (slice_idx * 360.0 / g_list_length(self->slices)) / 360.0;
            double r, g, b;

            // HSV to RGB conversion (simplified)
            int h = (int)(hue * 6);
            double f = hue * 6 - h;

            switch (h % 6) {
                case 0: r = 1.0; g = f; b = 0; break;
                case 1: r = 1-f; g = 1.0; b = 0; break;
                case 2: r = 0; g = 1.0; b = f; break;
                case 3: r = 0; g = 1-f; b = 1.0; break;
                case 4: r = f; g = 0; b = 1.0; break;
                default: r = 1.0; g = 0; b = 1-f; break;
            }

            cairo_set_source_rgb(cr, r, g, b);
        }

        // Draw slice
        cairo_move_to(cr, center_x, center_y);
        cairo_arc(cr, center_x, center_y, radius, current_angle, end_angle);
        cairo_close_path(cr);
        cairo_fill_preserve(cr);

        // Draw border
        cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
        cairo_set_line_width(cr, 1.0);
        cairo_stroke(cr);

        // Draw label if enabled
        if (self->show_labels || self->show_percentages) {
            double mid_angle = current_angle + slice_angle / 2;
            double label_radius = radius * 0.7;
            double label_x = center_x + cos(mid_angle) * label_radius;
            double label_y = center_y + sin(mid_angle) * label_radius;

            cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
            cairo_set_font_size(cr, 12.0);
            cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL,
                                 CAIRO_FONT_WEIGHT_BOLD);

            gchar *label_text = NULL;
            if (self->show_labels && self->show_percentages) {
                double percentage = (slice->value / total) * 100.0;
                label_text = g_strdup_printf("%s\n%.1f%%", slice->name, percentage);
            } else if (self->show_labels) {
                label_text = g_strdup(slice->name);
            } else if (self->show_percentages) {
                double percentage = (slice->value / total) * 100.0;
                label_text = g_strdup_printf("%.1f%%", percentage);
            }

            if (label_text) {
                cairo_text_extents_t extents;
                cairo_text_extents(cr, label_text, &extents);

                cairo_move_to(cr, label_x - extents.width / 2,
                            label_y + extents.height / 2);
                cairo_show_text(cr, label_text);

                g_free(label_text);
            }
        }

        current_angle = end_angle;
        slice_idx++;
    }
}

// Draw legend
static void draw_legend(cairo_t *cr, GtkTermSunshinePieChart *self,
                       int width, int height) {
    const int legend_x = width - 150;
    const int legend_y = 60;
    const int box_size = 15;
    const int spacing = 20;

    cairo_set_font_size(cr, 10.0);
    cairo_select_font_face(cr, "Sans", CAIRO_FONT_SLANT_NORMAL,
                         CAIRO_FONT_WEIGHT_NORMAL);

    gdouble total = calculate_total(self);
    gint y = legend_y;
    gint slice_idx = 0;

    for (GList *l = self->slices; l != NULL; l = l->next) {
        GtkTermSunshinePieSlice *slice = (GtkTermSunshinePieSlice *)l->data;

        if (!slice->visible) {
            continue;
        }

        // Draw color box
        if (slice->color.has_color) {
            cairo_set_source_rgb(cr, slice->color.r / 255.0,
                               slice->color.g / 255.0,
                               slice->color.b / 255.0);
        } else {
            double hue = (slice_idx * 360.0 / g_list_length(self->slices)) / 360.0;
            double r, g, b;

            int h = (int)(hue * 6);
            double f = hue * 6 - h;

            switch (h % 6) {
                case 0: r = 1.0; g = f; b = 0; break;
                case 1: r = 1-f; g = 1.0; b = 0; break;
                case 2: r = 0; g = 1.0; b = f; break;
                case 3: r = 0; g = 1-f; b = 1.0; break;
                case 4: r = f; g = 0; b = 1.0; break;
                default: r = 1.0; g = 0; b = 1-f; break;
            }

            cairo_set_source_rgb(cr, r, g, b);
        }

        cairo_rectangle(cr, legend_x, y, box_size, box_size);
        cairo_fill(cr);

        cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
        cairo_rectangle(cr, legend_x, y, box_size, box_size);
        cairo_stroke(cr);

        // Draw label
        gchar *label;
        if (total > 0.0) {
            double percentage = (slice->value / total) * 100.0;
            label = g_strdup_printf("%s (%.1f%%)", slice->name, percentage);
        } else {
            label = g_strdup(slice->name);
        }

        cairo_move_to(cr, legend_x + box_size + 5, y + box_size - 3);
        cairo_show_text(cr, label);

        g_free(label);

        y += spacing;
        slice_idx++;
    }
}

// Main drawing function
static void draw_chart(GtkTermSunshinePieChart *self, cairo_t *cr, int width, int height) {
    // Background
    cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
    cairo_paint(cr);

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

    // Calculate pie dimensions
    double center_x = width * 0.4;
    double center_y = height * 0.5;
    double radius = MIN(width, height) * 0.35;

    if (radius > 200) {
        radius = 200;
    }

    // Draw pie slices
    draw_pie_slices(cr, self, center_x, center_y, radius);

    // Draw legend
    draw_legend(cr, self, width, height);
}

// GTK4 snapshot method for rendering
static void gtkterm_sunshine_pie_chart_snapshot(GtkWidget *widget, GtkSnapshot *snapshot) {
    GtkTermSunshinePieChart *self = GTKTERM_SUNSHINE_PIE_CHART(widget);

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
