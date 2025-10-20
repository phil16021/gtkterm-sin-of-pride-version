/************************************************************************/
/* gtkterm_port_control.h                                               */
/* ----------------------                                               */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      Advanced port control and monitoring                           */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_PORT_CONTROL_H
#define GTKTERM_PORT_CONTROL_H

#include <glib-object.h>

G_BEGIN_DECLS

/**
 * @brief Flow control modes
 */
typedef enum {
    GTKTERM_FLOW_CONTROL_NONE,      /**< No flow control */
    GTKTERM_FLOW_CONTROL_XONXOFF,   /**< Software flow control (XON/XOFF) */
    GTKTERM_FLOW_CONTROL_RTSCTS,    /**< Hardware flow control (RTS/CTS) */
    GTKTERM_FLOW_CONTROL_DTRDSR     /**< Hardware flow control (DTR/DSR) */
} GtkTermFlowControl;

/**
 * @brief Serial signal lines
 */
typedef enum {
    GTKTERM_SIGNAL_DTR = (1 << 0),  /**< Data Terminal Ready */
    GTKTERM_SIGNAL_RTS = (1 << 1),  /**< Request To Send */
    GTKTERM_SIGNAL_CTS = (1 << 2),  /**< Clear To Send */
    GTKTERM_SIGNAL_DSR = (1 << 3),  /**< Data Set Ready */
    GTKTERM_SIGNAL_DCD = (1 << 4),  /**< Data Carrier Detect */
    GTKTERM_SIGNAL_RI  = (1 << 5),  /**< Ring Indicator */
} GtkTermSignalLine;

/**
 * @brief Parity modes
 */
typedef enum {
    GTKTERM_PARITY_NONE,
    GTKTERM_PARITY_ODD,
    GTKTERM_PARITY_EVEN,
    GTKTERM_PARITY_MARK,
    GTKTERM_PARITY_SPACE
} GtkTermParity;

/**
 * @brief Stop bits
 */
typedef enum {
    GTKTERM_STOP_BITS_1,
    GTKTERM_STOP_BITS_1_5,
    GTKTERM_STOP_BITS_2
} GtkTermStopBits;

/**
 * @brief Port configuration
 */
typedef struct {
    guint32 baud_rate;
    guint8 data_bits;
    GtkTermParity parity;
    GtkTermStopBits stop_bits;
    GtkTermFlowControl flow_control;
    gboolean canonical_mode;
    gboolean echo;
    guint vtime;  // Read timeout (tenths of second)
    guint vmin;   // Minimum characters for read
} GtkTermPortConfig;

/**
 * @brief Port diagnostics
 */
typedef struct {
    guint64 rx_bytes;
    guint64 tx_bytes;
    guint64 rx_errors;
    guint64 tx_errors;
    guint64 framing_errors;
    guint64 parity_errors;
    guint64 overrun_errors;
    guint64 break_interrupts;
    GDateTime *start_time;
    gdouble rx_rate;  // bytes/sec
    gdouble tx_rate;  // bytes/sec
} GtkTermPortDiagnostics;

#define GTKTERM_TYPE_PORT_CONTROL gtkterm_port_control_get_type ()
G_DECLARE_FINAL_TYPE (GtkTermPortControl, gtkterm_port_control, GTKTERM, PORT_CONTROL, GObject)

/**
 * @brief Create a new port control object
 */
GtkTermPortControl *gtkterm_port_control_new (const char *device);

/**
 * @brief Configure port
 */
gboolean gtkterm_port_control_configure (GtkTermPortControl *self,
                                          const GtkTermPortConfig *config,
                                          GError **error);

/**
 * @brief Get current configuration
 */
void gtkterm_port_control_get_config (GtkTermPortControl *self,
                                       GtkTermPortConfig *config);

/**
 * @brief Set signal line
 */
gboolean gtkterm_port_control_set_signal (GtkTermPortControl *self,
                                           GtkTermSignalLine signal,
                                           gboolean state,
                                           GError **error);

/**
 * @brief Get signal line state
 */
gboolean gtkterm_port_control_get_signal (GtkTermPortControl *self,
                                           GtkTermSignalLine signal,
                                           gboolean *state,
                                           GError **error);

/**
 * @brief Get all signal states
 */
guint32 gtkterm_port_control_get_all_signals (GtkTermPortControl *self,
                                                GError **error);

/**
 * @brief Send break signal
 */
gboolean gtkterm_port_control_send_break (GtkTermPortControl *self,
                                            guint duration_ms,
                                            GError **error);

/**
 * @brief Flush buffers
 */
gboolean gtkterm_port_control_flush (GtkTermPortControl *self,
                                      gboolean flush_input,
                                      gboolean flush_output,
                                      GError **error);

/**
 * @brief Detect baud rate
 */
gboolean gtkterm_port_control_detect_baud_rate (GtkTermPortControl *self,
                                                 guint32 *detected_rate,
                                                 GError **error);

/**
 * @brief Get diagnostics
 */
void gtkterm_port_control_get_diagnostics (GtkTermPortControl *self,
                                            GtkTermPortDiagnostics *diag);

/**
 * @brief Reset diagnostics
 */
void gtkterm_port_control_reset_diagnostics (GtkTermPortControl *self);

/**
 * @brief Enable signal monitoring
 */
void gtkterm_port_control_set_signal_monitoring (GtkTermPortControl *self,
                                                  gboolean enabled,
                                                  guint interval_ms);

/* Helper functions */
const char *gtkterm_flow_control_to_string (GtkTermFlowControl flow);
const char *gtkterm_parity_to_string (GtkTermParity parity);
const char *gtkterm_stop_bits_to_string (GtkTermStopBits stop_bits);
const char *gtkterm_signal_line_to_string (GtkTermSignalLine signal);

void gtkterm_port_config_init (GtkTermPortConfig *config);
void gtkterm_port_diagnostics_init (GtkTermPortDiagnostics *diag);
void gtkterm_port_diagnostics_clear (GtkTermPortDiagnostics *diag);

G_END_DECLS

#endif // GTKTERM_PORT_CONTROL_H
