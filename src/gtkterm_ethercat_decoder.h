/************************************************************************/
/* gtkterm_ethercat_decoder.h                                           */
/* --------------------------                                           */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      EtherCAT protocol decoder                                      */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_ETHERCAT_DECODER_H
#define GTKTERM_ETHERCAT_DECODER_H

#include <glib-object.h>
#include "gtkterm_protocol_decoder.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_ETHERCAT_DECODER gtkterm_ethercat_decoder_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermEtherCATDecoder, gtkterm_ethercat_decoder, GTKTERM, ETHERCAT_DECODER, GObject)

/**
 * @brief Create a new EtherCAT decoder
 */
GtkTermEtherCATDecoder *gtkterm_ethercat_decoder_new (void);

G_END_DECLS

#endif // GTKTERM_ETHERCAT_DECODER_H
