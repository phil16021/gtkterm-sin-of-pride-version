/************************************************************************/
/* gtkterm_dataview.h                                                   */
/* ------------------                                                   */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Custom data view widget for multi-format display               */
/*      Supports ASCII, Hex, Binary, Decimal, and Mixed modes          */
/*                                                                      */
/* This GtkTerm is free software: you can redistribute it and/or modify*/
/* it under the terms of the GNU  General Public License as published  */
/* by the Free Software Foundation, either version 3 of the License,   */
/* or (at your option) any later version.                              */
/*                                                                      */
/* GtkTerm is distributed in the hope that it will be useful, but      */
/* WITHOUT ANY WARRANTY; without even the implied warranty of          */
/* MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.                */
/* See the GNU General Public License for more details.                */
/*                                                                      */
/* You should have received a copy of the GNU General Public License   */
/* along with GtkTerm If not, see <https://www.gnu.org/licenses/>.     */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_DATAVIEW_H
#define GTKTERM_DATAVIEW_H

#include <gtk/gtk.h>
#include "gtkterm_data_chunk.h"
#include "gtkterm_terminal.h"  // For GtkTermTerminalView enum

G_BEGIN_DECLS

#define GTKTERM_TYPE_DATAVIEW gtkterm_dataview_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermDataView, gtkterm_dataview, GTKTERM, DATAVIEW, GtkTextView)

/**
 * @brief Display options for the data view
 */
typedef struct {
    gboolean show_timestamps;       /**< Display timestamp column */
    gboolean show_direction;        /**< Display TX/RX indicator */
    gboolean show_index;            /**< Display byte index/offset */
    guint hex_bytes_per_line;       /**< Bytes per line in hex mode (8, 16, 32) */
    gboolean auto_scroll;           /**< Auto-scroll to bottom on new data */
    gint64 session_start_time;      /**< Session start for relative timestamps */
    const char *timestamp_format;   /**< Format: "absolute", "relative", "delta" */
} GtkTermDataViewOptions;

/* Widget creation */
GtkTermDataView *gtkterm_dataview_new (void);

/* View mode control */
void gtkterm_dataview_set_view_mode (GtkTermDataView *self, GtkTermTerminalView mode);
GtkTermTerminalView gtkterm_dataview_get_view_mode (GtkTermDataView *self);

/* Display options */
void gtkterm_dataview_set_options (GtkTermDataView *self, const GtkTermDataViewOptions *options);
void gtkterm_dataview_get_options (GtkTermDataView *self, GtkTermDataViewOptions *options);

void gtkterm_dataview_set_show_timestamps (GtkTermDataView *self, gboolean show);
gboolean gtkterm_dataview_get_show_timestamps (GtkTermDataView *self);

void gtkterm_dataview_set_hex_bytes_per_line (GtkTermDataView *self, guint bytes);
guint gtkterm_dataview_get_hex_bytes_per_line (GtkTermDataView *self);

/* Data management */
void gtkterm_dataview_append_chunk (GtkTermDataView *self, const GtkTermDataChunk *chunk);
void gtkterm_dataview_append_data (GtkTermDataView *self,
                                   const guint8 *data,
                                   gsize length,
                                   GtkTermDataDirection direction);
void gtkterm_dataview_clear (GtkTermDataView *self);

/* Search & filter */
void gtkterm_dataview_set_filter_direction (GtkTermDataView *self, GtkTermDataDirection direction);
void gtkterm_dataview_clear_filter (GtkTermDataView *self);

/* Utility */
void gtkterm_dataview_scroll_to_bottom (GtkTermDataView *self);
gchar *gtkterm_dataview_get_all_text (GtkTermDataView *self);

G_END_DECLS

#endif // GTKTERM_DATAVIEW_H
