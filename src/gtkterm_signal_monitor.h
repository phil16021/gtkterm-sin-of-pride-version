/************************************************************************/
/* gtkterm_signal_monitor.h                                             */
/* ------------------------                                             */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Real-time signal line monitoring widget                        */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_SIGNAL_MONITOR_H
#define GTKTERM_SIGNAL_MONITOR_H

#include <gtk/gtk.h>
#include "gtkterm_port_control.h"

G_BEGIN_DECLS

#define GTKTERM_TYPE_SIGNAL_MONITOR gtkterm_signal_monitor_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermSignalMonitor, gtkterm_signal_monitor, GTKTERM, SIGNAL_MONITOR, GtkBox)

/**
 * @brief Signal change event
 */
typedef struct {
    guint32 signal_mask;
    guint32 state;
    gint64 timestamp;
} GtkTermSignalEvent;

/**
 * @brief Create a new signal monitor
 */
GtkWidget *gtkterm_signal_monitor_new (void);

/**
 * @brief Update signal states
 */
void gtkterm_signal_monitor_update (GtkTermSignalMonitor *self,
                                     guint32 signals);

/**
 * @brief Clear signal history
 */
void gtkterm_signal_monitor_clear_history (GtkTermSignalMonitor *self);

/**
 * @brief Set update interval (ms)
 */
void gtkterm_signal_monitor_set_interval (GtkTermSignalMonitor *self,
                                           guint interval_ms);

/**
 * @brief Set history duration (seconds)
 */
void gtkterm_signal_monitor_set_history_duration (GtkTermSignalMonitor *self,
                                                    guint duration_sec);

/**
 * @brief Get signal statistics
 */
guint gtkterm_signal_monitor_get_change_count (GtkTermSignalMonitor *self,
                                                guint32 signal_mask);

/**
 * @brief Export signal history to file
 */
gboolean gtkterm_signal_monitor_export (GtkTermSignalMonitor *self,
                                         const char *filename,
                                         GError **error);

G_END_DECLS

#endif // GTKTERM_SIGNAL_MONITOR_H
