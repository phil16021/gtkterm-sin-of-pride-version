/************************************************************************/
/* gtkterm_rs232_decoder.c                                              */
/* -----------------------                                              */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      RS232 control signals decoder implementation                   */
/*                                                                      */
/************************************************************************/

#include "gtkterm_rs232_decoder.h"
#include <string.h>
#include <stdio.h>
#include <ctype.h>

struct _GtkTermRS232Decoder {
    GObject parent_instance;
    guint signals_decoded;
    guint8 last_state;
    guint state_changes;
};

static void gtkterm_rs232_decoder_interface_init (GtkTermProtocolDecoderInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermRS232Decoder, gtkterm_rs232_decoder, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PROTOCOL_DECODER,
                                                gtkterm_rs232_decoder_interface_init))

/**
 * @brief RS232 signal information
 */
typedef struct {
    const char *name;
    const char *abbreviation;
    const char *description;
    GtkTermRS232Signal flag;
} RS232SignalInfo;

static const RS232SignalInfo signal_info[] = {
    {"Data Terminal Ready", "DTR", "Terminal is ready to communicate", GTKTERM_RS232_SIGNAL_DTR},
    {"Data Set Ready", "DSR", "Modem is ready to communicate", GTKTERM_RS232_SIGNAL_DSR},
    {"Request To Send", "RTS", "Terminal requests to send data", GTKTERM_RS232_SIGNAL_RTS},
    {"Clear To Send", "CTS", "Modem is ready to receive data", GTKTERM_RS232_SIGNAL_CTS},
    {"Data Carrier Detect", "DCD", "Carrier signal detected", GTKTERM_RS232_SIGNAL_DCD},
    {"Ring Indicator", "RI", "Phone line is ringing", GTKTERM_RS232_SIGNAL_RI},
    {NULL, NULL, NULL, 0}
};

/**
 * @brief Get protocol name
 */
static const char *rs232_decoder_get_name (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "RS232 Signals";
}

/**
 * @brief Get protocol ID
 */
static const char *rs232_decoder_get_id (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "rs232";
}

/**
 * @brief Detect RS232 signal data
 */
static gdouble rs232_decoder_detect (GtkTermProtocolDecoder *decoder,
                                      const guint8 *data,
                                      gsize length) {
    (void)decoder;

    if (length == 0)
        return 0.0;

    // Format 1: Single byte with signal flags
    if (length == 1) {
        // Valid if at least one signal bit is set and no reserved bits
        guint8 valid_mask = 0x3F; // 6 signal bits
        if ((data[0] & ~valid_mask) == 0 && (data[0] & valid_mask) != 0) {
            return 0.7;
        }
    }

    // Format 2: Text representation (e.g., "DTR:1 RTS:1 CTS:0")
    if (length > 4) {
        const char *str = (const char *)data;

        // Look for signal names
        int signal_count = 0;
        if (strstr (str, "DTR") || strstr (str, "dtr")) signal_count++;
        if (strstr (str, "DSR") || strstr (str, "dsr")) signal_count++;
        if (strstr (str, "RTS") || strstr (str, "rts")) signal_count++;
        if (strstr (str, "CTS") || strstr (str, "cts")) signal_count++;
        if (strstr (str, "DCD") || strstr (str, "dcd")) signal_count++;
        if (strstr (str, "RI") || strstr (str, "ri")) signal_count++;

        if (signal_count >= 2) {
            return 0.85;
        } else if (signal_count == 1) {
            return 0.6;
        }
    }

    return 0.0;
}

/**
 * @brief Parse binary signal state
 */
static char *decode_binary_signals (GtkTermRS232Decoder *self, guint8 signals) {
    GString *decoded = g_string_new ("[RS232] ");

    gboolean first = TRUE;

    for (int i = 0; signal_info[i].name != NULL; i++) {
        gboolean is_set = (signals & signal_info[i].flag) != 0;

        if (!first) {
            g_string_append (decoded, ", ");
        }
        first = FALSE;

        g_string_append_printf (decoded, "%s:%s",
                                signal_info[i].abbreviation,
                                is_set ? "HIGH" : "LOW");
    }

    // Check for state changes
    if (self->last_state != signals) {
        g_string_append (decoded, " [CHANGED: ");

        gboolean first_change = TRUE;
        for (int i = 0; signal_info[i].name != NULL; i++) {
            gboolean old_state = (self->last_state & signal_info[i].flag) != 0;
            gboolean new_state = (signals & signal_info[i].flag) != 0;

            if (old_state != new_state) {
                if (!first_change) {
                    g_string_append (decoded, ", ");
                }
                first_change = FALSE;

                g_string_append_printf (decoded, "%s:%s→%s",
                                        signal_info[i].abbreviation,
                                        old_state ? "1" : "0",
                                        new_state ? "1" : "0");
            }
        }

        g_string_append (decoded, "]");
        self->state_changes++;
        self->last_state = signals;
    }

    return g_string_free (decoded, FALSE);
}

/**
 * @brief Parse text signal representation
 */
static char *decode_text_signals (GtkTermRS232Decoder *self, const char *str) {
    GString *decoded = g_string_new ("[RS232] ");

    guint8 signals = 0;
    gboolean found_any = FALSE;

    // Parse each signal
    for (int i = 0; signal_info[i].name != NULL; i++) {
        // Look for signal name (case-insensitive)
        const char *pos = strcasestr (str, signal_info[i].abbreviation);

        if (pos) {
            found_any = TRUE;

            // Try to find state after the name
            const char *p = pos + strlen (signal_info[i].abbreviation);

            // Skip whitespace and separators
            while (*p && (isspace (*p) || *p == ':' || *p == '=')) p++;

            // Check for 1/0, on/off, high/low, true/false
            gboolean is_set = FALSE;

            if (*p == '1' || *p == 'H' || *p == 'h') {
                is_set = TRUE;
            } else if (strncasecmp (p, "on", 2) == 0) {
                is_set = TRUE;
            } else if (strncasecmp (p, "high", 4) == 0) {
                is_set = TRUE;
            } else if (strncasecmp (p, "true", 4) == 0) {
                is_set = TRUE;
            }

            if (is_set) {
                signals |= signal_info[i].flag;
            }
        }
    }

    if (!found_any) {
        g_string_append (decoded, "No signals detected");
    } else {
        // Use the binary decoder to format output
        g_free (decode_binary_signals (self, signals));
        return decode_binary_signals (self, signals);
    }

    return g_string_free (decoded, FALSE);
}

/**
 * @brief Decode RS232 signals
 */
static char *rs232_decoder_decode (GtkTermProtocolDecoder *decoder,
                                    const GtkTermDataChunk *chunk) {
    GtkTermRS232Decoder *self = GTKTERM_RS232_DECODER (decoder);

    if (!chunk || !chunk->data || chunk->length == 0)
        return g_strdup ("[RS232] Empty data");

    self->signals_decoded++;

    // Try binary format first (1 byte)
    if (chunk->length == 1) {
        return decode_binary_signals (self, chunk->data[0]);
    }

    // Try text format
    char *str = g_strndup ((const char *)chunk->data, chunk->length);
    g_strstrip (str);

    char *result = decode_text_signals (self, str);
    g_free (str);

    return result;
}

/**
 * @brief Validate RS232 signal data
 */
static gboolean rs232_decoder_validate (GtkTermProtocolDecoder *decoder,
                                          const guint8 *data,
                                          gsize length,
                                          char **error) {
    (void)decoder;

    if (length == 0) {
        if (error) *error = g_strdup ("Empty data");
        return FALSE;
    }

    // Binary format: single byte
    if (length == 1) {
        guint8 valid_mask = 0x3F; // 6 signal bits
        if ((data[0] & ~valid_mask) != 0) {
            if (error) *error = g_strdup_printf ("Invalid signal bits: 0x%02X", data[0]);
            return FALSE;
        }
    }

    // Text format: should be printable ASCII
    if (length > 1) {
        for (gsize i = 0; i < length; i++) {
            if (data[i] < 0x20 && data[i] != '\r' && data[i] != '\n' && data[i] != '\t') {
                if (error) *error = g_strdup_printf ("Non-printable character at position %zu", i);
                return FALSE;
            }
        }
    }

    return TRUE;
}

/**
 * @brief Get decoder statistics
 */
static char *rs232_decoder_get_stats (GtkTermProtocolDecoder *decoder) {
    GtkTermRS232Decoder *self = GTKTERM_RS232_DECODER (decoder);

    return g_strdup_printf ("Signals Decoded: %u, State Changes: %u, Current State: 0x%02X",
                            self->signals_decoded,
                            self->state_changes,
                            self->last_state);
}

/**
 * @brief Reset decoder statistics
 */
static void rs232_decoder_reset (GtkTermProtocolDecoder *decoder) {
    GtkTermRS232Decoder *self = GTKTERM_RS232_DECODER (decoder);

    self->signals_decoded = 0;
    self->last_state = 0;
    self->state_changes = 0;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_rs232_decoder_interface_init (GtkTermProtocolDecoderInterface *iface) {
    iface->get_name = rs232_decoder_get_name;
    iface->get_id = rs232_decoder_get_id;
    iface->detect = rs232_decoder_detect;
    iface->decode = rs232_decoder_decode;
    iface->validate = rs232_decoder_validate;
    iface->get_stats = rs232_decoder_get_stats;
    iface->reset = rs232_decoder_reset;
}

/**
 * @brief Class initialization
 */
static void gtkterm_rs232_decoder_class_init (GtkTermRS232DecoderClass *klass) {
    (void)klass;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_rs232_decoder_init (GtkTermRS232Decoder *self) {
    self->signals_decoded = 0;
    self->last_state = 0;
    self->state_changes = 0;
}

/**
 * @brief Create a new RS232 decoder
 */
GtkTermRS232Decoder *gtkterm_rs232_decoder_new (void) {
    return g_object_new (GTKTERM_TYPE_RS232_DECODER, NULL);
}
