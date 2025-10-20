/************************************************************************/
/* gtkterm_modbus_decoder.c                                             */
/* ------------------------                                             */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Modbus ASCII/RTU protocol decoder implementation               */
/*                                                                      */
/************************************************************************/

#include "gtkterm_modbus_decoder.h"
#include <string.h>
#include <stdio.h>

struct _GtkTermModbusDecoder {
    GObject parent_instance;
    GtkTermModbusMode mode;
    guint frames_decoded;
    guint errors_detected;
    guint checksum_errors;
};

static void gtkterm_modbus_decoder_interface_init (GtkTermProtocolDecoderInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermModbusDecoder, gtkterm_modbus_decoder, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PROTOCOL_DECODER,
                                                gtkterm_modbus_decoder_interface_init))

/**
 * @brief Modbus function codes
 */
typedef struct {
    guint8 code;
    const char *name;
    const char *description;
} ModbusFunctionCode;

static const ModbusFunctionCode modbus_functions[] = {
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
 * @brief Calculate Modbus LRC (Longitudinal Redundancy Check)
 */
static guint8 calculate_lrc (const guint8 *data, gsize length) {
    guint8 lrc = 0;
    for (gsize i = 0; i < length; i++) {
        lrc += data[i];
    }
    return ((lrc ^ 0xFF) + 1) & 0xFF;
}

/**
 * @brief Calculate Modbus CRC16
 */
static guint16 calculate_crc16 (const guint8 *data, gsize length) {
    guint16 crc = 0xFFFF;

    for (gsize i = 0; i < length; i++) {
        crc ^= data[i];
        for (int j = 0; j < 8; j++) {
            if (crc & 0x0001) {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    return crc;
}

/**
 * @brief Convert ASCII hex char to byte
 */
static guint8 hex_to_byte (char high, char low) {
    guint8 h = (high >= 'A') ? (high - 'A' + 10) : (high - '0');
    guint8 l = (low >= 'A') ? (low - 'A' + 10) : (low - '0');
    return (h << 4) | l;
}

/**
 * @brief Get protocol name
 */
static const char *modbus_decoder_get_name (GtkTermProtocolDecoder *decoder) {
    GtkTermModbusDecoder *self = GTKTERM_MODBUS_DECODER (decoder);
    return (self->mode == GTKTERM_MODBUS_ASCII) ? "Modbus ASCII" : "Modbus RTU";
}

/**
 * @brief Get protocol ID
 */
static const char *modbus_decoder_get_id (GtkTermProtocolDecoder *decoder) {
    GtkTermModbusDecoder *self = GTKTERM_MODBUS_DECODER (decoder);
    return (self->mode == GTKTERM_MODBUS_ASCII) ? "modbus-ascii" : "modbus-rtu";
}

/**
 * @brief Detect Modbus protocol
 */
static gdouble modbus_decoder_detect (GtkTermProtocolDecoder *decoder,
                                       const guint8 *data,
                                       gsize length) {
    GtkTermModbusDecoder *self = GTKTERM_MODBUS_DECODER (decoder);

    if (self->mode == GTKTERM_MODBUS_ASCII) {
        // ASCII format: starts with ':', ends with CR/LF
        if (length < 9) // Minimum: :AAFFCC\r\n
            return 0.0;

        if (data[0] == ':' && (data[length-1] == '\n' || data[length-2] == '\r')) {
            // Check if it contains only hex characters
            for (gsize i = 1; i < length - 2; i++) {
                char c = data[i];
                if (!((c >= '0' && c <= '9') || (c >= 'A' && c <= 'F') ||
                      (c >= 'a' && c <= 'f') || c == '\r' || c == '\n')) {
                    return 0.0;
                }
            }
            return 0.9;
        }
    } else {
        // RTU format: binary, minimum 4 bytes (addr + func + CRC)
        if (length < 4)
            return 0.0;

        // Check if slave address is reasonable (1-247)
        if (data[0] >= 1 && data[0] <= 247) {
            // Check if function code is valid
            guint8 func = data[1] & 0x7F;
            if (func >= 1 && func <= 0x2B) {
                return 0.7;
            }
        }
    }

    return 0.0;
}

/**
 * @brief Decode Modbus ASCII frame
 */
static char *decode_ascii_frame (GtkTermModbusDecoder *self, const guint8 *data, gsize length) {
    GString *decoded = g_string_new (NULL);

    // Parse ASCII frame: :AAFFDD...DDCC\r\n
    if (length < 9 || data[0] != ':') {
        g_string_printf (decoded, "[MODBUS-ASCII] Invalid frame");
        self->errors_detected++;
        return g_string_free (decoded, FALSE);
    }

    // Convert ASCII hex to bytes
    guint8 slave_addr = hex_to_byte (data[1], data[2]);
    guint8 func_code = hex_to_byte (data[3], data[4]);

    const char *func_name = get_function_name (func_code);
    gboolean is_exception = (func_code & 0x80) != 0;

    g_string_printf (decoded, "[MODBUS-ASCII] Slave:%02X Func:%02X (%s)",
                     slave_addr, func_code, func_name);

    // Extract data bytes (everything between function and LRC)
    gsize data_hex_len = length - 7; // Skip :AA FF and CC\r\n
    if (data_hex_len > 0 && (data_hex_len % 2) == 0) {
        g_string_append (decoded, " Data:");
        for (gsize i = 5; i < 5 + data_hex_len; i += 2) {
            guint8 byte = hex_to_byte (data[i], data[i+1]);
            g_string_append_printf (decoded, " %02X", byte);
        }
    }

    // Verify LRC
    guint8 received_lrc = hex_to_byte (data[length-4], data[length-3]);
    // Calculate LRC on the binary data (addr + func + data)
    gsize bin_length = (length - 5) / 2; // Number of binary bytes
    guint8 *bin_data = g_malloc (bin_length);
    for (gsize i = 0; i < bin_length; i++) {
        bin_data[i] = hex_to_byte (data[1 + i*2], data[2 + i*2]);
    }
    guint8 calc_lrc = calculate_lrc (bin_data, bin_length);
    g_free (bin_data);

    if (received_lrc == calc_lrc) {
        g_string_append (decoded, " [LRC:OK]");
    } else {
        g_string_append_printf (decoded, " [LRC:FAIL rx:%02X calc:%02X]",
                                received_lrc, calc_lrc);
        self->checksum_errors++;
    }

    if (is_exception) {
        self->errors_detected++;
    }

    self->frames_decoded++;
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Decode Modbus RTU frame
 */
static char *decode_rtu_frame (GtkTermModbusDecoder *self, const guint8 *data, gsize length) {
    GString *decoded = g_string_new (NULL);

    if (length < 4) {
        g_string_printf (decoded, "[MODBUS-RTU] Frame too short");
        self->errors_detected++;
        return g_string_free (decoded, FALSE);
    }

    guint8 slave_addr = data[0];
    guint8 func_code = data[1];

    const char *func_name = get_function_name (func_code);
    gboolean is_exception = (func_code & 0x80) != 0;

    // BUG FIX #5: Check for reserved function codes per Modbus specification
    // Modbus spec Section 5 lists reserved function codes that should be marked
    gboolean is_reserved = FALSE;
    if (func_code == 0x09 || func_code == 0x0A ||
        func_code == 0x0D || func_code == 0x0E ||
        func_code == 0x29 || func_code == 0x2A ||
        (func_code >= 0x5A && func_code <= 0x63) ||
        (func_code >= 0x64 && func_code <= 0x7F)) {
        is_reserved = TRUE;
    }

    if (is_reserved) {
        g_string_printf (decoded, "[MODBUS-RTU] Slave:%02X Func:%02X (RESERVED)",
                         slave_addr, func_code);
    } else {
        g_string_printf (decoded, "[MODBUS-RTU] Slave:%02X Func:%02X (%s)",
                         slave_addr, func_code, func_name);
    }

    // Extract data (everything between function and CRC)
    gsize data_length = length - 4; // Exclude addr, func, and 2-byte CRC
    if (data_length > 0) {
        g_string_append (decoded, " Data:");
        for (gsize i = 2; i < length - 2; i++) {
            g_string_append_printf (decoded, " %02X", data[i]);
        }
    }

    // Verify CRC
    guint16 received_crc = data[length-2] | (data[length-1] << 8);
    guint16 calc_crc = calculate_crc16 (data, length - 2);

    if (received_crc == calc_crc) {
        g_string_append (decoded, " [CRC:OK]");
    } else {
        g_string_append_printf (decoded, " [CRC:FAIL rx:%04X calc:%04X]",
                                received_crc, calc_crc);
        self->checksum_errors++;
    }

    if (is_exception) {
        self->errors_detected++;
    }

    self->frames_decoded++;
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Decode Modbus frame
 */
static char *modbus_decoder_decode (GtkTermProtocolDecoder *decoder,
                                     const GtkTermDataChunk *chunk) {
    GtkTermModbusDecoder *self = GTKTERM_MODBUS_DECODER (decoder);

    if (!chunk || !chunk->data || chunk->length == 0)
        return g_strdup ("[MODBUS] Empty frame");

    if (self->mode == GTKTERM_MODBUS_ASCII) {
        return decode_ascii_frame (self, chunk->data, chunk->length);
    } else {
        return decode_rtu_frame (self, chunk->data, chunk->length);
    }
}

/**
 * @brief Validate Modbus frame
 */
static gboolean modbus_decoder_validate (GtkTermProtocolDecoder *decoder,
                                          const guint8 *data,
                                          gsize length,
                                          char **error) {
    GtkTermModbusDecoder *self = GTKTERM_MODBUS_DECODER (decoder);

    if (self->mode == GTKTERM_MODBUS_ASCII) {
        if (length < 9) {
            if (error) *error = g_strdup ("Frame too short (min 9 bytes)");
            return FALSE;
        }
        if (data[0] != ':') {
            if (error) *error = g_strdup ("Missing start character ':'");
            return FALSE;
        }
        if (data[length-1] != '\n' && data[length-2] != '\r') {
            if (error) *error = g_strdup ("Missing CR/LF terminator");
            return FALSE;
        }
    } else {
        if (length < 4) {
            if (error) *error = g_strdup ("Frame too short (min 4 bytes)");
            return FALSE;
        }
        if (data[0] == 0 || data[0] > 247) {
            if (error) *error = g_strdup_printf ("Invalid slave address: %d", data[0]);
            return FALSE;
        }
    }

    return TRUE;
}

/**
 * @brief Get decoder statistics
 */
static char *modbus_decoder_get_stats (GtkTermProtocolDecoder *decoder) {
    GtkTermModbusDecoder *self = GTKTERM_MODBUS_DECODER (decoder);

    return g_strdup_printf ("Frames: %u, Errors: %u, Checksum Errors: %u",
                            self->frames_decoded,
                            self->errors_detected,
                            self->checksum_errors);
}

/**
 * @brief Reset decoder statistics
 */
static void modbus_decoder_reset (GtkTermProtocolDecoder *decoder) {
    GtkTermModbusDecoder *self = GTKTERM_MODBUS_DECODER (decoder);

    self->frames_decoded = 0;
    self->errors_detected = 0;
    self->checksum_errors = 0;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_modbus_decoder_interface_init (GtkTermProtocolDecoderInterface *iface) {
    iface->get_name = modbus_decoder_get_name;
    iface->get_id = modbus_decoder_get_id;
    iface->detect = modbus_decoder_detect;
    iface->decode = modbus_decoder_decode;
    iface->validate = modbus_decoder_validate;
    iface->get_stats = modbus_decoder_get_stats;
    iface->reset = modbus_decoder_reset;
}

/**
 * @brief Class initialization
 */
static void gtkterm_modbus_decoder_class_init (GtkTermModbusDecoderClass *klass) {
    (void)klass;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_modbus_decoder_init (GtkTermModbusDecoder *self) {
    self->mode = GTKTERM_MODBUS_ASCII;
    self->frames_decoded = 0;
    self->errors_detected = 0;
    self->checksum_errors = 0;
}

/**
 * @brief Create a new Modbus decoder
 */
GtkTermModbusDecoder *gtkterm_modbus_decoder_new (GtkTermModbusMode mode) {
    GtkTermModbusDecoder *decoder = g_object_new (GTKTERM_TYPE_MODBUS_DECODER, NULL);
    decoder->mode = mode;
    return decoder;
}
