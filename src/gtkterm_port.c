/************************************************************************/
/* gtkterm_port.c                                                       */
/* --------------                                                       */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Abstract port interface implementation                         */
/*                                                                      */
/************************************************************************/

#include "gtkterm_port.h"

G_DEFINE_INTERFACE (GtkTermPort, gtkterm_port, G_TYPE_OBJECT)

enum {
    SIGNAL_DATA_RECEIVED,
    SIGNAL_STATE_CHANGED,
    SIGNAL_ERROR,
    N_SIGNALS
};

static guint signals[N_SIGNALS] = {0};

static void gtkterm_port_default_init (GtkTermPortInterface *iface) {
    (void)iface;

    /**
     * GtkTermPort::data-received:
     * @self: the port object
     * @data: received data
     * @length: data length
     *
     * Emitted when data is received
     */
    signals[SIGNAL_DATA_RECEIVED] = g_signal_new (
        "data-received",
        G_TYPE_FROM_INTERFACE (iface),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL,
        NULL,
        G_TYPE_NONE,
        2,
        G_TYPE_POINTER,
        G_TYPE_UINT64
    );

    /**
     * GtkTermPort::state-changed:
     * @self: the port object
     * @old_state: previous state
     * @new_state: new state
     *
     * Emitted when port state changes
     */
    signals[SIGNAL_STATE_CHANGED] = g_signal_new (
        "state-changed",
        G_TYPE_FROM_INTERFACE (iface),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL,
        NULL,
        G_TYPE_NONE,
        2,
        G_TYPE_INT,
        G_TYPE_INT
    );

    /**
     * GtkTermPort::error:
     * @self: the port object
     * @error_message: error description
     *
     * Emitted when an error occurs
     */
    signals[SIGNAL_ERROR] = g_signal_new (
        "error",
        G_TYPE_FROM_INTERFACE (iface),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL,
        NULL,
        G_TYPE_NONE,
        1,
        G_TYPE_STRING
    );
}

/**
 * @brief Get port name
 */
const char *gtkterm_port_get_name (GtkTermPort *self) {
    GtkTermPortInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PORT (self), NULL);

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_val_if_fail (iface->get_name != NULL, NULL);

    return iface->get_name (self);
}

/**
 * @brief Get port type
 */
GtkTermPortType gtkterm_port_get_port_type (GtkTermPort *self) {
    GtkTermPortInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PORT (self), GTKTERM_PORT_TYPE_SERIAL);

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_val_if_fail (iface->get_port_type != NULL, GTKTERM_PORT_TYPE_SERIAL);

    return iface->get_port_type (self);
}

/**
 * @brief Get port state
 */
GtkTermPortState gtkterm_port_get_state (GtkTermPort *self) {
    GtkTermPortInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PORT (self), GTKTERM_PORT_STATE_CLOSED);

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_val_if_fail (iface->get_state != NULL, GTKTERM_PORT_STATE_CLOSED);

    return iface->get_state (self);
}

/**
 * @brief Open port
 */
gboolean gtkterm_port_open (GtkTermPort *self, GError **error) {
    GtkTermPortInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PORT (self), FALSE);

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_val_if_fail (iface->open != NULL, FALSE);

    return iface->open (self, error);
}

/**
 * @brief Close port
 */
void gtkterm_port_close (GtkTermPort *self) {
    GtkTermPortInterface *iface;

    g_return_if_fail (GTKTERM_IS_PORT (self));

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_if_fail (iface->close != NULL);

    iface->close (self);
}

/**
 * @brief Write data to port
 */
gssize gtkterm_port_write (GtkTermPort *self, const guint8 *data, gsize length, GError **error) {
    GtkTermPortInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PORT (self), -1);
    g_return_val_if_fail (data != NULL || length == 0, -1);

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_val_if_fail (iface->write != NULL, -1);

    return iface->write (self, data, length, error);
}

/**
 * @brief Read data from port
 */
gssize gtkterm_port_read (GtkTermPort *self, guint8 *buffer, gsize length, GError **error) {
    GtkTermPortInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PORT (self), -1);
    g_return_val_if_fail (buffer != NULL || length == 0, -1);

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_val_if_fail (iface->read != NULL, -1);

    return iface->read (self, buffer, length, error);
}

/**
 * @brief Get port statistics
 */
void gtkterm_port_get_stats (GtkTermPort *self, GtkTermPortStats *stats) {
    GtkTermPortInterface *iface;

    g_return_if_fail (GTKTERM_IS_PORT (self));
    g_return_if_fail (stats != NULL);

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_if_fail (iface->get_stats != NULL);

    iface->get_stats (self, stats);
}

/**
 * @brief Reset port statistics
 */
void gtkterm_port_reset_stats (GtkTermPort *self) {
    GtkTermPortInterface *iface;

    g_return_if_fail (GTKTERM_IS_PORT (self));

    iface = GTKTERM_PORT_GET_IFACE (self);
    g_return_if_fail (iface->reset_stats != NULL);

    iface->reset_stats (self);
}

/**
 * @brief Initialize port stats structure
 */
void gtkterm_port_stats_init (GtkTermPortStats *stats) {
    g_return_if_fail (stats != NULL);

    stats->bytes_sent = 0;
    stats->bytes_received = 0;
    stats->frames_sent = 0;
    stats->frames_received = 0;
    stats->errors = 0;
    stats->connected_time = NULL;
}

/**
 * @brief Clear port stats and free resources
 */
void gtkterm_port_stats_clear (GtkTermPortStats *stats) {
    g_return_if_fail (stats != NULL);

    if (stats->connected_time) {
        g_date_time_unref (stats->connected_time);
        stats->connected_time = NULL;
    }
}
