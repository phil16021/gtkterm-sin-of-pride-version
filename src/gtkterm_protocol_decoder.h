/************************************************************************/
/* gtkterm_protocol_decoder.h                                           */
/* --------------------------                                           */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Plugin architecture for protocol analysis and decoding         */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_PROTOCOL_DECODER_H
#define GTKTERM_PROTOCOL_DECODER_H

#include <glib-object.h>
#include "gtkterm_data_chunk.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_PROTOCOL_DECODER gtkterm_protocol_decoder_get_type ()
G_DECLARE_INTERFACE (GtkTermProtocolDecoder, gtkterm_protocol_decoder, GTKTERM, PROTOCOL_DECODER, GObject)

/**
 * @brief Protocol decoder interface
 *
 * Base interface for all protocol decoders. Each decoder implements
 * detection, validation, and decoding logic for a specific protocol.
 */
struct _GtkTermProtocolDecoderInterface {
    GTypeInterface parent_iface;

    /**
     * @brief Get the protocol name
     * @return Protocol name (e.g., "AT Commands", "NMEA 0183")
     */
    const char * (*get_name) (GtkTermProtocolDecoder *self);

    /**
     * @brief Get a short protocol identifier
     * @return Short ID (e.g., "at", "nmea", "modbus")
     */
    const char * (*get_id) (GtkTermProtocolDecoder *self);

    /**
     * @brief Detect if data matches this protocol
     * @param data Raw data to analyze
     * @param length Data length
     * @return Confidence score 0.0-1.0 (0=no match, 1=certain match)
     */
    gdouble (*detect) (GtkTermProtocolDecoder *self, const guint8 *data, gsize length);

    /**
     * @brief Decode a data chunk
     * @param chunk Data chunk to decode
     * @return Decoded string (caller must free), or NULL on error
     */
    char * (*decode) (GtkTermProtocolDecoder *self, const GtkTermDataChunk *chunk);

    /**
     * @brief Validate data integrity
     * @param data Raw data to validate
     * @param length Data length
     * @param error Error message return location
     * @return TRUE if valid, FALSE if errors detected
     */
    gboolean (*validate) (GtkTermProtocolDecoder *self,
                          const guint8 *data,
                          gsize length,
                          char **error);

    /**
     * @brief Get decoder statistics
     * @return Statistics string (caller must free), or NULL
     */
    char * (*get_stats) (GtkTermProtocolDecoder *self);

    /**
     * @brief Reset decoder state
     */
    void (*reset) (GtkTermProtocolDecoder *self);
};

/* Interface methods */
const char *gtkterm_protocol_decoder_get_name (GtkTermProtocolDecoder *self);
const char *gtkterm_protocol_decoder_get_id (GtkTermProtocolDecoder *self);
gdouble gtkterm_protocol_decoder_detect (GtkTermProtocolDecoder *self,
                                          const guint8 *data,
                                          gsize length);
char *gtkterm_protocol_decoder_decode (GtkTermProtocolDecoder *self,
                                        const GtkTermDataChunk *chunk);
gboolean gtkterm_protocol_decoder_validate (GtkTermProtocolDecoder *self,
                                             const guint8 *data,
                                             gsize length,
                                             char **error);
char *gtkterm_protocol_decoder_get_stats (GtkTermProtocolDecoder *self);
void gtkterm_protocol_decoder_reset (GtkTermProtocolDecoder *self);

/**
 * @brief Decoded packet information
 */
typedef struct {
    char *protocol;          /**< Protocol name */
    char *summary;           /**< One-line summary */
    char *details;           /**< Detailed breakdown */
    gboolean valid;          /**< Data is valid */
    char *error;             /**< Error message if invalid */
    GHashTable *fields;      /**< Decoded fields (name -> value) */
} GtkTermDecodedPacket;

GtkTermDecodedPacket *gtkterm_decoded_packet_new (void);
void gtkterm_decoded_packet_free (GtkTermDecodedPacket *packet);
void gtkterm_decoded_packet_add_field (GtkTermDecodedPacket *packet,
                                        const char *name,
                                        const char *value);

G_END_DECLS

#endif // GTKTERM_PROTOCOL_DECODER_H
