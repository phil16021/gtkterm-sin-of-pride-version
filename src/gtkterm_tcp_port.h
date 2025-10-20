/************************************************************************/
/* gtkterm_tcp_port.h                                                   */
/* ------------------                                                   */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      TCP client/server port implementation                          */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_TCP_PORT_H
#define GTKTERM_TCP_PORT_H

#include <glib-object.h>
#include "gtkterm_port.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_TCP_PORT gtkterm_tcp_port_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermTCPPort, gtkterm_tcp_port, GTKTERM, TCP_PORT, GObject)

/**
 * @brief TCP mode
 */
typedef enum {
    GTKTERM_TCP_MODE_CLIENT,  /**< TCP client (connect to server) */
    GTKTERM_TCP_MODE_SERVER   /**< TCP server (listen for connections) */
} GtkTermTCPMode;

/**
 * @brief Create a new TCP port
 */
GtkTermTCPPort *gtkterm_tcp_port_new (GtkTermTCPMode mode,
                                       const char *host,
                                       guint16 port);

/**
 * @brief Get remote address
 */
const char *gtkterm_tcp_port_get_remote_address (GtkTermTCPPort *self);

/**
 * @brief Get number of connected clients (server mode only)
 */
guint gtkterm_tcp_port_get_client_count (GtkTermTCPPort *self);

G_END_DECLS

#endif // GTKTERM_TCP_PORT_H
