/************************************************************************/
/* gtkterm_tcp_port.c                                                   */
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

#include "gtkterm_tcp_port.h"
#include <string.h>

struct _GtkTermTCPPort {
    GObject parent_instance;

    /* Configuration */
    GtkTermTCPMode mode;
    char *host;
    guint16 port;
    char *name;

    /* State */
    GtkTermPortState state;
    GtkTermPortStats stats;

    /* Client mode */
    GSocketClient *client;
    GSocketConnection *connection;
    GInputStream *input_stream;
    GOutputStream *output_stream;

    /* Server mode */
    GSocketService *service;
    GList *client_connections;  // List of GSocketConnection*

    /* I/O */
    GSource *read_source;
    guint8 read_buffer[4096];
};

static void gtkterm_tcp_port_port_interface_init (GtkTermPortInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermTCPPort, gtkterm_tcp_port, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PORT,
                                                gtkterm_tcp_port_port_interface_init))

/**
 * @brief Handle incoming data
 */
static gboolean on_socket_readable (GSocket *socket, GIOCondition condition, gpointer user_data) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (user_data);
    (void)condition;

    GError *error = NULL;
    gssize bytes_read = g_socket_receive (socket,
                                          (gchar *)self->read_buffer,
                                          sizeof (self->read_buffer),
                                          NULL,
                                          &error);

    if (bytes_read > 0) {
        self->stats.bytes_received += bytes_read;
        self->stats.frames_received++;

        g_signal_emit_by_name (self, "data-received", self->read_buffer, (gsize)bytes_read);
        return G_SOURCE_CONTINUE;
    }

    if (bytes_read == 0) {
        // Connection closed
        g_message ("TCP connection closed by remote");
        self->state = GTKTERM_PORT_STATE_CLOSED;
        g_signal_emit_by_name (self, "state-changed",
                              GTKTERM_PORT_STATE_OPEN,
                              GTKTERM_PORT_STATE_CLOSED);
        return G_SOURCE_REMOVE;
    }

    // Error occurred
    if (error) {
        self->stats.errors++;
        g_signal_emit_by_name (self, "error", error->message);
        g_error_free (error);
    }

    return G_SOURCE_REMOVE;
}

/**
 * @brief Setup read monitoring for a socket
 */
static void setup_socket_read (GtkTermTCPPort *self, GSocket *socket) {
    if (self->read_source) {
        g_source_destroy (self->read_source);
        g_source_unref (self->read_source);
    }

    self->read_source = g_socket_create_source (socket, G_IO_IN, NULL);
    g_source_set_callback (self->read_source,
                          (GSourceFunc)on_socket_readable,
                          self,
                          NULL);
    g_source_attach (self->read_source, NULL);
}

/**
 * @brief Handle incoming client connection (server mode)
 */
static gboolean on_incoming_connection (GSocketService *service,
                                         GSocketConnection *connection,
                                         GObject *source_object,
                                         gpointer user_data) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (user_data);
    (void)service;
    (void)source_object;

    g_message ("TCP client connected");

    // Add to client list
    g_object_ref (connection);
    self->client_connections = g_list_append (self->client_connections, connection);

    // Setup read monitoring
    GSocket *socket = g_socket_connection_get_socket (connection);
    setup_socket_read (self, socket);

    return TRUE;
}

/**
 * @brief Get port name
 */
static const char *tcp_port_get_name (GtkTermPort *port) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);
    return self->name;
}

/**
 * @brief Get port type
 */
static GtkTermPortType tcp_port_get_port_type (GtkTermPort *port) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);

    return (self->mode == GTKTERM_TCP_MODE_CLIENT) ?
           GTKTERM_PORT_TYPE_TCP_CLIENT :
           GTKTERM_PORT_TYPE_TCP_SERVER;
}

/**
 * @brief Get port state
 */
static GtkTermPortState tcp_port_get_state (GtkTermPort *port) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);
    return self->state;
}

/**
 * @brief Open port
 */
static gboolean tcp_port_open (GtkTermPort *port, GError **error) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);

    if (self->state == GTKTERM_PORT_STATE_OPEN) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_BUSY, "Port already open");
        return FALSE;
    }

    self->state = GTKTERM_PORT_STATE_OPENING;

    if (self->mode == GTKTERM_TCP_MODE_CLIENT) {
        // Client mode: connect to server
        self->client = g_socket_client_new ();

        self->connection = g_socket_client_connect_to_host (self->client,
                                                             self->host,
                                                             self->port,
                                                             NULL,
                                                             error);

        if (!self->connection) {
            self->state = GTKTERM_PORT_STATE_ERROR;
            g_object_unref (self->client);
            self->client = NULL;
            return FALSE;
        }

        self->input_stream = g_io_stream_get_input_stream (G_IO_STREAM (self->connection));
        self->output_stream = g_io_stream_get_output_stream (G_IO_STREAM (self->connection));

        // Setup read monitoring
        GSocket *socket = g_socket_connection_get_socket (self->connection);
        setup_socket_read (self, socket);

    } else {
        // Server mode: listen for connections
        self->service = g_socket_service_new ();

        GInetAddress *addr = g_inet_address_new_any (G_SOCKET_FAMILY_IPV4);
        GSocketAddress *sock_addr = g_inet_socket_address_new (addr, self->port);

        gboolean success = g_socket_listener_add_address (
            G_SOCKET_LISTENER (self->service),
            sock_addr,
            G_SOCKET_TYPE_STREAM,
            G_SOCKET_PROTOCOL_TCP,
            NULL,
            NULL,
            error
        );

        g_object_unref (addr);
        g_object_unref (sock_addr);

        if (!success) {
            self->state = GTKTERM_PORT_STATE_ERROR;
            g_object_unref (self->service);
            self->service = NULL;
            return FALSE;
        }

        g_signal_connect (self->service, "incoming",
                         G_CALLBACK (on_incoming_connection), self);

        g_socket_service_start (self->service);
    }

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
static void tcp_port_close (GtkTermPort *port) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);

    if (self->state == GTKTERM_PORT_STATE_CLOSED)
        return;

    GtkTermPortState old_state = self->state;
    self->state = GTKTERM_PORT_STATE_CLOSING;

    // Cleanup read source
    if (self->read_source) {
        g_source_destroy (self->read_source);
        g_source_unref (self->read_source);
        self->read_source = NULL;
    }

    // Client mode cleanup
    if (self->connection) {
        g_io_stream_close (G_IO_STREAM (self->connection), NULL, NULL);
        g_object_unref (self->connection);
        self->connection = NULL;
        self->input_stream = NULL;
        self->output_stream = NULL;
    }

    if (self->client) {
        g_object_unref (self->client);
        self->client = NULL;
    }

    // Server mode cleanup
    if (self->service) {
        g_socket_service_stop (self->service);
        g_object_unref (self->service);
        self->service = NULL;
    }

    // Close all client connections
    for (GList *l = self->client_connections; l != NULL; l = l->next) {
        GSocketConnection *conn = G_SOCKET_CONNECTION (l->data);
        g_io_stream_close (G_IO_STREAM (conn), NULL, NULL);
        g_object_unref (conn);
    }
    g_list_free (self->client_connections);
    self->client_connections = NULL;

    self->state = GTKTERM_PORT_STATE_CLOSED;

    g_signal_emit_by_name (self, "state-changed",
                          old_state,
                          GTKTERM_PORT_STATE_CLOSED);
}

/**
 * @brief Write data to port
 */
static gssize tcp_port_write (GtkTermPort *port,
                               const guint8 *data,
                               gsize length,
                               GError **error) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);

    if (self->state != GTKTERM_PORT_STATE_OPEN) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED, "Port not open");
        return -1;
    }

    gssize bytes_written = 0;

    if (self->mode == GTKTERM_TCP_MODE_CLIENT) {
        // Client mode: write to single connection
        if (self->output_stream) {
            bytes_written = g_output_stream_write (self->output_stream,
                                                    data, length,
                                                    NULL, error);
        }
    } else {
        // Server mode: broadcast to all clients
        for (GList *l = self->client_connections; l != NULL; l = l->next) {
            GSocketConnection *conn = G_SOCKET_CONNECTION (l->data);
            GOutputStream *stream = g_io_stream_get_output_stream (G_IO_STREAM (conn));

            gssize written = g_output_stream_write (stream, data, length, NULL, error);
            if (written > bytes_written)
                bytes_written = written;
        }
    }

    if (bytes_written > 0) {
        self->stats.bytes_sent += bytes_written;
        self->stats.frames_sent++;
    }

    return bytes_written;
}

/**
 * @brief Read data from port (not used - async read via signals)
 */
static gssize tcp_port_read (GtkTermPort *port,
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
static void tcp_port_get_stats (GtkTermPort *port, GtkTermPortStats *stats) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);
    *stats = self->stats;
}

/**
 * @brief Reset port statistics
 */
static void tcp_port_reset_stats (GtkTermPort *port) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (port);

    gtkterm_port_stats_clear (&self->stats);
    gtkterm_port_stats_init (&self->stats);
}

/**
 * @brief Interface initialization
 */
static void gtkterm_tcp_port_port_interface_init (GtkTermPortInterface *iface) {
    iface->get_name = tcp_port_get_name;
    iface->get_port_type = tcp_port_get_port_type;
    iface->get_state = tcp_port_get_state;
    iface->open = tcp_port_open;
    iface->close = tcp_port_close;
    iface->write = tcp_port_write;
    iface->read = tcp_port_read;
    iface->get_stats = tcp_port_get_stats;
    iface->reset_stats = tcp_port_reset_stats;
}

/**
 * @brief Dispose
 */
static void gtkterm_tcp_port_dispose (GObject *object) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (object);

    tcp_port_close (GTKTERM_PORT (self));

    G_OBJECT_CLASS (gtkterm_tcp_port_parent_class)->dispose (object);
}

/**
 * @brief Finalize
 */
static void gtkterm_tcp_port_finalize (GObject *object) {
    GtkTermTCPPort *self = GTKTERM_TCP_PORT (object);

    g_free (self->host);
    g_free (self->name);
    gtkterm_port_stats_clear (&self->stats);

    G_OBJECT_CLASS (gtkterm_tcp_port_parent_class)->finalize (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_tcp_port_class_init (GtkTermTCPPortClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_tcp_port_dispose;
    object_class->finalize = gtkterm_tcp_port_finalize;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_tcp_port_init (GtkTermTCPPort *self) {
    self->state = GTKTERM_PORT_STATE_CLOSED;
    gtkterm_port_stats_init (&self->stats);
    self->client_connections = NULL;
    self->read_source = NULL;
}

/**
 * @brief Create a new TCP port
 */
GtkTermTCPPort *gtkterm_tcp_port_new (GtkTermTCPMode mode,
                                       const char *host,
                                       guint16 port) {
    GtkTermTCPPort *tcp_port = g_object_new (GTKTERM_TYPE_TCP_PORT, NULL);

    tcp_port->mode = mode;
    tcp_port->host = g_strdup (host);
    tcp_port->port = port;

    if (mode == GTKTERM_TCP_MODE_CLIENT) {
        tcp_port->name = g_strdup_printf ("TCP:%s:%u", host, port);
    } else {
        tcp_port->name = g_strdup_printf ("TCP-Server:%u", port);
    }

    return tcp_port;
}

/**
 * @brief Get remote address
 */
const char *gtkterm_tcp_port_get_remote_address (GtkTermTCPPort *self) {
    g_return_val_if_fail (GTKTERM_IS_TCP_PORT (self), NULL);

    if (self->mode == GTKTERM_TCP_MODE_CLIENT && self->connection) {
        GSocketAddress *addr = g_socket_connection_get_remote_address (self->connection, NULL);
        if (addr && G_IS_INET_SOCKET_ADDRESS (addr)) {
            GInetAddress *inet_addr = g_inet_socket_address_get_address (G_INET_SOCKET_ADDRESS (addr));
            return g_inet_address_to_string (inet_addr);
        }
    }

    return NULL;
}

/**
 * @brief Get number of connected clients
 */
guint gtkterm_tcp_port_get_client_count (GtkTermTCPPort *self) {
    g_return_val_if_fail (GTKTERM_IS_TCP_PORT (self), 0);

    if (self->mode == GTKTERM_TCP_MODE_SERVER) {
        return g_list_length (self->client_connections);
    }

    return 0;
}
