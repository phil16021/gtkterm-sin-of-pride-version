/************************************************************************/
/* gtkterm_statusbar.h                                                  */
/* -------------------                                                  */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Enhanced status bar with live statistics                       */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_STATUSBAR_H
#define GTKTERM_STATUSBAR_H

#include <gtk/gtk.h>

G_BEGIN_DECLS

#define GTKTERM_TYPE_STATUSBAR gtkterm_statusbar_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermStatusbar, gtkterm_statusbar, GTKTERM, STATUSBAR, GtkBox)

/**
 * @brief Status indicators
 */
typedef enum {
    GTKTERM_STATUS_DISCONNECTED,
    GTKTERM_STATUS_CONNECTING,
    GTKTERM_STATUS_CONNECTED,
    GTKTERM_STATUS_ERROR
} GtkTermStatusState;

/**
 * @brief Create a new statusbar
 */
GtkWidget *gtkterm_statusbar_new (void);

/**
 * @brief Set connection status
 */
void gtkterm_statusbar_set_status (GtkTermStatusbar *self,
                                    GtkTermStatusState state,
                                    const char *message);

/**
 * @brief Update port information
 */
void gtkterm_statusbar_set_port_info (GtkTermStatusbar *self,
                                       const char *port_name,
                                       guint32 baud_rate,
                                       const char *format);

/**
 * @brief Update statistics
 */
void gtkterm_statusbar_update_stats (GtkTermStatusbar *self,
                                      guint64 rx_bytes,
                                      guint64 tx_bytes,
                                      gdouble rx_rate,
                                      gdouble tx_rate);

/**
 * @brief Set signal states
 */
void gtkterm_statusbar_set_signals (GtkTermStatusbar *self,
                                     gboolean dtr,
                                     gboolean rts,
                                     gboolean cts,
                                     gboolean dsr,
                                     gboolean dcd);

/**
 * @brief Show temporary message
 */
void gtkterm_statusbar_show_message (GtkTermStatusbar *self,
                                      const char *message,
                                      guint timeout_ms);

/**
 * @brief Set protocol decoder status
 */
void gtkterm_statusbar_set_protocol (GtkTermStatusbar *self,
                                      const char *protocol_name);

/**
 * @brief Set recording status
 */
void gtkterm_statusbar_set_recording (GtkTermStatusbar *self,
                                       gboolean recording,
                                       const char *filename);

G_END_DECLS

#endif // GTKTERM_STATUSBAR_H
