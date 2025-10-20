/************************************************************************/
/* gtkterm_udp_port.c                                                   */
/* ------------------                                                   */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      UDP port implementation                                        */
/*                                                                      */
/************************************************************************/

#include "gtkterm_udp_port.h"
#include <string.h>

struct _GtkTermUDPPort {
    GObject parent_instance;

    /* Configuration */
    char *local_host;
    guint16 local_port;
    char *remote_host;
    guint16 remote_port;
    char *name;

    /* State */
    GtkTermPortState state;
    GtkTermPortStats stats;

    /* Socket */
    GSocket *socket;
    GSocketAddress *remote_address;
    GSource *read_source;
    guint8 read_buffer[4096];
};

static void gtkterm_udp_port_port_interface_init (GtkTermPortInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermUDPPort, gtkterm_udp_port, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PORT,
                                                gtkterm_udp_port_port_interface_init))

/**
 * @brief Handle incoming data
 */
static gboolean on_socket_readable (GSocket *socket, GIOCondition condition, gpointer user_data) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (user_data);
    (void)condition;

    GSocketAddress *sender_addr = NULL;
    GError *error = NULL;

    gssize bytes_read = g_socket_receive_from (socket,
                                                &sender_addr,
                                                (gchar *)self->read_buffer,
                                                sizeof (self->read_buffer),
                                                NULL,
                                                &error);

    if (bytes_read > 0) {
        self->stats.bytes_received += bytes_read;
        self->stats.frames_received++;

        // Update remote address if not set
        if (!self->remote_address && sender_addr) {
            self->remote_address = g_object_ref (sender_addr);
        }

        g_signal_emit_by_name (self, "data-received", self->read_buffer, (gsize)bytes_read);
    } else if (error) {
        self->stats.errors++;
        g_signal_emit_by_name (self, "error", error->message);
        g_error_free (error);
    }

    if (sender_addr)
        g_object_unref (sender_addr);

    return G_SOURCE_CONTINUE;
}

/**
 * @brief Get port name
 */
static const char *udp_port_get_name (GtkTermPort *port) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (port);
    return self->name;
}

/**
 * @brief Get port type
 */
static GtkTermPortType udp_port_get_port_type (GtkTermPort *port) {
    (void)port;
    return GTKTERM_PORT_TYPE_UDP;
}

/**
 * @brief Get port state
 */
static GtkTermPortState udp_port_get_state (GtkTermPort *port) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (port);
    return self->state;
}

/**
 * @brief Open port
 */
static gboolean udp_port_open (GtkTermPort *port, GError **error) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (port);

    if (self->state == GTKTERM_PORT_STATE_OPEN) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_BUSY, "Port already open");
        return FALSE;
    }

    self->state = GTKTERM_PORT_STATE_OPENING;

    // Create UDP socket
    self->socket = g_socket_new (G_SOCKET_FAMILY_IPV4,
                                  G_SOCKET_TYPE_DATAGRAM,
                                  G_SOCKET_PROTOCOL_UDP,
                                  error);

    if (!self->socket) {
        self->state = GTKTERM_PORT_STATE_ERROR;
        return FALSE;
    }

    // Bind to local address
    GInetAddress *local_addr = NULL;
    if (self->local_host && strlen (self->local_host) > 0) {
        local_addr = g_inet_address_new_from_string (self->local_host);
    } else {
        local_addr = g_inet_address_new_any (G_SOCKET_FAMILY_IPV4);
    }

    GSocketAddress *bind_addr = g_inet_socket_address_new (local_addr, self->local_port);
    g_object_unref (local_addr);

    gboolean success = g_socket_bind (self->socket, bind_addr, TRUE, error);
    g_object_unref (bind_addr);

    if (!success) {
        self->state = GTKTERM_PORT_STATE_ERROR;
        g_object_unref (self->socket);
        self->socket = NULL;
        return FALSE;
    }

    // Create remote address if specified
    if (self->remote_host && strlen (self->remote_host) > 0) {
        GInetAddress *remote_inet_addr = g_inet_address_new_from_string (self->remote_host);
        if (remote_inet_addr) {
            self->remote_address = g_inet_socket_address_new (remote_inet_addr, self->remote_port);
            g_object_unref (remote_inet_addr);
        }
    }

    // Setup read monitoring
    self->read_source = g_socket_create_source (self->socket, G_IO_IN, NULL);
    g_source_set_callback (self->read_source,
                          (GSourceFunc)on_socket_readable,
                          self,
                          NULL);
    g_source_attach (self->read_source, NULL);

    self->state = GTKTERM_PORT_STATE_OPEN;
    self->stats.connected_time = g_date_time_new_now_local ();

    g_signal_emit_by_name (self, "state-changed",
                          GTKTERM_PORT_STATE_CLOSED,
                          GTKTERM_PORT_STATE_OPEN);

    return TRUE;
}

/**
 * @brief Close port
 */
static void udp_port_close (GtkTermPort *port) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (port);

    if (self->state == GTKTERM_PORT_STATE_CLOSED)
        return;

    GtkTermPortState old_state = self->state;
    self->state = GTKTERM_PORT_STATE_CLOSING;

    if (self->read_source) {
        g_source_destroy (self->read_source);
        g_source_unref (self->read_source);
        self->read_source = NULL;
    }

    if (self->socket) {
        g_socket_close (self->socket, NULL);
        g_object_unref (self->socket);
        self->socket = NULL;
    }

    if (self->remote_address) {
        g_object_unref (self->remote_address);
        self->remote_address = NULL;
    }

    self->state = GTKTERM_PORT_STATE_CLOSED;

    g_signal_emit_by_name (self, "state-changed",
                          old_state,
                          GTKTERM_PORT_STATE_CLOSED);
}

/**
 * @brief Write data to port
 */
static gssize udp_port_write (GtkTermPort *port,
                               const guint8 *data,
                               gsize length,
                               GError **error) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (port);

    if (self->state != GTKTERM_PORT_STATE_OPEN) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED, "Port not open");
        return -1;
    }

    if (!self->remote_address) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED, "No remote address set");
        return -1;
    }

    gssize bytes_written = g_socket_send_to (self->socket,
                                              self->remote_address,
                                              (const gchar *)data,
                                              length,
                                              NULL,
                                              error);

    if (bytes_written > 0) {
        self->stats.bytes_sent += bytes_written;
        self->stats.frames_sent++;
    }

    return bytes_written;
}

/**
 * @brief Read data from port (not used - async read via signals)
 */
static gssize udp_port_read (GtkTermPort *port,
                              guint8 *buffer,
                              gsize length,
                              GError **error) {
    (void)port;
    (void)buffer;
    (void)length;
    (void)error;

    // Data is delivered via data-received signal
    return 0;
}

/**
 * @brief Get port statistics
 */
static void udp_port_get_stats (GtkTermPort *port, GtkTermPortStats *stats) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (port);
    *stats = self->stats;
}

/**
 * @brief Reset port statistics
 */
static void udp_port_reset_stats (GtkTermPort *port) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (port);

    gtkterm_port_stats_clear (&self->stats);
    gtkterm_port_stats_init (&self->stats);
}

/**
 * @brief Interface initialization
 */
static void gtkterm_udp_port_port_interface_init (GtkTermPortInterface *iface) {
    iface->get_name = udp_port_get_name;
    iface->get_port_type = udp_port_get_port_type;
    iface->get_state = udp_port_get_state;
    iface->open = udp_port_open;
    iface->close = udp_port_close;
    iface->write = udp_port_write;
    iface->read = udp_port_read;
    iface->get_stats = udp_port_get_stats;
    iface->reset_stats = udp_port_reset_stats;
}

/**
 * @brief Dispose
 */
static void gtkterm_udp_port_dispose (GObject *object) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (object);

    udp_port_close (GTKTERM_PORT (self));

    G_OBJECT_CLASS (gtkterm_udp_port_parent_class)->dispose (object);
}

/**
 * @brief Finalize
 */
static void gtkterm_udp_port_finalize (GObject *object) {
    GtkTermUDPPort *self = GTKTERM_UDP_PORT (object);

    g_free (self->local_host);
    g_free (self->remote_host);
    g_free (self->name);
    gtkterm_port_stats_clear (&self->stats);

    G_OBJECT_CLASS (gtkterm_udp_port_parent_class)->finalize (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_udp_port_class_init (GtkTermUDPPortClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_udp_port_dispose;
    object_class->finalize = gtkterm_udp_port_finalize;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_udp_port_init (GtkTermUDPPort *self) {
    self->state = GTKTERM_PORT_STATE_CLOSED;
    gtkterm_port_stats_init (&self->stats);
    self->read_source = NULL;
}

/**
 * @brief Create a new UDP port
 */
GtkTermUDPPort *gtkterm_udp_port_new (const char *local_host,
                                       guint16 local_port,
                                       const char *remote_host,
                                       guint16 remote_port) {
    GtkTermUDPPort *udp_port = g_object_new (GTKTERM_TYPE_UDP_PORT, NULL);

    udp_port->local_host = g_strdup (local_host);
    udp_port->local_port = local_port;
    udp_port->remote_host = g_strdup (remote_host);
    udp_port->remote_port = remote_port;

    udp_port->name = g_strdup_printf ("UDP:%s:%u->%s:%u",
                                       local_host ? local_host : "0.0.0.0",
                                       local_port,
                                       remote_host ? remote_host : "*",
                                       remote_port);

    return udp_port;
}

/**
 * @brief Set remote endpoint
 */
void gtkterm_udp_port_set_remote (GtkTermUDPPort *self,
                                   const char *host,
                                   guint16 port) {
    g_return_if_fail (GTKTERM_IS_UDP_PORT (self));

    g_free (self->remote_host);
    self->remote_host = g_strdup (host);
    self->remote_port = port;

    if (self->remote_address) {
        g_object_unref (self->remote_address);
        self->remote_address = NULL;
    }

    if (host && strlen (host) > 0) {
        GInetAddress *remote_inet_addr = g_inet_address_new_from_string (host);
        if (remote_inet_addr) {
            self->remote_address = g_inet_socket_address_new (remote_inet_addr, port);
            g_object_unref (remote_inet_addr);
        }
    }
}
