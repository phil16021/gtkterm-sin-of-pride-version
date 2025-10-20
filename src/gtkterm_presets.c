/************************************************************************/
/* gtkterm_presets.c                                                    */
/* -----------------                                                    */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Configuration presets management implementation                */
/*                                                                      */
/************************************************************************/

#include "gtkterm_presets.h"
#include <json-glib/json-glib.h>
#include <string.h>

struct _GtkTermPresets {
    GObject parent_instance;

    GHashTable *presets;  /* name -> GtkTermPreset */
};

G_DEFINE_TYPE (GtkTermPresets, gtkterm_presets, G_TYPE_OBJECT)

/**
 * @brief Free preset
 */
void gtkterm_preset_free (GtkTermPreset *preset) {
    if (preset) {
        g_free (preset->name);
        g_free (preset->description);
        g_free (preset);
    }
}

/**
 * @brief Clone preset
 */
static GtkTermPreset *gtkterm_preset_clone (const GtkTermPreset *preset) {
    GtkTermPreset *clone = g_new0 (GtkTermPreset, 1);
    clone->name = g_strdup (preset->name);
    clone->description = g_strdup (preset->description);
    memcpy (&clone->config, &preset->config, sizeof (GtkTermPortConfig));
    return clone;
}

/**
 * @brief Add preset
 */
gboolean gtkterm_presets_add (GtkTermPresets *self,
                               const char *name,
                               const char *description,
                               const GtkTermPortConfig *config,
                               GError **error) {
    g_return_val_if_fail (GTKTERM_IS_PRESETS (self), FALSE);
    g_return_val_if_fail (name != NULL && strlen (name) > 0, FALSE);
    g_return_val_if_fail (config != NULL, FALSE);

    GtkTermPreset *preset = g_new0 (GtkTermPreset, 1);
    preset->name = g_strdup (name);
    preset->description = g_strdup (description ? description : "");
    memcpy (&preset->config, config, sizeof (GtkTermPortConfig));

    g_hash_table_insert (self->presets, g_strdup (name), preset);

    return TRUE;
}

/**
 * @brief Remove preset
 */
gboolean gtkterm_presets_remove (GtkTermPresets *self,
                                  const char *name,
                                  GError **error) {
    g_return_val_if_fail (GTKTERM_IS_PRESETS (self), FALSE);
    g_return_val_if_fail (name != NULL, FALSE);

    if (!g_hash_table_remove (self->presets, name)) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                    "Preset '%s' not found", name);
        return FALSE;
    }

    return TRUE;
}

/**
 * @brief Get preset by name
 */
const GtkTermPreset *gtkterm_presets_get (GtkTermPresets *self,
                                           const char *name) {
    g_return_val_if_fail (GTKTERM_IS_PRESETS (self), NULL);
    g_return_val_if_fail (name != NULL, NULL);

    return g_hash_table_lookup (self->presets, name);
}

/**
 * @brief Get all presets
 */
GList *gtkterm_presets_get_all (GtkTermPresets *self) {
    g_return_val_if_fail (GTKTERM_IS_PRESETS (self), NULL);

    return g_hash_table_get_values (self->presets);
}

/**
 * @brief Get preset names
 */
char **gtkterm_presets_get_names (GtkTermPresets *self) {
    g_return_val_if_fail (GTKTERM_IS_PRESETS (self), NULL);

    GList *keys = g_hash_table_get_keys (self->presets);
    guint count = g_list_length (keys);

    char **names = g_new0 (char *, count + 1);

    GList *l;
    int i = 0;
    for (l = keys; l != NULL; l = l->next) {
        names[i++] = g_strdup ((const char *) l->data);
    }

    g_list_free (keys);

    return names;
}

/**
 * @brief Convert parity enum to string
 */
static const char *parity_to_string (GtkTermParity parity) {
    switch (parity) {
        case GTKTERM_PARITY_NONE: return "none";
        case GTKTERM_PARITY_ODD: return "odd";
        case GTKTERM_PARITY_EVEN: return "even";
        case GTKTERM_PARITY_MARK: return "mark";
        case GTKTERM_PARITY_SPACE: return "space";
        default: return "none";
    }
}

/**
 * @brief Convert string to parity enum
 */
static GtkTermParity string_to_parity (const char *str) {
    if (g_strcmp0 (str, "odd") == 0) return GTKTERM_PARITY_ODD;
    if (g_strcmp0 (str, "even") == 0) return GTKTERM_PARITY_EVEN;
    if (g_strcmp0 (str, "mark") == 0) return GTKTERM_PARITY_MARK;
    if (g_strcmp0 (str, "space") == 0) return GTKTERM_PARITY_SPACE;
    return GTKTERM_PARITY_NONE;
}

/**
 * @brief Convert stop bits enum to string
 */
static const char *stop_bits_to_string (GtkTermStopBits stop_bits) {
    switch (stop_bits) {
        case GTKTERM_STOP_BITS_1: return "1";
        case GTKTERM_STOP_BITS_1_5: return "1.5";
        case GTKTERM_STOP_BITS_2: return "2";
        default: return "1";
    }
}

/**
 * @brief Convert string to stop bits enum
 */
static GtkTermStopBits string_to_stop_bits (const char *str) {
    if (g_strcmp0 (str, "1.5") == 0) return GTKTERM_STOP_BITS_1_5;
    if (g_strcmp0 (str, "2") == 0) return GTKTERM_STOP_BITS_2;
    return GTKTERM_STOP_BITS_1;
}

/**
 * @brief Convert flow control enum to string
 */
static const char *flow_control_to_string (GtkTermFlowControl flow_control) {
    switch (flow_control) {
        case GTKTERM_FLOW_CONTROL_NONE: return "none";
        case GTKTERM_FLOW_CONTROL_XONXOFF: return "xonxoff";
        case GTKTERM_FLOW_CONTROL_RTSCTS: return "rtscts";
        case GTKTERM_FLOW_CONTROL_DTRDSR: return "dtrdsr";
        default: return "none";
    }
}

/**
 * @brief Convert string to flow control enum
 */
static GtkTermFlowControl string_to_flow_control (const char *str) {
    if (g_strcmp0 (str, "xonxoff") == 0) return GTKTERM_FLOW_CONTROL_XONXOFF;
    if (g_strcmp0 (str, "rtscts") == 0) return GTKTERM_FLOW_CONTROL_RTSCTS;
    if (g_strcmp0 (str, "dtrdsr") == 0) return GTKTERM_FLOW_CONTROL_DTRDSR;
    return GTKTERM_FLOW_CONTROL_NONE;
}

/**
 * @brief Save presets to file
 */
gboolean gtkterm_presets_save (GtkTermPresets *self,
                                const char *filename,
                                GError **error) {
    g_return_val_if_fail (GTKTERM_IS_PRESETS (self), FALSE);
    g_return_val_if_fail (filename != NULL, FALSE);

    JsonBuilder *builder = json_builder_new ();
    json_builder_begin_object (builder);
    json_builder_set_member_name (builder, "presets");
    json_builder_begin_array (builder);

    GList *presets = g_hash_table_get_values (self->presets);
    for (GList *l = presets; l != NULL; l = l->next) {
        GtkTermPreset *preset = (GtkTermPreset *) l->data;

        json_builder_begin_object (builder);

        json_builder_set_member_name (builder, "name");
        json_builder_add_string_value (builder, preset->name);

        json_builder_set_member_name (builder, "description");
        json_builder_add_string_value (builder, preset->description);

        json_builder_set_member_name (builder, "config");
        json_builder_begin_object (builder);

        json_builder_set_member_name (builder, "baud_rate");
        json_builder_add_int_value (builder, preset->config.baud_rate);

        json_builder_set_member_name (builder, "data_bits");
        json_builder_add_int_value (builder, preset->config.data_bits);

        json_builder_set_member_name (builder, "parity");
        json_builder_add_string_value (builder, parity_to_string (preset->config.parity));

        json_builder_set_member_name (builder, "stop_bits");
        json_builder_add_string_value (builder, stop_bits_to_string (preset->config.stop_bits));

        json_builder_set_member_name (builder, "flow_control");
        json_builder_add_string_value (builder, flow_control_to_string (preset->config.flow_control));

        json_builder_end_object (builder);
        json_builder_end_object (builder);
    }

    json_builder_end_array (builder);
    json_builder_end_object (builder);

    g_list_free (presets);

    JsonGenerator *generator = json_generator_new ();
    JsonNode *root = json_builder_get_root (builder);
    json_generator_set_root (generator, root);
    json_generator_set_pretty (generator, TRUE);

    gboolean success = json_generator_to_file (generator, filename, error);

    json_node_free (root);
    g_object_unref (generator);
    g_object_unref (builder);

    return success;
}

/**
 * @brief Load presets from file
 */
gboolean gtkterm_presets_load (GtkTermPresets *self,
                                const char *filename,
                                GError **error) {
    g_return_val_if_fail (GTKTERM_IS_PRESETS (self), FALSE);
    g_return_val_if_fail (filename != NULL, FALSE);

    JsonParser *parser = json_parser_new ();

    if (!json_parser_load_from_file (parser, filename, error)) {
        g_object_unref (parser);
        return FALSE;
    }

    JsonNode *root = json_parser_get_root (parser);
    JsonObject *root_obj = json_node_get_object (root);
    JsonArray *presets_array = json_object_get_array_member (root_obj, "presets");

    guint length = json_array_get_length (presets_array);
    for (guint i = 0; i < length; i++) {
        JsonObject *preset_obj = json_array_get_object_element (presets_array, i);

        GtkTermPreset *preset = g_new0 (GtkTermPreset, 1);

        preset->name = g_strdup (json_object_get_string_member (preset_obj, "name"));
        preset->description = g_strdup (json_object_get_string_member (preset_obj, "description"));

        JsonObject *config_obj = json_object_get_object_member (preset_obj, "config");

        preset->config.baud_rate = json_object_get_int_member (config_obj, "baud_rate");
        preset->config.data_bits = json_object_get_int_member (config_obj, "data_bits");
        preset->config.parity = string_to_parity (json_object_get_string_member (config_obj, "parity"));
        preset->config.stop_bits = string_to_stop_bits (json_object_get_string_member (config_obj, "stop_bits"));
        preset->config.flow_control = string_to_flow_control (json_object_get_string_member (config_obj, "flow_control"));

        g_hash_table_insert (self->presets, g_strdup (preset->name), preset);
    }

    g_object_unref (parser);

    return TRUE;
}

/**
 * @brief Load default presets
 */
void gtkterm_presets_load_defaults (GtkTermPresets *self) {
    g_return_if_fail (GTKTERM_IS_PRESETS (self));

    // Arduino (Standard)
    GtkTermPortConfig arduino_config;
    gtkterm_port_config_init (&arduino_config);
    arduino_config.baud_rate = 115200;
    arduino_config.data_bits = 8;
    arduino_config.parity = GTKTERM_PARITY_NONE;
    arduino_config.stop_bits = GTKTERM_STOP_BITS_1;
    arduino_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "Arduino", "Standard Arduino configuration (115200 8N1)", &arduino_config, NULL);

    // ESP32/ESP8266
    GtkTermPortConfig esp_config;
    gtkterm_port_config_init (&esp_config);
    esp_config.baud_rate = 115200;
    esp_config.data_bits = 8;
    esp_config.parity = GTKTERM_PARITY_NONE;
    esp_config.stop_bits = GTKTERM_STOP_BITS_1;
    esp_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "ESP32/ESP8266", "Espressif ESP boards (115200 8N1)", &esp_config, NULL);

    // GPS Module (Standard)
    GtkTermPortConfig gps_config;
    gtkterm_port_config_init (&gps_config);
    gps_config.baud_rate = 9600;
    gps_config.data_bits = 8;
    gps_config.parity = GTKTERM_PARITY_NONE;
    gps_config.stop_bits = GTKTERM_STOP_BITS_1;
    gps_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "GPS Module", "Standard GPS module (9600 8N1)", &gps_config, NULL);

    // HC-05 Bluetooth
    GtkTermPortConfig bluetooth_config;
    gtkterm_port_config_init (&bluetooth_config);
    bluetooth_config.baud_rate = 9600;
    bluetooth_config.data_bits = 8;
    bluetooth_config.parity = GTKTERM_PARITY_NONE;
    bluetooth_config.stop_bits = GTKTERM_STOP_BITS_1;
    bluetooth_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "HC-05 Bluetooth", "HC-05 Bluetooth module (9600 8N1)", &bluetooth_config, NULL);

    // Modbus RTU
    GtkTermPortConfig modbus_config;
    gtkterm_port_config_init (&modbus_config);
    modbus_config.baud_rate = 19200;
    modbus_config.data_bits = 8;
    modbus_config.parity = GTKTERM_PARITY_EVEN;
    modbus_config.stop_bits = GTKTERM_STOP_BITS_1;
    modbus_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "Modbus RTU", "Industrial Modbus protocol (19200 8E1)", &modbus_config, NULL);

    // Old Modem
    GtkTermPortConfig modem_config;
    gtkterm_port_config_init (&modem_config);
    modem_config.baud_rate = 9600;
    modem_config.data_bits = 8;
    modem_config.parity = GTKTERM_PARITY_NONE;
    modem_config.stop_bits = GTKTERM_STOP_BITS_1;
    modem_config.flow_control = GTKTERM_FLOW_CONTROL_RTSCTS;
    gtkterm_presets_add (self, "Modem (9600)", "Classic modem with hardware flow control", &modem_config, NULL);

    // Raspberry Pi
    GtkTermPortConfig raspberry_config;
    gtkterm_port_config_init (&raspberry_config);
    raspberry_config.baud_rate = 115200;
    raspberry_config.data_bits = 8;
    raspberry_config.parity = GTKTERM_PARITY_NONE;
    raspberry_config.stop_bits = GTKTERM_STOP_BITS_1;
    raspberry_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "Raspberry Pi", "Raspberry Pi serial console (115200 8N1)", &raspberry_config, NULL);

    // STM32 UART
    GtkTermPortConfig stm32_config;
    gtkterm_port_config_init (&stm32_config);
    stm32_config.baud_rate = 115200;
    stm32_config.data_bits = 8;
    stm32_config.parity = GTKTERM_PARITY_NONE;
    stm32_config.stop_bits = GTKTERM_STOP_BITS_1;
    stm32_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "STM32", "STM32 microcontroller (115200 8N1)", &stm32_config, NULL);

    // Legacy System (2400 7E1)
    GtkTermPortConfig legacy_config;
    gtkterm_port_config_init (&legacy_config);
    legacy_config.baud_rate = 2400;
    legacy_config.data_bits = 7;
    legacy_config.parity = GTKTERM_PARITY_EVEN;
    legacy_config.stop_bits = GTKTERM_STOP_BITS_1;
    legacy_config.flow_control = GTKTERM_FLOW_CONTROL_NONE;
    gtkterm_presets_add (self, "Legacy System", "Old industrial equipment (2400 7E1)", &legacy_config, NULL);

    // High Speed
    GtkTermPortConfig highspeed_config;
    gtkterm_port_config_init (&highspeed_config);
    highspeed_config.baud_rate = 921600;
    highspeed_config.data_bits = 8;
    highspeed_config.parity = GTKTERM_PARITY_NONE;
    highspeed_config.stop_bits = GTKTERM_STOP_BITS_1;
    highspeed_config.flow_control = GTKTERM_FLOW_CONTROL_RTSCTS;
    gtkterm_presets_add (self, "High Speed", "High-speed debugging (921600 8N1 RTS/CTS)", &highspeed_config, NULL);
}

/**
 * @brief Dispose
 */
static void gtkterm_presets_dispose (GObject *object) {
    GtkTermPresets *self = GTKTERM_PRESETS (object);

    g_clear_pointer (&self->presets, g_hash_table_destroy);

    G_OBJECT_CLASS (gtkterm_presets_parent_class)->dispose (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_presets_class_init (GtkTermPresetsClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_presets_dispose;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_presets_init (GtkTermPresets *self) {
    self->presets = g_hash_table_new_full (g_str_hash, g_str_equal,
                                            g_free,
                                            (GDestroyNotify) gtkterm_preset_free);
}

/**
 * @brief Create presets manager
 */
GtkTermPresets *gtkterm_presets_new (void) {
    GtkTermPresets *self = g_object_new (GTKTERM_TYPE_PRESETS, NULL);

    // Load default presets
    gtkterm_presets_load_defaults (self);

    return self;
}
