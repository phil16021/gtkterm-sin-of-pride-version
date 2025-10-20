/************************************************************************/
/* gtkterm_nmea_decoder.c                                               */
/* ----------------------                                               */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      NMEA 0183 protocol decoder implementation                      */
/*                                                                      */
/************************************************************************/

#include "gtkterm_nmea_decoder.h"
#include <string.h>
#include <stdio.h>
#include <ctype.h>
#include <math.h>

struct _GtkTermNMEADecoder {
    GObject parent_instance;
    guint sentences_decoded;
    guint checksum_errors;
    guint parse_errors;
};

static void gtkterm_nmea_decoder_interface_init (GtkTermProtocolDecoderInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermNMEADecoder, gtkterm_nmea_decoder, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_PROTOCOL_DECODER,
                                                gtkterm_nmea_decoder_interface_init))

/**
 * @brief NMEA sentence types and descriptions
 */
typedef struct {
    const char *sentence_id;
    const char *description;
} NMEASentenceInfo;

static const NMEASentenceInfo nmea_sentences[] = {
    {"GGA", "Global Positioning System Fix Data"},
    {"RMC", "Recommended Minimum Navigation Information"},
    {"GSV", "Satellites in View"},
    {"GSA", "GPS DOP and active satellites"},
    {"GLL", "Geographic Position - Latitude/Longitude"},
    {"VTG", "Track made good and Ground speed"},
    {"GBS", "GPS Satellite Fault Detection"},
    {"GST", "GPS Pseudorange Noise Statistics"},
    {"ZDA", "Time & Date"},
    {"HDT", "Heading - True"},
    {"HDM", "Heading - Magnetic"},
    {"DBT", "Depth Below Transducer"},
    {"MTW", "Mean Temperature of Water"},
    {"VHW", "Water speed and heading"},
    {"VLW", "Distance Traveled through Water"},
    {NULL, NULL}
};

/**
 * @brief NMEA talker IDs
 */
static const char *get_talker_name (const char *talker_id) {
    if (strcmp (talker_id, "GP") == 0) return "GPS";
    if (strcmp (talker_id, "GL") == 0) return "GLONASS";
    if (strcmp (talker_id, "GA") == 0) return "Galileo";
    if (strcmp (talker_id, "GN") == 0) return "Multi-GNSS";
    if (strcmp (talker_id, "BD") == 0) return "BeiDou";
    if (strcmp (talker_id, "GB") == 0) return "BeiDou";
    if (strcmp (talker_id, "QZ") == 0) return "QZSS";
    if (strcmp (talker_id, "II") == 0) return "Integrated Instrumentation";
    if (strcmp (talker_id, "EC") == 0) return "Electronic Chart";
    if (strcmp (talker_id, "SD") == 0) return "Sounder (depth)";
    if (strcmp (talker_id, "YX") == 0) return "Transducer";
    return "Unknown";
}

/**
 * @brief Get sentence description
 */
static const char *get_sentence_description (const char *sentence_id) {
    for (int i = 0; nmea_sentences[i].sentence_id != NULL; i++) {
        if (strcmp (nmea_sentences[i].sentence_id, sentence_id) == 0) {
            return nmea_sentences[i].description;
        }
    }
    return "Unknown sentence type";
}

/**
 * @brief Calculate NMEA checksum
 */
static guint8 calculate_checksum (const char *sentence) {
    guint8 checksum = 0;
    // Calculate XOR from after $ to before *
    const char *start = strchr (sentence, '$');
    const char *end = strchr (sentence, '*');

    if (!start || !end || end <= start)
        return 0;

    for (const char *p = start + 1; p < end; p++) {
        checksum ^= *p;
    }

    return checksum;
}

/**
 * @brief Convert NMEA coordinate to decimal degrees
 */
static gdouble nmea_coord_to_decimal (const char *coord, const char *direction) {
    if (!coord || strlen (coord) == 0)
        return 0.0;

    // Format: DDMM.MMMM or DDDMM.MMMM
    gdouble value = g_strtod (coord, NULL);

    // Determine if latitude (DD) or longitude (DDD)
    gint degrees_digits = (strchr (coord, '.') - coord <= 4) ? 2 : 3;

    gint degrees = (gint)(value / pow (10, strlen (coord) - degrees_digits - strcspn (coord, ".")));
    gdouble minutes = value - (degrees * pow (10, strlen (coord) - degrees_digits - strcspn (coord, ".")));

    gdouble decimal = degrees + (minutes / 60.0);

    // Apply direction
    if (direction && (direction[0] == 'S' || direction[0] == 'W'))
        decimal = -decimal;

    return decimal;
}

/**
 * @brief Get protocol name
 */
static const char *nmea_decoder_get_name (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "NMEA 0183";
}

/**
 * @brief Get protocol ID
 */
static const char *nmea_decoder_get_id (GtkTermProtocolDecoder *decoder) {
    (void)decoder;
    return "nmea";
}

/**
 * @brief Detect NMEA protocol
 */
static gdouble nmea_decoder_detect (GtkTermProtocolDecoder *decoder,
                                     const guint8 *data,
                                     gsize length) {
    (void)decoder;

    if (length < 10) // Minimum: $GPGGA*XX
        return 0.0;

    // Must start with $
    if (data[0] != '$')
        return 0.0;

    // Look for checksum separator *
    const char *str = (const char *)data;
    const char *star = strchr (str, '*');

    if (!star)
        return 0.3; // Might be NMEA without checksum

    // Check if checksum is hex
    if (star - str < 3 || star - str > length - 3)
        return 0.0;

    if (!isxdigit (star[1]) || !isxdigit (star[2]))
        return 0.0;

    // Check for valid talker ID (2 uppercase letters after $)
    if (!isupper (data[1]) || !isupper (data[2]))
        return 0.5;

    // Check for valid sentence ID (3 uppercase letters)
    if (!isupper (data[3]) || !isupper (data[4]) || !isupper (data[5]))
        return 0.5;

    return 0.95;
}

/**
 * @brief Parse GGA sentence (Fix Data)
 */
static void parse_gga (GString *output, char **fields, gint field_count) {
    if (field_count < 10)
        return;

    // fields[1] = UTC time
    // fields[2] = Latitude
    // fields[3] = N/S
    // fields[4] = Longitude
    // fields[5] = E/W
    // fields[6] = Fix quality
    // fields[7] = Number of satellites
    // fields[8] = HDOP
    // fields[9] = Altitude

    const char *fix_quality[] = {"Invalid", "GPS fix", "DGPS fix", "PPS fix",
                                  "RTK", "Float RTK", "Estimated", "Manual", "Simulation"};

    g_string_append (output, " | ");

    if (strlen (fields[1]) >= 6) {
        g_string_append_printf (output, "Time:%c%c:%c%c:%c%c ",
                                fields[1][0], fields[1][1],
                                fields[1][2], fields[1][3],
                                fields[1][4], fields[1][5]);
    }

    if (strlen (fields[2]) > 0 && strlen (fields[4]) > 0) {
        gdouble lat = nmea_coord_to_decimal (fields[2], fields[3]);
        gdouble lon = nmea_coord_to_decimal (fields[4], fields[5]);
        g_string_append_printf (output, "Pos:%.6f°%s,%.6f°%s ",
                                fabs (lat), lat >= 0 ? "N" : "S",
                                fabs (lon), lon >= 0 ? "E" : "W");
    }

    if (strlen (fields[6]) > 0) {
        gint quality = atoi (fields[6]);
        if (quality >= 0 && quality < 9)
            g_string_append_printf (output, "Fix:%s ", fix_quality[quality]);
    }

    if (strlen (fields[7]) > 0) {
        g_string_append_printf (output, "Sats:%s ", fields[7]);
    }

    if (strlen (fields[9]) > 0) {
        g_string_append_printf (output, "Alt:%sm ", fields[9]);
    }
}

/**
 * @brief Parse RMC sentence (Recommended Minimum)
 */
static void parse_rmc (GString *output, char **fields, gint field_count) {
    if (field_count < 10)
        return;

    // fields[1] = UTC time
    // fields[2] = Status (A=active, V=void)
    // fields[3] = Latitude
    // fields[4] = N/S
    // fields[5] = Longitude
    // fields[6] = E/W
    // fields[7] = Speed over ground (knots)
    // fields[8] = Track angle (degrees)
    // fields[9] = Date

    g_string_append (output, " | ");

    if (strlen (fields[2]) > 0) {
        g_string_append_printf (output, "Status:%s ",
                                fields[2][0] == 'A' ? "Active" : "Void");
    }

    if (strlen (fields[1]) >= 6) {
        g_string_append_printf (output, "Time:%c%c:%c%c:%c%c ",
                                fields[1][0], fields[1][1],
                                fields[1][2], fields[1][3],
                                fields[1][4], fields[1][5]);
    }

    if (strlen (fields[3]) > 0 && strlen (fields[5]) > 0) {
        gdouble lat = nmea_coord_to_decimal (fields[3], fields[4]);
        gdouble lon = nmea_coord_to_decimal (fields[5], fields[6]);
        g_string_append_printf (output, "Pos:%.6f°,%.6f° ", lat, lon);
    }

    if (strlen (fields[7]) > 0) {
        g_string_append_printf (output, "Speed:%.1fkt ", g_strtod (fields[7], NULL));
    }

    if (strlen (fields[8]) > 0) {
        g_string_append_printf (output, "Course:%.1f° ", g_strtod (fields[8], NULL));
    }
}

/**
 * @brief Parse GSV sentence (Satellites in View)
 */
static void parse_gsv (GString *output, char **fields, gint field_count) {
    if (field_count < 4)
        return;

    // fields[1] = Total messages
    // fields[2] = Message number
    // fields[3] = Satellites in view

    g_string_append (output, " | ");

    g_string_append_printf (output, "Msg:%s/%s Sats:%s ",
                            fields[2], fields[1], fields[3]);

    // Each satellite: PRN, Elevation, Azimuth, SNR (4 satellites per message)
    gint sat_count = (field_count - 4) / 4;
    if (sat_count > 0) {
        g_string_append (output, "Data:[");
        for (int i = 0; i < sat_count && (4 + i*4 + 3) < field_count; i++) {
            if (i > 0) g_string_append (output, ", ");
            g_string_append_printf (output, "PRN:%s", fields[4 + i*4]);
            if (strlen (fields[4 + i*4 + 3]) > 0)
                g_string_append_printf (output, " SNR:%s", fields[4 + i*4 + 3]);
        }
        g_string_append (output, "]");
    }
}

/**
 * @brief Decode NMEA sentence
 */
static char *nmea_decoder_decode (GtkTermProtocolDecoder *decoder,
                                   const GtkTermDataChunk *chunk) {
    GtkTermNMEADecoder *self = GTKTERM_NMEA_DECODER (decoder);
    GString *decoded = g_string_new (NULL);

    if (!chunk || !chunk->data || chunk->length == 0)
        return g_string_free (decoded, FALSE);

    // Create null-terminated string
    char *sentence = g_strndup ((const char *)chunk->data, chunk->length);
    g_strstrip (sentence);

    // BUG FIX #4: Validate NMEA 0183 maximum sentence length (82 characters)
    // NMEA 0183 Section 5.3: "The maximum number of characters shall be 82"
    gsize sentence_len = strlen (sentence);
    if (sentence_len > 82) {
        g_string_printf (decoded, "[NMEA] Invalid sentence (length %zu exceeds 82 char limit)", sentence_len);
        self->parse_errors++;
        g_free (sentence);
        return g_string_free (decoded, FALSE);
    }

    if (sentence[0] != '$') {
        g_string_printf (decoded, "[NMEA] Invalid sentence (missing $)");
        self->parse_errors++;
        g_free (sentence);
        return g_string_free (decoded, FALSE);
    }

    // Extract talker ID and sentence type
    char talker_id[3] = {sentence[1], sentence[2], '\0'};
    char sentence_id[4] = {sentence[3], sentence[4], sentence[5], '\0'};

    const char *talker_name = get_talker_name (talker_id);
    const char *sentence_desc = get_sentence_description (sentence_id);

    g_string_printf (decoded, "[NMEA] %s-%s (%s: %s)",
                     talker_id, sentence_id, talker_name, sentence_desc);

    // Verify checksum
    const char *star = strchr (sentence, '*');
    if (star && strlen (star) >= 3) {
        guint8 received_checksum = 0;
        sscanf (star + 1, "%02hhx", &received_checksum);

        guint8 calc_checksum = calculate_checksum (sentence);

        if (received_checksum == calc_checksum) {
            g_string_append (decoded, " [CHK:OK]");
        } else {
            g_string_append_printf (decoded, " [CHK:FAIL rx:%02X calc:%02X]",
                                    received_checksum, calc_checksum);
            self->checksum_errors++;
        }
    }

    // Parse sentence fields
    char *data_part = strchr (sentence, ',');
    if (data_part) {
        // Extract everything between first , and *
        char *end = strchr (sentence, '*');
        char *fields_str = NULL;

        if (end) {
            fields_str = g_strndup (data_part, end - data_part);
        } else {
            fields_str = g_strdup (data_part);
        }

        char **fields = g_strsplit (fields_str, ",", -1);
        gint field_count = g_strv_length (fields);

        // Parse specific sentence types
        if (strcmp (sentence_id, "GGA") == 0) {
            parse_gga (decoded, fields, field_count);
        } else if (strcmp (sentence_id, "RMC") == 0) {
            parse_rmc (decoded, fields, field_count);
        } else if (strcmp (sentence_id, "GSV") == 0) {
            parse_gsv (decoded, fields, field_count);
        }

        g_strfreev (fields);
        g_free (fields_str);
    }

    self->sentences_decoded++;
    g_free (sentence);
    return g_string_free (decoded, FALSE);
}

/**
 * @brief Validate NMEA sentence
 */
static gboolean nmea_decoder_validate (GtkTermProtocolDecoder *decoder,
                                         const guint8 *data,
                                         gsize length,
                                         char **error) {
    (void)decoder;

    if (length < 10) {
        if (error) *error = g_strdup ("Sentence too short (min 10 bytes)");
        return FALSE;
    }

    if (data[0] != '$') {
        if (error) *error = g_strdup ("Missing start character '$'");
        return FALSE;
    }

    // Check for printable ASCII
    for (gsize i = 0; i < length; i++) {
        if (data[i] < 0x20 && data[i] != '\r' && data[i] != '\n') {
            if (error) *error = g_strdup_printf ("Non-printable character at position %zu", i);
            return FALSE;
        }
    }

    return TRUE;
}

/**
 * @brief Get decoder statistics
 */
static char *nmea_decoder_get_stats (GtkTermProtocolDecoder *decoder) {
    GtkTermNMEADecoder *self = GTKTERM_NMEA_DECODER (decoder);

    return g_strdup_printf ("Sentences: %u, Checksum Errors: %u, Parse Errors: %u",
                            self->sentences_decoded,
                            self->checksum_errors,
                            self->parse_errors);
}

/**
 * @brief Reset decoder statistics
 */
static void nmea_decoder_reset (GtkTermProtocolDecoder *decoder) {
    GtkTermNMEADecoder *self = GTKTERM_NMEA_DECODER (decoder);

    self->sentences_decoded = 0;
    self->checksum_errors = 0;
    self->parse_errors = 0;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_nmea_decoder_interface_init (GtkTermProtocolDecoderInterface *iface) {
    iface->get_name = nmea_decoder_get_name;
    iface->get_id = nmea_decoder_get_id;
    iface->detect = nmea_decoder_detect;
    iface->decode = nmea_decoder_decode;
    iface->validate = nmea_decoder_validate;
    iface->get_stats = nmea_decoder_get_stats;
    iface->reset = nmea_decoder_reset;
}

/**
 * @brief Class initialization
 */
static void gtkterm_nmea_decoder_class_init (GtkTermNMEADecoderClass *klass) {
    (void)klass;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_nmea_decoder_init (GtkTermNMEADecoder *self) {
    self->sentences_decoded = 0;
    self->checksum_errors = 0;
    self->parse_errors = 0;
}

/**
 * @brief Create a new NMEA decoder
 */
GtkTermNMEADecoder *gtkterm_nmea_decoder_new (void) {
    return g_object_new (GTKTERM_TYPE_NMEA_DECODER, NULL);
}
