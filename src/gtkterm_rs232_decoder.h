/************************************************************************/
/* gtkterm_rs232_decoder.h                                              */
/* -----------------------                                              */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      RS232 control signals decoder                                  */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_RS232_DECODER_H
#define GTKTERM_RS232_DECODER_H

#include <glib-object.h>
#include "gtkterm_protocol_decoder.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_RS232_DECODER gtkterm_rs232_decoder_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermRS232Decoder, gtkterm_rs232_decoder, GTKTERM, RS232_DECODER, GObject)

/**
 * @brief RS232 signal flags
 */
typedef enum {
    GTKTERM_RS232_SIGNAL_DTR = (1 << 0),  /**< Data Terminal Ready */
    GTKTERM_RS232_SIGNAL_DSR = (1 << 1),  /**< Data Set Ready */
    GTKTERM_RS232_SIGNAL_RTS = (1 << 2),  /**< Request To Send */
    GTKTERM_RS232_SIGNAL_CTS = (1 << 3),  /**< Clear To Send */
    GTKTERM_RS232_SIGNAL_DCD = (1 << 4),  /**< Data Carrier Detect */
    GTKTERM_RS232_SIGNAL_RI  = (1 << 5)   /**< Ring Indicator */
} GtkTermRS232Signal;

/**
 * @brief Create a new RS232 decoder
 */
GtkTermRS232Decoder *gtkterm_rs232_decoder_new (void);

G_END_DECLS

#endif // GTKTERM_RS232_DECODER_H
