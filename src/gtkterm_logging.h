/************************************************************************/
/* gtkterm_logging.h                                                    */
/* -----------------                                                    */
/*           GTKTerm Software                                           */
/*                      (c) Julien Schmitt                              */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Include file for GTKTerm logging functionality                 */
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

#ifndef GTKTERM_LOGGING_H
#define GTKTERM_LOGGING_H

#include <glib-object.h>
#include <gio/gio.h>
#include "gtkterm_data_chunk.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_LOGGING gtkterm_logging_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermLogging, gtkterm_logging, GTKTERM, LOGGING, GObject)

typedef enum {
    GTKTERM_LOGGING_STOPPED,
    GTKTERM_LOGGING_ACTIVE,
    GTKTERM_LOGGING_PAUSED
} GtkTermLoggingState;

typedef enum {
    GTKTERM_LOGGING_FORMAT_RAW,      /**< Raw binary data */
    GTKTERM_LOGGING_FORMAT_TEXT,     /**< Plain text with timestamps */
    GTKTERM_LOGGING_FORMAT_CSV,      /**< CSV format with metadata */
    GTKTERM_LOGGING_FORMAT_JSON,     /**< JSON format */
    GTKTERM_LOGGING_FORMAT_XML       /**< XML format */
} GtkTermLoggingFormat;

GtkTermLogging *gtkterm_logging_new (void);

/* Public API */
gboolean gtkterm_logging_start (GtkTermLogging *self, const char *filename, GError **error);
void gtkterm_logging_stop (GtkTermLogging *self);
void gtkterm_logging_pause (GtkTermLogging *self);
void gtkterm_logging_resume (GtkTermLogging *self);
void gtkterm_logging_clear (GtkTermLogging *self);
void gtkterm_logging_write (GtkTermLogging *self, const char *data, gsize length);
GtkTermLoggingState gtkterm_logging_get_state (GtkTermLogging *self);
const char *gtkterm_logging_get_filename (GtkTermLogging *self);

/* Enhanced session-based logging */
gboolean gtkterm_logging_start_session (GtkTermLogging *self,
                                        const char *directory,
                                        const char *port_name,
                                        const char *profile_name,
                                        GtkTermLoggingFormat format,
                                        GError **error);
void gtkterm_logging_write_chunk (GtkTermLogging *self,
                                  const GtkTermDataChunk *chunk,
                                  gint64 session_start_time);
void gtkterm_logging_write_metadata (GtkTermLogging *self,
                                     const char *key,
                                     const char *value);

/* Export functions */
gboolean gtkterm_logging_export_chunks (const char *filename,
                                        GPtrArray *chunks,
                                        GtkTermLoggingFormat format,
                                        gint64 session_start_time,
                                        GError **error);

G_END_DECLS

#endif // GTKTERM_LOGGING_H
