/************************************************************************/
/* gtkterm_protocol_decoder.c                                           */
/* --------------------------                                           */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Base implementation for protocol decoder interface             */
/*                                                                      */
/************************************************************************/

#include "gtkterm_protocol_decoder.h"

G_DEFINE_INTERFACE (GtkTermProtocolDecoder, gtkterm_protocol_decoder, G_TYPE_OBJECT)

/**
 * @brief Interface initialization
 */
static void gtkterm_protocol_decoder_default_init (GtkTermProtocolDecoderInterface *iface) {
    // Default implementations can be added here
    (void)iface;
}

/**
 * @brief Get protocol name
 */
const char *gtkterm_protocol_decoder_get_name (GtkTermProtocolDecoder *self) {
    GtkTermProtocolDecoderInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PROTOCOL_DECODER (self), NULL);

    iface = GTKTERM_PROTOCOL_DECODER_GET_IFACE (self);
    g_return_val_if_fail (iface->get_name != NULL, NULL);

    return iface->get_name (self);
}

/**
 * @brief Get protocol ID
 */
const char *gtkterm_protocol_decoder_get_id (GtkTermProtocolDecoder *self) {
    GtkTermProtocolDecoderInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PROTOCOL_DECODER (self), NULL);

    iface = GTKTERM_PROTOCOL_DECODER_GET_IFACE (self);
    g_return_val_if_fail (iface->get_id != NULL, NULL);

    return iface->get_id (self);
}

/**
 * @brief Detect protocol
 */
gdouble gtkterm_protocol_decoder_detect (GtkTermProtocolDecoder *self,
                                          const guint8 *data,
                                          gsize length) {
    GtkTermProtocolDecoderInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PROTOCOL_DECODER (self), 0.0);
    g_return_val_if_fail (data != NULL, 0.0);

    iface = GTKTERM_PROTOCOL_DECODER_GET_IFACE (self);
    g_return_val_if_fail (iface->detect != NULL, 0.0);

    return iface->detect (self, data, length);
}

/**
 * @brief Decode data chunk
 */
char *gtkterm_protocol_decoder_decode (GtkTermProtocolDecoder *self,
                                        const GtkTermDataChunk *chunk) {
    GtkTermProtocolDecoderInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PROTOCOL_DECODER (self), NULL);

    // BUG FIX #3: Use graceful error handling instead of fatal assertion
    // g_return_val_if_fail can be fatal in test mode, but we want consistent graceful handling
    if (chunk == NULL) {
        return NULL;
    }

    iface = GTKTERM_PROTOCOL_DECODER_GET_IFACE (self);
    g_return_val_if_fail (iface->decode != NULL, NULL);

    return iface->decode (self, chunk);
}

/**
 * @brief Validate data
 */
gboolean gtkterm_protocol_decoder_validate (GtkTermProtocolDecoder *self,
                                             const guint8 *data,
                                             gsize length,
                                             char **error) {
    GtkTermProtocolDecoderInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PROTOCOL_DECODER (self), FALSE);
    g_return_val_if_fail (data != NULL, FALSE);

    iface = GTKTERM_PROTOCOL_DECODER_GET_IFACE (self);
    g_return_val_if_fail (iface->validate != NULL, FALSE);

    return iface->validate (self, data, length, error);
}

/**
 * @brief Get statistics
 */
char *gtkterm_protocol_decoder_get_stats (GtkTermProtocolDecoder *self) {
    GtkTermProtocolDecoderInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_PROTOCOL_DECODER (self), NULL);

    iface = GTKTERM_PROTOCOL_DECODER_GET_IFACE (self);

    if (iface->get_stats == NULL)
        return NULL;

    return iface->get_stats (self);
}

/**
 * @brief Reset decoder
 */
void gtkterm_protocol_decoder_reset (GtkTermProtocolDecoder *self) {
    GtkTermProtocolDecoderInterface *iface;

    g_return_if_fail (GTKTERM_IS_PROTOCOL_DECODER (self));

    iface = GTKTERM_PROTOCOL_DECODER_GET_IFACE (self);

    if (iface->reset != NULL)
        iface->reset (self);
}

/**
 * @brief Create a new decoded packet
 */
GtkTermDecodedPacket *gtkterm_decoded_packet_new (void) {
    GtkTermDecodedPacket *packet = g_new0 (GtkTermDecodedPacket, 1);
    packet->fields = g_hash_table_new_full (g_str_hash, g_str_equal, g_free, g_free);
    packet->valid = TRUE;
    return packet;
}

/**
 * @brief Free a decoded packet
 */
void gtkterm_decoded_packet_free (GtkTermDecodedPacket *packet) {
    if (!packet)
        return;

    g_free (packet->protocol);
    g_free (packet->summary);
    g_free (packet->details);
    g_free (packet->error);

    if (packet->fields)
        g_hash_table_destroy (packet->fields);

    g_free (packet);
}

/**
 * @brief Add a field to decoded packet
 */
void gtkterm_decoded_packet_add_field (GtkTermDecodedPacket *packet,
                                        const char *name,
                                        const char *value) {
    g_return_if_fail (packet != NULL);
    g_return_if_fail (name != NULL);
    g_return_if_fail (value != NULL);

    g_hash_table_insert (packet->fields, g_strdup (name), g_strdup (value));
}
