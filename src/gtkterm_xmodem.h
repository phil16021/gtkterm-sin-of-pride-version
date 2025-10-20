/************************************************************************/
/* gtkterm_xmodem.h                                                     */
/* ----------------                                                     */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      XMODEM/XMODEM-1K/XMODEM-CRC file transfer protocol             */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_XMODEM_H
#define GTKTERM_XMODEM_H

#include <glib-object.h>
#include "gtkterm_file_transfer.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_XMODEM gtkterm_xmodem_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermXModem, gtkterm_xmodem, GTKTERM, XMODEM, GObject)

/**
 * @brief XMODEM variant
 */
typedef enum {
    GTKTERM_XMODEM_STANDARD,   /**< Standard XMODEM with checksum */
    GTKTERM_XMODEM_CRC,        /**< XMODEM with CRC */
    GTKTERM_XMODEM_1K          /**< XMODEM-1K (1024 byte blocks) */
} GtkTermXModemVariant;

/**
 * @brief Create a new XMODEM transfer object
 */
GtkTermXModem *gtkterm_xmodem_new (GtkTermXModemVariant variant);

/**
 * @brief Set the serial port write function
 */
void gtkterm_xmodem_set_write_func (GtkTermXModem *self,
                                     gboolean (*write_func)(const guint8 *, gsize, gpointer),
                                     gpointer user_data);

G_END_DECLS

#endif // GTKTERM_XMODEM_H
