/************************************************************************/
/* gtkterm_at_decoder.c                                                 */
/* --------------------                                                 */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      AT Command protocol decoder implementation                     */
/*                                                                      */
/************************************************************************/

#include "gtkterm_at_decoder.h"
#include <string.h>
#include <ctype.h>

struct _GtkTermATDecoder {
    GObject parent_instance;
    guint commands_decoded;
    guint responses_decoded;
    guint errors_detected;
};

static void gtkterm_at_decoder_interface_init (GtkTermProtocolDecoderInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermATDecoder, gtkterm_at_decoder, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PROTOCOL_DECODER,
                                                gtkterm_at_decoder_interface_init))

/**
 * @brief Known AT commands and their descriptions
 */
typedef struct {
    const char *command;
    const char *description;
} ATCommandInfo;

static const ATCommandInfo at_commands[] = {
    {"AT", "Attention - basic command"},
    {"ATA", "Answer incoming call"},
    {"ATD", "Dial number"},
    {"ATE", "Command echo"},
    {"ATH", "Hook control (hang up)"},
    {"ATI", "Request identification"},
    {"ATL", "Speaker loudness"},
    {"ATM", "Speaker mode"},
    {"ATO", "Return to online mode"},
    {"ATQ", "Quiet mode"},
    {"ATS", "S-Register access"},
    {"ATV", "Verbose response mode"},
    {"ATZ", "Reset to default configuration"},
    {"AT&C", "DCD signal control"},
    {"AT&D", "DTR signal control"},
    {"AT&F", "Factory defaults"},
    {"AT&V", "View configuration"},
    {"AT&W", "Store profile"},
    {"AT+CGMI", "Manufacturer identification"},
    {"AT+CGMM", "Model identification"},
    {"AT+CGMR", "Revision identification"},
    {"AT+CGSN", "Serial number"},
    {"AT+CIMI", "IMSI"},
    {"AT+CCID", "SIM card ID"},
    {"AT+CREG", "Network registration"},
    {"AT+COPS", "Operator selection"},
    {"AT+CSQ", "Signal quality"},
    {"AT+CPIN", "Enter PIN"},
    {"AT+CMGF", "Message format"},
    {"AT+CMGS", "Send message"},
    {"AT+CMGR", "Read message"},
    {"AT+CMGD", "Delete message"},
    {"AT+CIPSTART", "Start TCP connection"},
    {"AT+CIPCLOSE", "Close TCP connection"},
    {"AT+CIPSEND", "Send TCP data"},
    {"AT+CIPSTATUS", "Connection status"},
    {NULL, NULL}
};

/**
 * @brief Get AT command description
 */
static const char *get_at_command_description (const char *cmd) {
    for (int i = 0; at_commands[i].command != NULL; i++) {
        if (g_str_has_prefix (cmd, at_commands[i].command)) {
            return at_commands[i].description;
        }
    }
    return "Unknown AT command";
}

/**
 * @brief Get protocol name
 */
static const char *at_decoder_get_name (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "AT Commands";
}

/**
 * @brief Get protocol ID
 */
static const char *at_decoder_get_id (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "at";
}

/**
 * @brief Detect AT command protocol
 */
static gdouble at_decoder_detect (GtkTermProtocolDecoder *decoder,
                                   const guint8 *data,
                                   gsize length) {
    (void)decoder;

    if (length < 2)
        return 0.0;

    // Check for AT command prefix
    if ((data[0] == 'A' || data[0] == 'a') &&
        (data[1] == 'T' || data[1] == 't')) {
        return 0.9; // Very likely AT command
    }

    // Check for common responses
    if (length >= 2) {
        if (g_str_has_prefix ((const char *)data, "OK") ||
            g_str_has_prefix ((const char *)data, "ERROR") ||
            g_str_has_prefix ((const char *)data, "+")) {
            return 0.7; // Likely AT response
        }
    }

    if (length >= 4) {
        if (g_str_has_prefix ((const char *)data, "RING") ||
            g_str_has_prefix ((const char *)data, "BUSY") ||
            g_str_has_prefix ((const char *)data, "CONNECT")) {
            return 0.8; // Likely AT response
        }
    }

    return 0.0;
}

/**
 * @brief Decode AT command or response
 */
static char *at_decoder_decode (GtkTermProtocolDecoder *decoder,
                                 const GtkTermDataChunk *chunk) {
    GtkTermATDecoder *self = GTKTERM_AT_DECODER (decoder);
    GString *decoded = g_string_new (NULL);

    if (!chunk || !chunk->data || chunk->length == 0)
        return g_string_free (decoded, FALSE);

    // Create null-terminated string for easier parsing
    char *str = g_strndup ((const char *)chunk->data, chunk->length);
    g_strstrip (str); // Remove leading/trailing whitespace

    // BUG FIX #6: Enforce ITU-T V.250 requirement that commands must start with "AT"
    // Section 5.2.1: "The command line shall start with the characters AT"
    if (chunk->direction == GTKTERM_DATA_DIRECTION_TX) {
        // TX direction = commands, which MUST start with AT
        if (!g_str_has_prefix (str, "AT") && !g_str_has_prefix (str, "at")) {
            g_string_printf (decoded, "[ERR] Invalid AT command (missing 'AT' prefix): %s", str);
            self->errors_detected++;
            g_free (str);
            return g_string_free (decoded, FALSE);
        }
    }

    // Determine if it's a command or response
    if (g_str_has_prefix (str, "AT") || g_str_has_prefix (str, "at")) {
        // AT Command
        self->commands_decoded++;
        g_string_append (decoded, "[CMD] ");
        g_string_append (decoded, str);

        const char *desc = get_at_command_description (str);
        g_string_append_printf (decoded, " → %s", desc);

        // Parse parameters if present
        char *eq = strchr (str, '=');
        char *question = strchr (str, '?');

        if (question) {
            g_string_append (decoded, " (Query)");
        } else if (eq) {
            g_string_append_printf (decoded, " (Set: %s)", eq + 1);
        }

    } else if (g_strcmp0 (str, "OK") == 0) {
        // Success response
        self->responses_decoded++;
        g_string_append (decoded, "[RSP] OK - Command successful");

    } else if (g_str_has_prefix (str, "ERROR")) {
        // Error response
        self->errors_detected++;
        g_string_append (decoded, "[ERR] ");
        g_string_append (decoded, str);

    } else if (g_str_has_prefix (str, "+CME ERROR") ||
               g_str_has_prefix (str, "+CMS ERROR")) {
        // CME/CMS Error
        self->errors_detected++;
        char **parts = g_strsplit (str, ":", 2);
        if (parts[1]) {
            g_string_append_printf (decoded, "[ERR] %s: Error code %s",
                                    parts[0], g_strstrip (parts[1]));
        } else {
            g_string_append_printf (decoded, "[ERR] %s", str);
        }
        g_strfreev (parts);

    } else if (g_str_has_prefix (str, "+")) {
        // Extended response
        self->responses_decoded++;
        g_string_append (decoded, "[RSP] ");
        char **parts = g_strsplit (str, ":", 2);

        g_string_append (decoded, parts[0]); // Command name

        if (parts[1]) {
            g_string_append_printf (decoded, " → %s", g_strstrip (parts[1]));
        }
        g_strfreev (parts);

    } else if (g_strcmp0 (str, "RING") == 0) {
        self->responses_decoded++;
        g_string_append (decoded, "[IND] Incoming call");

    } else if (g_strcmp0 (str, "CONNECT") == 0) {
        self->responses_decoded++;
        g_string_append (decoded, "[IND] Connection established");

    } else if (g_strcmp0 (str, "NO CARRIER") == 0) {
        self->responses_decoded++;
        g_string_append (decoded, "[IND] No carrier detected");

    } else if (g_strcmp0 (str, "BUSY") == 0) {
        self->responses_decoded++;
        g_string_append (decoded, "[IND] Line busy");

    } else {
        // Unknown format
        g_string_append (decoded, "[???] ");
        g_string_append (decoded, str);
    }

    g_free (str);
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Validate AT command/response
 */
static gboolean at_decoder_validate (GtkTermProtocolDecoder *decoder,
                                      const guint8 *data,
                                      gsize length,
                                      char **error) {
    (void)decoder;

    if (length == 0) {
        if (error)
            *error = g_strdup ("Empty data");
        return FALSE;
    }

    // AT commands should be printable ASCII
    for (gsize i = 0; i < length; i++) {
        if (data[i] < 0x20 && data[i] != '\r' && data[i] != '\n') {
            if (error)
                *error = g_strdup_printf ("Non-printable character at position %zu", i);
            return FALSE;
        }
    }

    return TRUE;
}

/**
 * @brief Get decoder statistics
 */
static char *at_decoder_get_stats (GtkTermProtocolDecoder *decoder) {
    GtkTermATDecoder *self = GTKTERM_AT_DECODER (decoder);

    return g_strdup_printf ("Commands: %u, Responses: %u, Errors: %u",
                            self->commands_decoded,
                            self->responses_decoded,
                            self->errors_detected);
}

/**
 * @brief Reset decoder statistics
 */
static void at_decoder_reset (GtkTermProtocolDecoder *decoder) {
    GtkTermATDecoder *self = GTKTERM_AT_DECODER (decoder);

    self->commands_decoded = 0;
    self->responses_decoded = 0;
    self->errors_detected = 0;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_at_decoder_interface_init (GtkTermProtocolDecoderInterface *iface) {
    iface->get_name = at_decoder_get_name;
    iface->get_id = at_decoder_get_id;
    iface->detect = at_decoder_detect;
    iface->decode = at_decoder_decode;
    iface->validate = at_decoder_validate;
    iface->get_stats = at_decoder_get_stats;
    iface->reset = at_decoder_reset;
}

/**
 * @brief Class initialization
 */
static void gtkterm_at_decoder_class_init (GtkTermATDecoderClass *klass) {
    (void)klass;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_at_decoder_init (GtkTermATDecoder *self) {
    self->commands_decoded = 0;
    self->responses_decoded = 0;
    self->errors_detected = 0;
}

/**
 * @brief Create a new AT decoder
 */
GtkTermATDecoder *gtkterm_at_decoder_new (void) {
    return g_object_new (GTKTERM_TYPE_AT_DECODER, NULL);
}
