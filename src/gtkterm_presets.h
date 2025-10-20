/************************************************************************/
/* gtkterm_presets.h                                                    */
/* -----------------                                                    */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Configuration presets management                               */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_PRESETS_H
#define GTKTERM_PRESETS_H

#include <gtk/gtk.h>
#include "gtkterm_port_control.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_PRESETS gtkterm_presets_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermPresets, gtkterm_presets, GTKTERM, PRESETS, GObject)

/**
 * @brief Configuration preset
 */
typedef struct {
    char *name;
    char *description;
    GtkTermPortConfig config;
} GtkTermPreset;

/**
 * @brief Create presets manager
 */
GtkTermPresets *gtkterm_presets_new (void);

/**
 * @brief Add preset
 */
gboolean gtkterm_presets_add (GtkTermPresets *self,
                               const char *name,
                               const char *description,
                               const GtkTermPortConfig *config,
                               GError **error);

/**
 * @brief Remove preset
 */
gboolean gtkterm_presets_remove (GtkTermPresets *self,
                                  const char *name,
                                  GError **error);

/**
 * @brief Get preset by name
 */
const GtkTermPreset *gtkterm_presets_get (GtkTermPresets *self,
                                           const char *name);

/**
 * @brief Get all presets
 */
GList *gtkterm_presets_get_all (GtkTermPresets *self);

/**
 * @brief Get preset names
 */
char **gtkterm_presets_get_names (GtkTermPresets *self);

/**
 * @brief Load presets from file
 */
gboolean gtkterm_presets_load (GtkTermPresets *self,
                                const char *filename,
                                GError **error);

/**
 * @brief Save presets to file
 */
gboolean gtkterm_presets_save (GtkTermPresets *self,
                                const char *filename,
                                GError **error);

/**
 * @brief Load default presets
 */
void gtkterm_presets_load_defaults (GtkTermPresets *self);

/**
 * @brief Free preset
 */
void gtkterm_preset_free (GtkTermPreset *preset);

G_END_DECLS

#endif // GTKTERM_PRESETS_H
