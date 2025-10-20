/************************************************************************/
/* gtkterm_can_decoder.c                                                */
/* ---------------------                                                */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      CAN bus protocol decoder implementation                        */
/*                                                                      */
/************************************************************************/

#include "gtkterm_can_decoder.h"
#include <string.h>
#include <stdio.h>
#include <ctype.h>

struct _GtkTermCANDecoder {
    GObject parent_instance;
    GtkTermCANFormat format;
    guint frames_decoded;
    guint extended_frames;
    guint rtr_frames;
    guint errors_detected;
};

static void gtkterm_can_decoder_interface_init (GtkTermProtocolDecoderInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermCANDecoder, gtkterm_can_decoder, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PROTOCOL_DECODER,
                                                gtkterm_can_decoder_interface_init))

/**
 * @brief Common CAN IDs and their meanings (example automotive)
 */
typedef struct {
    guint32 id;
    const char *description;
} CANIDInfo;

static const CANIDInfo common_can_ids[] = {
    {0x000, "NMT - Network Management"},
    {0x100, "Engine RPM"},
    {0x110, "Vehicle Speed"},
    {0x200, "Transmission Status"},
    {0x280, "Airbag Status"},
    {0x300, "ABS Status"},
    {0x400, "Climate Control"},
    {0x500, "Body Control Module"},
    {0x600, "Instrument Cluster"},
    {0x700, "Lighting Control"},
    {0x7DF, "OBD-II Functional Request"},
    {0x7E0, "OBD-II ECU Response"},
    {0x7E8, "OBD-II ECU Response"},
    {0x000, NULL}
};

/**
 * @brief Get CAN ID description
 */
static const char *get_can_id_description (guint32 id) {
    for (int i = 0; common_can_ids[i].description != NULL; i++) {
        if (common_can_ids[i].id == id) {
            return common_can_ids[i].description;
        }
    }
    return NULL;
}

/**
 * @brief Get protocol name
 */
static const char *can_decoder_get_name (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "CAN Bus";
}

/**
 * @brief Get protocol ID
 */
static const char *can_decoder_get_id (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "can";
}

/**
 * @brief Detect CAN protocol
 */
static gdouble can_decoder_detect (GtkTermProtocolDecoder *decoder,
                                    const guint8 *data,
                                    gsize length) {
    GtkTermCANDecoder *self = GTKTERM_CAN_DECODER (decoder);

    if (self->format == GTKTERM_CAN_FORMAT_CANDUMP || self->format == GTKTERM_CAN_FORMAT_AUTO) {
        // candump format: "  can0  123   [8]  01 02 03 04 05 06 07 08"
        // Look for pattern: optional spaces, "can", digit, spaces, hex, "[", digit, "]"
        if (length > 15) {
            const char *str = (const char *)data;
            // Look for "can" interface name
            const char *can_str = strstr (str, "can");
            if (can_str && (can_str - str) < 10) {
                // Look for DLC in brackets
                const char *bracket = strchr (can_str, '[');
                if (bracket && bracket[2] == ']') {
                    char dlc_char = bracket[1];
                    if (dlc_char >= '0' && dlc_char <= '8') {
                        return 0.95;
                    }
                }
            }
        }
    }

    if (self->format == GTKTERM_CAN_FORMAT_BINARY || self->format == GTKTERM_CAN_FORMAT_AUTO) {
        // Binary CAN frame: minimum 5 bytes (ID + DLC + at least 0 data bytes + CRC)
        // Standard frame structure varies, but typically:
        // - 4 bytes ID (may include flags)
        // - 1 byte DLC
        // - 0-8 bytes data
        if (length >= 5 && length <= 13) {
            guint8 dlc = data[4];
            if (dlc <= 8 && length >= (5 + dlc)) {
                return 0.7;
            }
        }
    }

    return 0.0;
}

/**
 * @brief Parse candump text format
 */
static char *decode_candump_text (GtkTermCANDecoder *self, const char *str) {
    GString *decoded = g_string_new (NULL);

    // Example: "  can0  123   [8]  01 02 03 04 05 06 07 08"
    // Example: "  can1  18FEF100  [8]  11 22 33 44 55 66 77 88"

    char interface[16] = {0};
    char id_str[16] = {0};
    int dlc = 0;
    guint8 data[8] = {0};

    // Parse the candump format
    const char *p = str;

    // Skip leading whitespace
    while (*p && isspace (*p)) p++;

    // Read interface name (e.g., "can0")
    int if_len = 0;
    while (*p && !isspace (*p) && if_len < 15) {
        interface[if_len++] = *p++;
    }

    // Skip whitespace
    while (*p && isspace (*p)) p++;

    // Read CAN ID (hex)
    int id_len = 0;
    while (*p && (isxdigit (*p) || *p == '#') && id_len < 15) {
        if (*p != '#')
            id_str[id_len++] = *p;
        p++;
    }

    // Skip whitespace
    while (*p && isspace (*p)) p++;

    // Read DLC in brackets [n]
    if (*p == '[') {
        p++;
        if (isdigit (*p)) {
            dlc = *p - '0';
            p++;
        }
        if (*p == ']') p++;
    }

    // Skip whitespace
    while (*p && isspace (*p)) p++;

    // Read data bytes
    int data_count = 0;
    while (*p && data_count < 8) {
        if (isxdigit (*p)) {
            unsigned int byte;
            if (sscanf (p, "%02x", &byte) == 1) {
                data[data_count++] = byte;
                p += 2;
                while (*p && isspace (*p)) p++;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Parse ID and detect extended format
    guint32 can_id = 0;
    gboolean is_extended = FALSE;
    gboolean is_rtr = FALSE;

    sscanf (id_str, "%x", &can_id);

    if (can_id > 0x7FF) {
        is_extended = TRUE;
        self->extended_frames++;
    }

    // Check for RTR in original string
    if (strstr (str, "RTR") || strstr (str, "rtr")) {
        is_rtr = TRUE;
        self->rtr_frames++;
    }

    // Build decoded output
    g_string_printf (decoded, "[CAN] %s ID:0x%03X%s",
                     interface,
                     can_id,
                     is_extended ? " (EXT)" : " (STD)");

    if (is_rtr) {
        g_string_append (decoded, " RTR");
    }

    g_string_append_printf (decoded, " DLC:%d", dlc);

    if (data_count > 0) {
        g_string_append (decoded, " Data:");
        for (int i = 0; i < data_count; i++) {
            g_string_append_printf (decoded, " %02X", data[i]);
        }
    }

    // Add description if known
    const char *desc = get_can_id_description (can_id);
    if (desc) {
        g_string_append_printf (decoded, " (%s)", desc);
    }

    self->frames_decoded++;
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Parse binary CAN frame
 */
static char *decode_binary_frame (GtkTermCANDecoder *self, const guint8 *data, gsize length) {
    GString *decoded = g_string_new (NULL);

    if (length < 5) {
        g_string_printf (decoded, "[CAN] Frame too short");
        self->errors_detected++;
        return g_string_free (decoded, FALSE);
    }

    // Simple binary format interpretation:
    // Bytes 0-3: CAN ID (little-endian)
    // Byte 4: DLC
    // Bytes 5+: Data

    guint32 can_id = data[0] | (data[1] << 8) | (data[2] << 16) | (data[3] << 24);
    guint8 dlc = data[4];

    // Check for extended frame flag (MSB of ID)
    gboolean is_extended = (can_id & 0x80000000) != 0;
    gboolean is_rtr = (can_id & 0x40000000) != 0;

    // Mask out flags
    can_id &= 0x1FFFFFFF;

    if (is_extended) {
        self->extended_frames++;
    }

    if (is_rtr) {
        self->rtr_frames++;
    }

    g_string_printf (decoded, "[CAN] ID:0x%03X%s",
                     can_id,
                     is_extended ? " (EXT)" : " (STD)");

    if (is_rtr) {
        g_string_append (decoded, " RTR");
    }

    g_string_append_printf (decoded, " DLC:%u", dlc);

    if (dlc > 8) {
        g_string_append (decoded, " [INVALID DLC]");
        self->errors_detected++;
        dlc = 8;
    }

    if (length >= 5 + dlc) {
        g_string_append (decoded, " Data:");
        for (guint8 i = 0; i < dlc; i++) {
            g_string_append_printf (decoded, " %02X", data[5 + i]);
        }
    } else {
        g_string_append (decoded, " [INCOMPLETE DATA]");
        self->errors_detected++;
    }

    // Add description if known
    const char *desc = get_can_id_description (can_id);
    if (desc) {
        g_string_append_printf (decoded, " (%s)", desc);
    }

    self->frames_decoded++;
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Decode CAN frame
 */
static char *can_decoder_decode (GtkTermProtocolDecoder *decoder,
                                  const GtkTermDataChunk *chunk) {
    GtkTermCANDecoder *self = GTKTERM_CAN_DECODER (decoder);

    if (!chunk || !chunk->data || chunk->length == 0)
        return g_strdup ("[CAN] Empty frame");

    // Try to auto-detect format
    if (self->format == GTKTERM_CAN_FORMAT_AUTO) {
        // Check if it looks like candump text
        const char *str = (const char *)chunk->data;
        if (strstr (str, "can") && strchr (str, '[')) {
            return decode_candump_text (self, str);
        } else {
            return decode_binary_frame (self, chunk->data, chunk->length);
        }
    } else if (self->format == GTKTERM_CAN_FORMAT_CANDUMP) {
        char *str = g_strndup ((const char *)chunk->data, chunk->length);
        char *result = decode_candump_text (self, str);
        g_free (str);
        return result;
    } else {
        return decode_binary_frame (self, chunk->data, chunk->length);
    }
}

/**
 * @brief Validate CAN frame
 */
static gboolean can_decoder_validate (GtkTermProtocolDecoder *decoder,
                                        const guint8 *data,
                                        gsize length,
                                        char **error) {
    GtkTermCANDecoder *self = GTKTERM_CAN_DECODER (decoder);

    if (length == 0) {
        if (error) *error = g_strdup ("Empty data");
        return FALSE;
    }

    if (self->format == GTKTERM_CAN_FORMAT_BINARY) {
        if (length < 5) {
            if (error) *error = g_strdup ("Binary frame too short (min 5 bytes)");
            return FALSE;
        }

        guint8 dlc = data[4];
        if (dlc > 8) {
            if (error) *error = g_strdup_printf ("Invalid DLC: %u (max 8)", dlc);
            return FALSE;
        }

        if (length < 5 + dlc) {
            if (error) *error = g_strdup_printf ("Incomplete data (expected %u bytes, got %zu)", 5 + dlc, length);
            return FALSE;
        }
    }

    return TRUE;
}

/**
 * @brief Get decoder statistics
 */
static char *can_decoder_get_stats (GtkTermProtocolDecoder *decoder) {
    GtkTermCANDecoder *self = GTKTERM_CAN_DECODER (decoder);

    return g_strdup_printf ("Frames: %u, Extended: %u, RTR: %u, Errors: %u",
                            self->frames_decoded,
                            self->extended_frames,
                            self->rtr_frames,
                            self->errors_detected);
}

/**
 * @brief Reset decoder statistics
 */
static void can_decoder_reset (GtkTermProtocolDecoder *decoder) {
    GtkTermCANDecoder *self = GTKTERM_CAN_DECODER (decoder);

    self->frames_decoded = 0;
    self->extended_frames = 0;
    self->rtr_frames = 0;
    self->errors_detected = 0;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_can_decoder_interface_init (GtkTermProtocolDecoderInterface *iface) {
    iface->get_name = can_decoder_get_name;
    iface->get_id = can_decoder_get_id;
    iface->detect = can_decoder_detect;
    iface->decode = can_decoder_decode;
    iface->validate = can_decoder_validate;
    iface->get_stats = can_decoder_get_stats;
    iface->reset = can_decoder_reset;
}

/**
 * @brief Class initialization
 */
static void gtkterm_can_decoder_class_init (GtkTermCANDecoderClass *klass) {
    (void)klass;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_can_decoder_init (GtkTermCANDecoder *self) {
    self->format = GTKTERM_CAN_FORMAT_AUTO;
    self->frames_decoded = 0;
    self->extended_frames = 0;
    self->rtr_frames = 0;
    self->errors_detected = 0;
}

/**
 * @brief Create a new CAN decoder
 */
GtkTermCANDecoder *gtkterm_can_decoder_new (GtkTermCANFormat format) {
    GtkTermCANDecoder *decoder = g_object_new (GTKTERM_TYPE_CAN_DECODER, NULL);
    decoder->format = format;
    return decoder;
}
