/************************************************************************/
/* gtkterm_ethercat_decoder.c                                           */
/* --------------------------                                           */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      EtherCAT protocol decoder implementation                       */
/*                                                                      */
/************************************************************************/

#include "gtkterm_ethercat_decoder.h"
#include <string.h>
#include <stdio.h>

struct _GtkTermEtherCATDecoder {
    GObject parent_instance;
    guint datagrams_decoded;
    guint errors_detected;
    guint working_counter_errors;
};

static void gtkterm_ethercat_decoder_interface_init (GtkTermProtocolDecoderInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermEtherCATDecoder, gtkterm_ethercat_decoder, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PROTOCOL_DECODER,
                                                gtkterm_ethercat_decoder_interface_init))

/**
 * @brief EtherCAT command types
 */
typedef struct {
    guint8 code;
    const char *name;
    const char *description;
} EtherCATCommand;

static const EtherCATCommand ethercat_commands[] = {
    {0x01, "APRD", "Auto Increment Physical Read"},
    {0x02, "APWR", "Auto Increment Physical Write"},
    {0x03, "APRW", "Auto Increment Physical Read Write"},
    {0x04, "FPRD", "Configured Address Physical Read"},
    {0x05, "FPWR", "Configured Address Physical Write"},
    {0x06, "FPRW", "Configured Address Physical Read Write"},
    {0x07, "BRD", "Broadcast Read"},
    {0x08, "BWR", "Broadcast Write"},
    {0x09, "BRW", "Broadcast Read Write"},
    {0x0A, "LRD", "Logical Memory Read"},
    {0x0B, "LWR", "Logical Memory Write"},
    {0x0C, "LRW", "Logical Memory Read Write"},
    {0x0D, "ARMW", "Auto Increment Physical Read Multiple Write"},
    {0x0E, "FRMW", "Configured Address Physical Read Multiple Write"},
    {0x00, NULL, NULL}
};

/**
 * @brief Get command name and description
 */
static const EtherCATCommand *get_command_info (guint8 cmd) {
    for (int i = 0; ethercat_commands[i].name != NULL; i++) {
        if (ethercat_commands[i].code == cmd) {
            return &ethercat_commands[i];
        }
    }
    return NULL;
}

/**
 * @brief Get protocol name
 */
static const char *ethercat_decoder_get_name (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "EtherCAT";
}

/**
 * @brief Get protocol ID
 */
static const char *ethercat_decoder_get_id (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "ethercat";
}

/**
 * @brief Detect EtherCAT protocol
 */
static gdouble ethercat_decoder_detect (GtkTermProtocolDecoder *decoder,
                                         const guint8 *data,
                                         gsize length) {
    (void)decoder;

    // EtherCAT frame minimum: 2 byte header + 10 byte datagram header + 2 byte WKC
    if (length < 14)
        return 0.0;

    // Check EtherCAT header (first 2 bytes)
    // Length field (11 bits) should be reasonable
    guint16 header = data[0] | (data[1] << 8);
    guint16 frame_length = header & 0x07FF;
    guint8 frame_type = (header >> 12) & 0x0F;

    // Frame type should be 1 (EtherCAT)
    if (frame_type != 0x01)
        return 0.0;

    // Length should match or be close
    if (frame_length > 0 && frame_length < 1500) {
        // Check if command byte looks valid
        guint8 cmd = data[2];
        if (get_command_info (cmd) != NULL) {
            return 0.9;
        } else if (cmd >= 0x01 && cmd <= 0x0E) {
            return 0.7;
        }
    }

    return 0.0;
}

/**
 * @brief Decode EtherCAT datagram
 */
static char *ethercat_decoder_decode (GtkTermProtocolDecoder *decoder,
                                       const GtkTermDataChunk *chunk) {
    GtkTermEtherCATDecoder *self = GTKTERM_ETHERCAT_DECODER (decoder);
    GString *decoded = g_string_new (NULL);

    if (!chunk || !chunk->data || chunk->length < 14) {
        g_string_printf (decoded, "[EtherCAT] Frame too short");
        self->errors_detected++;
        return g_string_free (decoded, FALSE);
    }

    const guint8 *data = chunk->data;

    // Parse EtherCAT header (2 bytes)
    guint16 header = data[0] | (data[1] << 8);
    guint16 frame_length = header & 0x07FF;
    guint8 reserved = (header >> 11) & 0x01;
    guint8 frame_type = (header >> 12) & 0x0F;

    (void)reserved; // Suppress unused warning

    g_string_printf (decoded, "[EtherCAT] Type:%u Len:%u",
                     frame_type, frame_length);

    // Parse datagram header (10 bytes minimum)
    guint8 cmd = data[2];
    guint8 idx = data[3];
    guint32 address = data[4] | (data[5] << 8) | (data[6] << 16) | (data[7] << 24);
    guint16 datagram_length = data[8] | ((data[9] & 0x07) << 8);
    guint8 circulating = (data[9] >> 6) & 0x01;
    guint8 more_follows = (data[9] >> 7) & 0x01;
    guint16 irq = data[10] | (data[11] << 8);

    const EtherCATCommand *cmd_info = get_command_info (cmd);

    if (cmd_info) {
        g_string_append_printf (decoded, " Cmd:%s", cmd_info->name);
    } else {
        g_string_append_printf (decoded, " Cmd:0x%02X", cmd);
    }

    g_string_append_printf (decoded, " Idx:%u Addr:0x%08X DLen:%u",
                            idx, address, datagram_length);

    if (circulating) {
        g_string_append (decoded, " [CIRCULATING]");
    }

    if (more_follows) {
        g_string_append (decoded, " [MORE]");
    }

    if (irq != 0) {
        g_string_append_printf (decoded, " IRQ:0x%04X", irq);
    }

    // Data starts at offset 12
    gsize data_offset = 12;

    if (datagram_length > 0 && chunk->length >= data_offset + datagram_length + 2) {
        // Show first few bytes of data
        g_string_append (decoded, " Data:");
        gsize display_bytes = (datagram_length > 8) ? 8 : datagram_length;
        for (gsize i = 0; i < display_bytes; i++) {
            g_string_append_printf (decoded, " %02X", data[data_offset + i]);
        }
        if (datagram_length > 8) {
            g_string_append (decoded, "...");
        }

        // Working counter (2 bytes at the end)
        gsize wkc_offset = data_offset + datagram_length;
        if (chunk->length >= wkc_offset + 2) {
            guint16 wkc = data[wkc_offset] | (data[wkc_offset + 1] << 8);
            g_string_append_printf (decoded, " WKC:%u", wkc);

            // WKC of 0 often indicates an error
            if (wkc == 0) {
                g_string_append (decoded, " [WKC ERROR]");
                self->working_counter_errors++;
            }
        }
    }

    // Add command description
    if (cmd_info) {
        g_string_append_printf (decoded, " (%s)", cmd_info->description);
    }

    self->datagrams_decoded++;
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Validate EtherCAT frame
 */
static gboolean ethercat_decoder_validate (GtkTermProtocolDecoder *decoder,
                                             const guint8 *data,
                                             gsize length,
                                             char **error) {
    (void)decoder;

    if (length < 14) {
        if (error) *error = g_strdup ("Frame too short (min 14 bytes)");
        return FALSE;
    }

    // Validate frame type
    guint16 header = data[0] | (data[1] << 8);
    guint8 frame_type = (header >> 12) & 0x0F;

    if (frame_type != 0x01) {
        if (error) *error = g_strdup_printf ("Invalid frame type: %u (expected 1)", frame_type);
        return FALSE;
    }

    // Validate command
    guint8 cmd = data[2];
    if (cmd == 0 || cmd > 0x0E) {
        if (error) *error = g_strdup_printf ("Invalid command: 0x%02X", cmd);
        return FALSE;
    }

    return TRUE;
}

/**
 * @brief Get decoder statistics
 */
static char *ethercat_decoder_get_stats (GtkTermProtocolDecoder *decoder) {
    GtkTermEtherCATDecoder *self = GTKTERM_ETHERCAT_DECODER (decoder);

    return g_strdup_printf ("Datagrams: %u, Errors: %u, WKC Errors: %u",
                            self->datagrams_decoded,
                            self->errors_detected,
                            self->working_counter_errors);
}

/**
 * @brief Reset decoder statistics
 */
static void ethercat_decoder_reset (GtkTermProtocolDecoder *decoder) {
    GtkTermEtherCATDecoder *self = GTKTERM_ETHERCAT_DECODER (decoder);

    self->datagrams_decoded = 0;
    self->errors_detected = 0;
    self->working_counter_errors = 0;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_ethercat_decoder_interface_init (GtkTermProtocolDecoderInterface *iface) {
    iface->get_name = ethercat_decoder_get_name;
    iface->get_id = ethercat_decoder_get_id;
    iface->detect = ethercat_decoder_detect;
    iface->decode = ethercat_decoder_decode;
    iface->validate = ethercat_decoder_validate;
    iface->get_stats = ethercat_decoder_get_stats;
    iface->reset = ethercat_decoder_reset;
}

/**
 * @brief Class initialization
 */
static void gtkterm_ethercat_decoder_class_init (GtkTermEtherCATDecoderClass *klass) {
    (void)klass;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_ethercat_decoder_init (GtkTermEtherCATDecoder *self) {
    self->datagrams_decoded = 0;
    self->errors_detected = 0;
    self->working_counter_errors = 0;
}

/**
 * @brief Create a new EtherCAT decoder
 */
GtkTermEtherCATDecoder *gtkterm_ethercat_decoder_new (void) {
    return g_object_new (GTKTERM_TYPE_ETHERCAT_DECODER, NULL);
}
