/************************************************************************/
/* gtkterm_modbus_tcp_decoder.c                                         */
/* ----------------------------                                         */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Modbus TCP protocol decoder implementation                     */
/*                                                                      */
/************************************************************************/

#include "gtkterm_modbus_tcp_decoder.h"
#include <string.h>
#include <stdio.h>

struct _GtkTermModbusTCPDecoder {
    GObject parent_instance;
    guint frames_decoded;
    guint errors_detected;
    guint protocol_errors;
};

static void gtkterm_modbus_tcp_decoder_interface_init (GtkTermProtocolDecoderInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermModbusTCPDecoder, gtkterm_modbus_tcp_decoder, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PROTOCOL_DECODER,
                                                gtkterm_modbus_tcp_decoder_interface_init))

/**
 * @brief Modbus function codes (same as RTU/ASCII)
 */
typedef struct {
    guint8 code;
    const char *name;
    const char *description;
} ModbusTCPFunctionCode;

static const ModbusTCPFunctionCode modbus_functions[] = {
    {0x01, "Read Coils", "Read discrete output coils"},
    {0x02, "Read Discrete Inputs", "Read discrete input contacts"},
    {0x03, "Read Holding Registers", "Read multiple holding registers"},
    {0x04, "Read Input Registers", "Read input registers"},
    {0x05, "Write Single Coil", "Write single output coil"},
    {0x06, "Write Single Register", "Write single holding register"},
    {0x0F, "Write Multiple Coils", "Write multiple coils"},
    {0x10, "Write Multiple Registers", "Write multiple holding registers"},
    {0x14, "Read File Record", "Read file record"},
    {0x15, "Write File Record", "Write file record"},
    {0x16, "Mask Write Register", "Mask write register"},
    {0x17, "Read/Write Multiple Registers", "Read/Write multiple registers"},
    {0x2B, "Read Device Identification", "Read device identification"},
    {0x00, NULL, NULL}
};

/**
 * @brief Get function code description
 */
static const char *get_function_name (guint8 code) {
    // Check for exception (high bit set)
    if (code & 0x80) {
        return "Exception Response";
    }

    for (int i = 0; modbus_functions[i].name != NULL; i++) {
        if (modbus_functions[i].code == code) {
            return modbus_functions[i].name;
        }
    }
    return "Unknown Function";
}

/**
 * @brief Modbus exception codes
 */
static const char *get_exception_name (guint8 code) {
    switch (code) {
        case 0x01: return "Illegal Function";
        case 0x02: return "Illegal Data Address";
        case 0x03: return "Illegal Data Value";
        case 0x04: return "Slave Device Failure";
        case 0x05: return "Acknowledge";
        case 0x06: return "Slave Device Busy";
        case 0x08: return "Memory Parity Error";
        case 0x0A: return "Gateway Path Unavailable";
        case 0x0B: return "Gateway Target Device Failed to Respond";
        default: return "Unknown Exception";
    }
}

/**
 * @brief Get protocol name
 */
static const char *modbus_tcp_decoder_get_name (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "Modbus TCP";
}

/**
 * @brief Get protocol ID
 */
static const char *modbus_tcp_decoder_get_id (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "modbus-tcp";
}

/**
 * @brief Detect Modbus TCP protocol
 */
static gdouble modbus_tcp_decoder_detect (GtkTermProtocolDecoder *decoder,
                                           const guint8 *data,
                                           gsize length) {
    (void)decoder;

    // Modbus TCP minimum: 7 byte MBAP header + 2 byte PDU (func + at least 1 byte)
    if (length < 9)
        return 0.0;

    // Parse MBAP header
    guint16 transaction_id = (data[0] << 8) | data[1];
    guint16 protocol_id = (data[2] << 8) | data[3];
    guint16 pdu_length = (data[4] << 8) | data[5];
    guint8 unit_id = data[6];

    (void)transaction_id; // Can be any value
    (void)unit_id; // Can be 0-247

    // Protocol ID must be 0 for Modbus
    if (protocol_id != 0)
        return 0.0;

    // PDU length should be reasonable (2-253 bytes typical)
    if (pdu_length < 2 || pdu_length > 253)
        return 0.3;

    // Length field should match actual data
    if (length < 6 + pdu_length)
        return 0.5;

    // Check function code
    guint8 func_code = data[7] & 0x7F;
    if (func_code >= 1 && func_code <= 0x2B) {
        return 0.95;
    }

    return 0.6;
}

/**
 * @brief Decode Modbus TCP frame
 */
static char *modbus_tcp_decoder_decode (GtkTermProtocolDecoder *decoder,
                                          const GtkTermDataChunk *chunk) {
    GtkTermModbusTCPDecoder *self = GTKTERM_MODBUS_TCP_DECODER (decoder);
    GString *decoded = g_string_new (NULL);

    if (!chunk || !chunk->data || chunk->length < 9) {
        g_string_printf (decoded, "[Modbus TCP] Frame too short");
        self->errors_detected++;
        return g_string_free (decoded, FALSE);
    }

    const guint8 *data = chunk->data;

    // Parse MBAP header (7 bytes)
    guint16 transaction_id = (data[0] << 8) | data[1];
    guint16 protocol_id = (data[2] << 8) | data[3];
    guint16 pdu_length = (data[4] << 8) | data[5];
    guint8 unit_id = data[6];

    // Verify protocol ID
    if (protocol_id != 0) {
        g_string_printf (decoded, "[Modbus TCP] Invalid protocol ID: %u (expected 0)",
                         protocol_id);
        self->protocol_errors++;
        return g_string_free (decoded, FALSE);
    }

    // Parse PDU
    guint8 func_code = data[7];
    const char *func_name = get_function_name (func_code);
    gboolean is_exception = (func_code & 0x80) != 0;

    g_string_printf (decoded, "[Modbus TCP] TID:0x%04X Unit:%u Func:0x%02X (%s)",
                     transaction_id, unit_id, func_code, func_name);

    // Show PDU length
    g_string_append_printf (decoded, " Len:%u", pdu_length);

    // Parse function-specific data
    if (is_exception) {
        // Exception response
        if (chunk->length >= 9) {
            guint8 exception_code = data[8];
            const char *exception_name = get_exception_name (exception_code);
            g_string_append_printf (decoded, " Exception:0x%02X (%s)",
                                    exception_code, exception_name);
        }
        self->errors_detected++;
    } else {
        // Normal response - show data bytes
        gsize data_length = chunk->length - 8; // Skip MBAP + function code
        if (data_length > 0) {
            g_string_append (decoded, " Data:");
            gsize display_bytes = (data_length > 16) ? 16 : data_length;
            for (gsize i = 0; i < display_bytes; i++) {
                g_string_append_printf (decoded, " %02X", data[8 + i]);
            }
            if (data_length > 16) {
                g_string_append_printf (decoded, "... (%zu more bytes)", data_length - 16);
            }
        }

        // Parse specific function codes
        switch (func_code) {
            case 0x03: // Read Holding Registers
            case 0x04: // Read Input Registers
                if (chunk->length >= 10) {
                    guint8 byte_count = data[8];
                    guint16 reg_count = byte_count / 2;
                    g_string_append_printf (decoded, " [%u registers]", reg_count);
                }
                break;

            case 0x06: // Write Single Register
                if (chunk->length >= 12) {
                    guint16 address = (data[8] << 8) | data[9];
                    guint16 value = (data[10] << 8) | data[11];
                    g_string_append_printf (decoded, " [Addr:0x%04X Val:0x%04X]",
                                            address, value);
                }
                break;

            case 0x10: // Write Multiple Registers
                if (chunk->length >= 12) {
                    guint16 start_addr = (data[8] << 8) | data[9];
                    guint16 reg_count = (data[10] << 8) | data[11];
                    g_string_append_printf (decoded, " [Addr:0x%04X Count:%u]",
                                            start_addr, reg_count);
                }
                break;
        }
    }

    self->frames_decoded++;
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Validate Modbus TCP frame
 */
static gboolean modbus_tcp_decoder_validate (GtkTermProtocolDecoder *decoder,
                                               const guint8 *data,
                                               gsize length,
                                               char **error) {
    (void)decoder;

    if (length < 9) {
        if (error) *error = g_strdup ("Frame too short (min 9 bytes)");
        return FALSE;
    }

    // Check protocol ID
    guint16 protocol_id = (data[2] << 8) | data[3];
    if (protocol_id != 0) {
        if (error) *error = g_strdup_printf ("Invalid protocol ID: %u (must be 0)", protocol_id);
        return FALSE;
    }

    // Check length field
    guint16 pdu_length = (data[4] << 8) | data[5];
    if (pdu_length < 2) {
        if (error) *error = g_strdup_printf ("Invalid PDU length: %u (min 2)", pdu_length);
        return FALSE;
    }

    if (length < 6 + pdu_length) {
        if (error) *error = g_strdup_printf ("Incomplete frame (expected %u bytes, got %zu)",
                                             6 + pdu_length, length);
        return FALSE;
    }

    return TRUE;
}

/**
 * @brief Get decoder statistics
 */
static char *modbus_tcp_decoder_get_stats (GtkTermProtocolDecoder *decoder) {
    GtkTermModbusTCPDecoder *self = GTKTERM_MODBUS_TCP_DECODER (decoder);

    return g_strdup_printf ("Frames: %u, Errors: %u, Protocol Errors: %u",
                            self->frames_decoded,
                            self->errors_detected,
                            self->protocol_errors);
}

/**
 * @brief Reset decoder statistics
 */
static void modbus_tcp_decoder_reset (GtkTermProtocolDecoder *decoder) {
    GtkTermModbusTCPDecoder *self = GTKTERM_MODBUS_TCP_DECODER (decoder);

    self->frames_decoded = 0;
    self->errors_detected = 0;
    self->protocol_errors = 0;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_modbus_tcp_decoder_interface_init (GtkTermProtocolDecoderInterface *iface) {
    iface->get_name = modbus_tcp_decoder_get_name;
    iface->get_id = modbus_tcp_decoder_get_id;
    iface->detect = modbus_tcp_decoder_detect;
    iface->decode = modbus_tcp_decoder_decode;
    iface->validate = modbus_tcp_decoder_validate;
    iface->get_stats = modbus_tcp_decoder_get_stats;
    iface->reset = modbus_tcp_decoder_reset;
}

/**
 * @brief Class initialization
 */
static void gtkterm_modbus_tcp_decoder_class_init (GtkTermModbusTCPDecoderClass *klass) {
    (void)klass;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_modbus_tcp_decoder_init (GtkTermModbusTCPDecoder *self) {
    self->frames_decoded = 0;
    self->errors_detected = 0;
    self->protocol_errors = 0;
}

/**
 * @brief Create a new Modbus TCP decoder
 */
GtkTermModbusTCPDecoder *gtkterm_modbus_tcp_decoder_new (void) {
    return g_object_new (GTKTERM_TYPE_MODBUS_TCP_DECODER, NULL);
}
