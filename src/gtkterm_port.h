/************************************************************************/
/* gtkterm_port.h                                                       */
/* --------------                                                       */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Abstract port interface (serial, TCP, UDP)                     */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_PORT_H
#define GTKTERM_PORT_H

#include <glib-object.h>
#include <gio/gio.h>

G_BEGIN_DECLS

/**
 * @brief Port type
 */
typedef enum {
    GTKTERM_PORT_TYPE_SERIAL,      /**< Serial port (RS232/RS485) */
    GTKTERM_PORT_TYPE_TCP_CLIENT,  /**< TCP client connection */
    GTKTERM_PORT_TYPE_TCP_SERVER,  /**< TCP server (listening) */
    GTKTERM_PORT_TYPE_UDP          /**< UDP socket */
} GtkTermPortType;

/**
 * @brief Port state
 */
typedef enum {
    GTKTERM_PORT_STATE_CLOSED,
    GTKTERM_PORT_STATE_OPENING,
    GTKTERM_PORT_STATE_OPEN,
    GTKTERM_PORT_STATE_CLOSING,
    GTKTERM_PORT_STATE_ERROR
} GtkTermPortState;

/**
 * @brief Port statistics
 */
typedef struct {
    guint64 bytes_sent;
    guint64 bytes_received;
    guint64 frames_sent;
    guint64 frames_received;
    guint64 errors;
    GDateTime *connected_time;
} GtkTermPortStats;

#define GTKTERM_TYPE_PORT gtkterm_port_get_type ()
G_DECLARE_INTERFACE (GtkTermPort, gtkterm_port, GTKTERM, PORT, GObject)

/**
 * @brief Port interface
 */
struct _GtkTermPortInterface {
    GTypeInterface parent_iface;

    /* Virtual methods */
    const char * (*get_name) (GtkTermPort *self);
    GtkTermPortType (*get_port_type) (GtkTermPort *self);
    GtkTermPortState (*get_state) (GtkTermPort *self);

    gboolean (*open) (GtkTermPort *self, GError **error);
    void (*close) (GtkTermPort *self);

    gssize (*write) (GtkTermPort *self, const guint8 *data, gsize length, GError **error);
    gssize (*read) (GtkTermPort *self, guint8 *buffer, gsize length, GError **error);

    void (*get_stats) (GtkTermPort *self, GtkTermPortStats *stats);
    void (*reset_stats) (GtkTermPort *self);

    /* Signals */
    void (*data_received) (GtkTermPort *self, const guint8 *data, gsize length);
    void (*state_changed) (GtkTermPort *self, GtkTermPortState old_state, GtkTermPortState new_state);
    void (*error_occurred) (GtkTermPort *self, const char *error_message);
};

/* Interface methods */
const char *gtkterm_port_get_name (GtkTermPort *self);
GtkTermPortType gtkterm_port_get_port_type (GtkTermPort *self);
GtkTermPortState gtkterm_port_get_state (GtkTermPort *self);

gboolean gtkterm_port_open (GtkTermPort *self, GError **error);
void gtkterm_port_close (GtkTermPort *self);

gssize gtkterm_port_write (GtkTermPort *self, const guint8 *data, gsize length, GError **error);
gssize gtkterm_port_read (GtkTermPort *self, guint8 *buffer, gsize length, GError **error);

void gtkterm_port_get_stats (GtkTermPort *self, GtkTermPortStats *stats);
void gtkterm_port_reset_stats (GtkTermPort *self);

/* Helper functions */
void gtkterm_port_stats_init (GtkTermPortStats *stats);
void gtkterm_port_stats_clear (GtkTermPortStats *stats);

G_END_DECLS

#endif // GTKTERM_PORT_H
