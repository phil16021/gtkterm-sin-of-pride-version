/************************************************************************/
/* gtkterm_nmea_decoder.h                                               */
/* ----------------------                                               */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      NMEA 0183 protocol decoder (GPS/GNSS sentences)                */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_NMEA_DECODER_H
#define GTKTERM_NMEA_DECODER_H

#include <glib-object.h>
#include "gtkterm_protocol_decoder.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_NMEA_DECODER gtkterm_nmea_decoder_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermNMEADecoder, gtkterm_nmea_decoder, GTKTERM, NMEA_DECODER, GObject)

/**
 * @brief Create a new NMEA decoder
 */
GtkTermNMEADecoder *gtkterm_nmea_decoder_new (void);

G_END_DECLS

#endif // GTKTERM_NMEA_DECODER_H
