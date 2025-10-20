/************************************************************************/
/* gtkterm_modbus_decoder.h                                             */
/* ------------------------                                             */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Modbus ASCII/RTU protocol decoder                              */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_MODBUS_DECODER_H
#define GTKTERM_MODBUS_DECODER_H

#include <glib-object.h>
#include "gtkterm_protocol_decoder.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_MODBUS_DECODER gtkterm_modbus_decoder_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermModbusDecoder, gtkterm_modbus_decoder, GTKTERM, MODBUS_DECODER, GObject)

/**
 * @brief Modbus protocol variant
 */
typedef enum {
    GTKTERM_MODBUS_ASCII,    /**< Modbus ASCII (text-based with LRC) */
    GTKTERM_MODBUS_RTU       /**< Modbus RTU (binary with CRC) */
} GtkTermModbusMode;

/**
 * @brief Create a new Modbus decoder
 */
GtkTermModbusDecoder *gtkterm_modbus_decoder_new (GtkTermModbusMode mode);

G_END_DECLS

#endif // GTKTERM_MODBUS_DECODER_H
