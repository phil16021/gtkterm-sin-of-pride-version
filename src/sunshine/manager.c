/*
 * Sunshine Visualization Manager
 *
 * This is basically the main controller for my visualization feature.
 * It manages all the charts and connects everything together.
 */

#include <stdbool.h>
#include <string.h>
#include "manager.h"
#include "scanner.h"
#include "line_chart.h"
#include "bar_chart.h"
#include "pie_chart.h"
#include "scatter_chart.h"
#include "../gtkterm_defaults.h"
#include "../gtkterm_serial_port.h"
#include "../gtkterm_terminal.h"
#include "../gtkterm_buffer.h"

struct _GtkTermSunshineManager {
    GObject parent_instance;

    // UI stuff
    GtkWidget *notebook;              // The notebook widget where we put all the chart tabs

    // Data handling
    GtkTermBuffer *buffer;            // The serial data buffer we're watching
    GtkTermSunshineScanner *scanner; // Scanner that finds packets in the data

    // Chart tracking - using a hash table seemed like the smartest way to do this
    GHashTable *charts;               // Maps packet IDs to their chart widgets

    // Keep track of where we are
    guint last_scanned_chunk_index;   // So we don't re-scan the same data over and over
    guint total_packets_processed;    // Just for debugging really

    // Signal connection stuff
    gulong buffer_signal_handler;     // Need to save this so we can disconnect later
};

G_DEFINE_TYPE(GtkTermSunshineManager, gtkterm_sunshine_manager, G_TYPE_OBJECT)

// Forward declarations
static void on_packet_found(GtkTermSunshineDataPacket *packet, gpointer user_data);
static void on_buffer_data_received(GtkTermBuffer *buffer, gpointer data, guint length, gpointer user_data);
static GtkWidget *create_chart_for_packet(GtkTermSunshineManager *self,
                                           GtkTermSunshineDataPacket *packet);
static void route_packet_to_chart(GtkTermSunshineManager *self,
                                  GtkTermSunshineDataPacket *packet);
static void on_renew_button_clicked(GtkButton *button, gpointer user_data);

static void gtkterm_sunshine_manager_dispose(GObject *object) {
    GtkTermSunshineManager *self = GTKTERM_SUNSHINE_MANAGER(object);

    // Disconnect buffer signal
    if (self->buffer && self->buffer_signal_handler > 0) {
        g_signal_handler_disconnect(self->buffer, self->buffer_signal_handler);
        self->buffer_signal_handler = 0;
    }

    // Clean up scanner
    if (self->scanner) {
        g_object_unref(self->scanner);
        self->scanner = NULL;
    }

    // Clean up charts hash table
    if (self->charts) {
        g_hash_table_destroy(self->charts);
        self->charts = NULL;
    }

    G_OBJECT_CLASS(gtkterm_sunshine_manager_parent_class)->dispose(object);
}

static void gtkterm_sunshine_manager_class_init(GtkTermSunshineManagerClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = gtkterm_sunshine_manager_dispose;
}

static void gtkterm_sunshine_manager_init(GtkTermSunshineManager *self) {
    self->notebook = NULL;
    self->buffer = NULL;
    self->scanner = gtkterm_sunshine_scanner_new();
    self->charts = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    self->last_scanned_chunk_index = 0;
    self->total_packets_processed = 0;
    self->buffer_signal_handler = 0;

    // Hook up the scanner so it calls us when it finds packets
    gtkterm_sunshine_scanner_set_callback(self->scanner, on_packet_found, self);
}

GtkTermSunshineManager *gtkterm_sunshine_manager_new(GtkWidget *notebook,
                                                        GtkTermBuffer *buffer) {
    GtkTermSunshineManager *self = g_object_new(GTKTERM_TYPE_SUNSHINE_MANAGER, NULL);

    self->notebook = notebook;
    self->buffer = buffer;

    // This is the important part - we listen for new data coming in
    // Took me a while to figure out which signal to use!
    if (buffer) {
        self->buffer_signal_handler = g_signal_connect(buffer, "buffer-updated",
                                                       G_CALLBACK(on_buffer_data_received), self);
        g_print("Sunshine: Connected to buffer-updated signal\n");
    }

    return self;
}

void gtkterm_sunshine_manager_scan_history(GtkTermSunshineManager *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_MANAGER(self));
    g_return_if_fail(self->buffer != NULL);

    g_print("Sunshine: Scanning buffer history for packets...\n");

    // Go through all the existing data to find any packets
    guint packets_found = gtkterm_sunshine_scanner_scan_buffer(self->scanner, self->buffer);

    // Remember where we left off so we don't re-scan everything later
    GPtrArray *chunks = gtkterm_buffer_get_data_chunks(self->buffer);
    if (chunks) {
        self->last_scanned_chunk_index = chunks->len;
    }

    g_print("Sunshine: Found %u packets in history\n", packets_found);
}

void gtkterm_sunshine_manager_clear_all(GtkTermSunshineManager *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_MANAGER(self));

    g_print("Sunshine: Clearing all charts\n");

    // Remove all chart tabs except the welcome tab (index 0)
    gint n_pages = gtk_notebook_get_n_pages(GTK_NOTEBOOK(self->notebook));
    for (gint i = n_pages - 1; i > 0; i--) {
        gtk_notebook_remove_page(GTK_NOTEBOOK(self->notebook), i);
    }

    // Clear charts hash table
    g_hash_table_remove_all(self->charts);

    // Reset statistics
    self->total_packets_processed = 0;
    gtkterm_sunshine_scanner_reset(self->scanner);
}

void gtkterm_sunshine_manager_reset(GtkTermSunshineManager *self) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_MANAGER(self));

    g_print("Sunshine: Resetting visualization from current point\n");

    // Clear all charts
    gtkterm_sunshine_manager_clear_all(self);

    // Update scan position to current buffer end
    GPtrArray *chunks = gtkterm_buffer_get_data_chunks(self->buffer);
    if (chunks) {
        self->last_scanned_chunk_index = chunks->len;
    }
}

void gtkterm_sunshine_manager_get_stats(GtkTermSunshineManager *self,
                                         guint *total_packets,
                                         guint *total_charts) {
    g_return_if_fail(GTKTERM_IS_SUNSHINE_MANAGER(self));

    if (total_packets) {
        *total_packets = self->total_packets_processed;
    }
    if (total_charts) {
        *total_charts = g_hash_table_size(self->charts);
    }
}

/**
 * Callback when scanner finds a packet
 */
static void on_packet_found(GtkTermSunshineDataPacket *packet, gpointer user_data) {
    GtkTermSunshineManager *self = GTKTERM_SUNSHINE_MANAGER(user_data);

    self->total_packets_processed++;

    g_print("Sunshine: Packet found - ID: %s, Series: %u\n",
            packet->id ? packet->id : "(null)",
            g_list_length(packet->series));

    // Route packet to appropriate chart (creating if needed)
    route_packet_to_chart(self, packet);

    // Free the packet
    gtkterm_sunshine_packet_free(packet);
}

// Figure out which chart this packet belongs to and send it there
static void route_packet_to_chart(GtkTermSunshineManager *self,
                                  GtkTermSunshineDataPacket *packet) {
    if (!packet->id) {
        g_warning("Sunshine: Packet has no ID, ignoring");
        return;
    }

    // Check if we already have a chart for this packet ID
    GtkWidget *chart = g_hash_table_lookup(self->charts, packet->id);

    if (!chart) {
        // First time seeing this packet ID - make a new chart for it
        chart = create_chart_for_packet(self, packet);
        if (!chart) {
            g_warning("Sunshine: Failed to create chart for packet %s", packet->id);
            return;
        }

        // Save it so we can find it again later
        g_hash_table_insert(self->charts, g_strdup(packet->id), chart);
    }

    // Send the data to the right chart type
    // TODO: maybe there's a cleaner way to do this with inheritance?
    if (GTKTERM_IS_SUNSHINE_LINE_CHART(chart)) {
        gtkterm_sunshine_line_chart_handle_packet(
            GTKTERM_SUNSHINE_LINE_CHART(chart), packet);
    } else if (GTKTERM_IS_SUNSHINE_BAR_CHART(chart)) {
        gtkterm_sunshine_bar_chart_handle_packet(
            GTKTERM_SUNSHINE_BAR_CHART(chart), packet);
    } else if (GTKTERM_IS_SUNSHINE_PIE_CHART(chart)) {
        gtkterm_sunshine_pie_chart_handle_packet(
            GTKTERM_SUNSHINE_PIE_CHART(chart), packet);
    } else if (GTKTERM_IS_SUNSHINE_SCATTER_CHART(chart)) {
        gtkterm_sunshine_scatter_chart_handle_packet(
            GTKTERM_SUNSHINE_SCATTER_CHART(chart), packet);
    }
}

/**
 * Create appropriate chart widget for packet
 */
static GtkWidget *create_chart_for_packet(GtkTermSunshineManager *self,
                                           GtkTermSunshineDataPacket *packet) {
    GtkWidget *chart = NULL;
    GtkWidget *scrolled_window;
    GtkWidget *tab_label;
    GtkWidget *container_box;
    GtkWidget *button_box;
    GtkWidget *renew_button;

    /* For now, default to line chart
     * TODO: Detect chart type from packet format */
    chart = GTK_WIDGET(gtkterm_sunshine_line_chart_new());

    if (!chart) {
        return NULL;
    }

    // Set widget ID and title
    gtkterm_sunshine_line_chart_set_widget_id(
        GTKTERM_SUNSHINE_LINE_CHART(chart), packet->id);
    gtkterm_sunshine_line_chart_set_title(
        GTKTERM_SUNSHINE_LINE_CHART(chart), packet->id);

    // Wrap chart in scrolled window
    scrolled_window = gtk_scrolled_window_new();
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(scrolled_window), chart);
    gtk_widget_set_vexpand(scrolled_window, TRUE);
    gtk_widget_set_hexpand(scrolled_window, TRUE);

    // Create button bar
    button_box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 5);
    gtk_widget_set_margin_start(button_box, 5);
    gtk_widget_set_margin_end(button_box, 5);
    gtk_widget_set_margin_top(button_box, 5);
    gtk_widget_set_margin_bottom(button_box, 5);

    // Create Renew button
    renew_button = gtk_button_new_with_label("Renew");
    gtk_widget_set_tooltip_text(renew_button, "Clear all data from this chart");
    g_signal_connect(renew_button, "clicked", G_CALLBACK(on_renew_button_clicked), chart);
    gtk_box_append(GTK_BOX(button_box), renew_button);

    // Create container box: scrolled chart above, buttons below
    container_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_box_append(GTK_BOX(container_box), scrolled_window);
    gtk_box_append(GTK_BOX(container_box), button_box);

    // Create tab label
    tab_label = gtk_label_new(packet->id);

    // Add to notebook
    gtk_notebook_append_page(GTK_NOTEBOOK(self->notebook),
                            container_box,
                            tab_label);

    gtk_widget_set_visible(container_box, TRUE);
    gtk_widget_set_visible(scrolled_window, TRUE);
    gtk_widget_set_visible(button_box, TRUE);
    gtk_widget_set_visible(renew_button, TRUE);
    gtk_widget_set_visible(chart, TRUE);

    g_print("Sunshine: Created new line chart for packet ID: %s\n", packet->id);

    return chart;
}

/**
 * Callback for new data in buffer (for real-time updates)
 */
static void on_buffer_data_received(GtkTermBuffer *buffer, gpointer data, guint length, gpointer user_data) {
    GtkTermSunshineManager *self = GTKTERM_SUNSHINE_MANAGER(user_data);

    g_print("Sunshine: Buffer updated with %u bytes of new data\n", length);

    // Scan new data incrementally
    GPtrArray *chunks = gtkterm_buffer_get_data_chunks(self->buffer);
    if (chunks && chunks->len > self->last_scanned_chunk_index) {
        guint packets_found = gtkterm_sunshine_scanner_scan_from_index(self->scanner,
                                                                        self->buffer,
                                                                        self->last_scanned_chunk_index);
        self->last_scanned_chunk_index = chunks->len;

        if (packets_found > 0) {
            g_print("Sunshine: Real-time scan found %u packets\n", packets_found);
        }
    }
}

/**
 * Callback for Renew button click
 */
static void on_renew_button_clicked(GtkButton *button, gpointer user_data) {
    GtkWidget *chart = GTK_WIDGET(user_data);

    // Check chart type and call appropriate clear function
    if (GTKTERM_IS_SUNSHINE_LINE_CHART(chart)) {
        const gchar *chart_id = gtkterm_sunshine_line_chart_get_widget_id(
            GTKTERM_SUNSHINE_LINE_CHART(chart));
        g_print("Sunshine: Renewing chart: %s\n", chart_id ? chart_id : "(unknown)");
        gtkterm_sunshine_line_chart_clear_all(GTKTERM_SUNSHINE_LINE_CHART(chart));
    } else if (GTKTERM_IS_SUNSHINE_BAR_CHART(chart)) {
        gtkterm_sunshine_bar_chart_clear_all(GTKTERM_SUNSHINE_BAR_CHART(chart));
    } else if (GTKTERM_IS_SUNSHINE_PIE_CHART(chart)) {
        gtkterm_sunshine_pie_chart_clear_all(GTKTERM_SUNSHINE_PIE_CHART(chart));
    } else if (GTKTERM_IS_SUNSHINE_SCATTER_CHART(chart)) {
        gtkterm_sunshine_scatter_chart_clear_all(GTKTERM_SUNSHINE_SCATTER_CHART(chart));
    }
}
