/************************************************************************/
/* gtkterm_modbus_tcp_decoder.h                                         */
/* ----------------------------                                         */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Modbus TCP protocol decoder                                    */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_MODBUS_TCP_DECODER_H
#define GTKTERM_MODBUS_TCP_DECODER_H

#include <glib-object.h>
#include "gtkterm_protocol_decoder.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_MODBUS_TCP_DECODER gtkterm_modbus_tcp_decoder_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermModbusTCPDecoder, gtkterm_modbus_tcp_decoder, GTKTERM, MODBUS_TCP_DECODER, GObject)

/**
 * @brief Create a new Modbus TCP decoder
 */
GtkTermModbusTCPDecoder *gtkterm_modbus_tcp_decoder_new (void);

G_END_DECLS

#endif // GTKTERM_MODBUS_TCP_DECODER_H
