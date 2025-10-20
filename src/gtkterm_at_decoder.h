/************************************************************************/
/* gtkterm_at_decoder.h                                                 */
/* --------------------                                                 */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      AT Command protocol decoder                                    */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_AT_DECODER_H
#define GTKTERM_AT_DECODER_H

#include <glib-object.h>
#include "gtkterm_protocol_decoder.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_AT_DECODER gtkterm_at_decoder_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermATDecoder, gtkterm_at_decoder, GTKTERM, AT_DECODER, GObject)

/**
 * @brief Create a new AT command decoder
 */
GtkTermATDecoder *gtkterm_at_decoder_new (void);

G_END_DECLS

#endif // GTKTERM_AT_DECODER_H
