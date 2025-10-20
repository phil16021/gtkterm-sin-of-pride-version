/************************************************************************/
/* gtkterm_can_decoder.h                                                */
/* ---------------------                                                */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      CAN bus protocol decoder                                       */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_CAN_DECODER_H
#define GTKTERM_CAN_DECODER_H

#include <glib-object.h>
#include "gtkterm_protocol_decoder.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_CAN_DECODER gtkterm_can_decoder_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermCANDecoder, gtkterm_can_decoder, GTKTERM, CAN_DECODER, GObject)

/**
 * @brief CAN frame format
 */
typedef enum {
    GTKTERM_CAN_FORMAT_AUTO,      /**< Auto-detect format */
    GTKTERM_CAN_FORMAT_CANDUMP,   /**< candump text format */
    GTKTERM_CAN_FORMAT_BINARY     /**< Raw binary format */
} GtkTermCANFormat;

/**
 * @brief Create a new CAN decoder
 */
GtkTermCANDecoder *gtkterm_can_decoder_new (GtkTermCANFormat format);

G_END_DECLS

#endif // GTKTERM_CAN_DECODER_H
