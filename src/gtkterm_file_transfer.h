/************************************************************************/
/* gtkterm_file_transfer.h                                              */
/* -----------------------                                              */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      File transfer protocol interface (XMODEM/YMODEM/ZMODEM)       */
/*                                                                      */
/************************************************************************/

#ifndef GTKTERM_FILE_TRANSFER_H
#define GTKTERM_FILE_TRANSFER_H

#include <glib-object.h>
#include <gio/gio.h>

G_BEGIN_DECLS

/**
 * @brief File transfer protocol types
 */
typedef enum {
    GTKTERM_TRANSFER_PROTOCOL_XMODEM,      /**< XMODEM (128 byte blocks) */
    GTKTERM_TRANSFER_PROTOCOL_XMODEM_1K,   /**< XMODEM-1K (1024 byte blocks) */
    GTKTERM_TRANSFER_PROTOCOL_XMODEM_CRC,  /**< XMODEM with CRC */
    GTKTERM_TRANSFER_PROTOCOL_YMODEM,      /**< YMODEM (batch mode) */
    GTKTERM_TRANSFER_PROTOCOL_YMODEM_G,    /**< YMODEM-G (streaming) */
    GTKTERM_TRANSFER_PROTOCOL_ZMODEM       /**< ZMODEM */
} GtkTermTransferProtocol;

/**
 * @brief Transfer direction
 */
typedef enum {
    GTKTERM_TRANSFER_SEND,      /**< Send file to remote */
    GTKTERM_TRANSFER_RECEIVE    /**< Receive file from remote */
} GtkTermTransferDirection;

/**
 * @brief Transfer status
 */
typedef enum {
    GTKTERM_TRANSFER_STATUS_IDLE,
    GTKTERM_TRANSFER_STATUS_INITIALIZING,
    GTKTERM_TRANSFER_STATUS_TRANSFERRING,
    GTKTERM_TRANSFER_STATUS_COMPLETED,
    GTKTERM_TRANSFER_STATUS_CANCELLED,
    GTKTERM_TRANSFER_STATUS_ERROR
} GtkTermTransferStatus;

/**
 * @brief Transfer statistics
 */
typedef struct {
    guint64 bytes_transferred;
    guint64 total_bytes;
    guint blocks_transferred;
    guint total_blocks;
    guint errors;
    guint retries;
    gdouble transfer_rate;  // bytes per second
    GDateTime *start_time;
    GDateTime *end_time;
} GtkTermTransferStats;

#define GTKTERM_TYPE_FILE_TRANSFER gtkterm_file_transfer_get_type ()
G_DECLARE_INTERFACE (GtkTermFileTransfer, gtkterm_file_transfer, GTKTERM, FILE_TRANSFER, GObject)

/**
 * @brief File transfer interface
 */
struct _GtkTermFileTransferInterface {
    GTypeInterface parent_iface;

    /* Virtual methods */
    const char * (*get_name) (GtkTermFileTransfer *self);
    const char * (*get_description) (GtkTermFileTransfer *self);

    gboolean (*start_send) (GtkTermFileTransfer *self,
                            const char *file_path,
                            GError **error);

    gboolean (*start_receive) (GtkTermFileTransfer *self,
                               const char *save_path,
                               GError **error);

    void (*cancel) (GtkTermFileTransfer *self);

    void (*process_data) (GtkTermFileTransfer *self,
                          const guint8 *data,
                          gsize length);

    GtkTermTransferStatus (*get_status) (GtkTermFileTransfer *self);

    void (*get_stats) (GtkTermFileTransfer *self,
                       GtkTermTransferStats *stats);

    /* Signals */
    void (*progress) (GtkTermFileTransfer *self,
                      guint64 bytes_transferred,
                      guint64 total_bytes);

    void (*completed) (GtkTermFileTransfer *self,
                       gboolean success,
                       const char *message);

    void (*error_occurred) (GtkTermFileTransfer *self,
                            const char *error_message);
};

/* Interface methods */
const char *gtkterm_file_transfer_get_name (GtkTermFileTransfer *self);

const char *gtkterm_file_transfer_get_description (GtkTermFileTransfer *self);

gboolean gtkterm_file_transfer_start_send (GtkTermFileTransfer *self,
                                            const char *file_path,
                                            GError **error);

gboolean gtkterm_file_transfer_start_receive (GtkTermFileTransfer *self,
                                               const char *save_path,
                                               GError **error);

void gtkterm_file_transfer_cancel (GtkTermFileTransfer *self);

void gtkterm_file_transfer_process_data (GtkTermFileTransfer *self,
                                          const guint8 *data,
                                          gsize length);

GtkTermTransferStatus gtkterm_file_transfer_get_status (GtkTermFileTransfer *self);

void gtkterm_file_transfer_get_stats (GtkTermFileTransfer *self,
                                       GtkTermTransferStats *stats);

/* Helper functions */
void gtkterm_transfer_stats_init (GtkTermTransferStats *stats);
void gtkterm_transfer_stats_clear (GtkTermTransferStats *stats);

G_END_DECLS

#endif // GTKTERM_FILE_TRANSFER_H
