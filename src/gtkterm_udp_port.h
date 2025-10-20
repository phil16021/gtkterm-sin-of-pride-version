/************************************************************************/
/* gtkterm_udp_port.h                                                   */
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

#ifndef GTKTERM_UDP_PORT_H
#define GTKTERM_UDP_PORT_H

#include <glib-object.h>
#include "gtkterm_port.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_UDP_PORT gtkterm_udp_port_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermUDPPort, gtkterm_udp_port, GTKTERM, UDP_PORT, GObject)

/**
 * @brief Create a new UDP port
 */
GtkTermUDPPort *gtkterm_udp_port_new (const char *local_host,
                                       guint16 local_port,
                                       const char *remote_host,
                                       guint16 remote_port);

/**
 * @brief Set remote endpoint
 */
void gtkterm_udp_port_set_remote (GtkTermUDPPort *self,
                                   const char *host,
                                   guint16 port);

G_END_DECLS

#endif // GTKTERM_UDP_PORT_H
