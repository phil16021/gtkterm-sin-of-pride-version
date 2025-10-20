/************************************************************************/
/* gtkterm_file_transfer.c                                              */
/* -----------------------                                              */
/*           GTKTerm Software                                           */
/*                      (c) 2025 Contributors                           */
/*                                                                      */
/* -------------------------------------------------------------------- */
/*                                                                      */
/*   Purpose                                                            */
/*      File transfer protocol interface implementation                */
/*                                                                      */
/************************************************************************/

#include "gtkterm_file_transfer.h"

G_DEFINE_INTERFACE (GtkTermFileTransfer, gtkterm_file_transfer, G_TYPE_OBJECT)

enum {
    SIGNAL_PROGRESS,
    SIGNAL_COMPLETED,
    SIGNAL_ERROR,
    N_SIGNALS
};

static guint signals[N_SIGNALS] = {0};

static void gtkterm_file_transfer_default_init (GtkTermFileTransferInterface *iface) {
    (void)iface;

    /**
     * GtkTermFileTransfer::progress:
     * @self: the file transfer object
     * @bytes_transferred: number of bytes transferred
     * @total_bytes: total bytes to transfer
     *
     * Emitted when transfer progress is updated
     */
    signals[SIGNAL_PROGRESS] = g_signal_new (
        "progress",
        G_TYPE_FROM_INTERFACE (iface),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL,
        NULL,
        G_TYPE_NONE,
        2,
        G_TYPE_UINT64,
        G_TYPE_UINT64
    );

    /**
     * GtkTermFileTransfer::completed:
     * @self: the file transfer object
     * @success: whether the transfer completed successfully
     * @message: completion message
     *
     * Emitted when transfer completes or fails
     */
    signals[SIGNAL_COMPLETED] = g_signal_new (
        "completed",
        G_TYPE_FROM_INTERFACE (iface),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL,
        NULL,
        G_TYPE_NONE,
        2,
        G_TYPE_BOOLEAN,
        G_TYPE_STRING
    );

    /**
     * GtkTermFileTransfer::error:
     * @self: the file transfer object
     * @error_message: error description
     *
     * Emitted when an error occurs during transfer
     */
    signals[SIGNAL_ERROR] = g_signal_new (
        "error",
        G_TYPE_FROM_INTERFACE (iface),
        G_SIGNAL_RUN_LAST,
        0,
        NULL, NULL,
        NULL,
        G_TYPE_NONE,
        1,
        G_TYPE_STRING
    );
}

/**
 * @brief Get protocol name
 */
const char *gtkterm_file_transfer_get_name (GtkTermFileTransfer *self) {
    GtkTermFileTransferInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_FILE_TRANSFER (self), NULL);

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_val_if_fail (iface->get_name != NULL, NULL);

    return iface->get_name (self);
}

/**
 * @brief Get protocol description
 */
const char *gtkterm_file_transfer_get_description (GtkTermFileTransfer *self) {
    GtkTermFileTransferInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_FILE_TRANSFER (self), NULL);

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_val_if_fail (iface->get_description != NULL, NULL);

    return iface->get_description (self);
}

/**
 * @brief Start sending a file
 */
gboolean gtkterm_file_transfer_start_send (GtkTermFileTransfer *self,
                                            const char *file_path,
                                            GError **error) {
    GtkTermFileTransferInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_FILE_TRANSFER (self), FALSE);
    g_return_val_if_fail (file_path != NULL, FALSE);

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_val_if_fail (iface->start_send != NULL, FALSE);

    return iface->start_send (self, file_path, error);
}

/**
 * @brief Start receiving a file
 */
gboolean gtkterm_file_transfer_start_receive (GtkTermFileTransfer *self,
                                               const char *save_path,
                                               GError **error) {
    GtkTermFileTransferInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_FILE_TRANSFER (self), FALSE);
    g_return_val_if_fail (save_path != NULL, FALSE);

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_val_if_fail (iface->start_receive != NULL, FALSE);

    return iface->start_receive (self, save_path, error);
}

/**
 * @brief Cancel transfer
 */
void gtkterm_file_transfer_cancel (GtkTermFileTransfer *self) {
    GtkTermFileTransferInterface *iface;

    g_return_if_fail (GTKTERM_IS_FILE_TRANSFER (self));

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_if_fail (iface->cancel != NULL);

    iface->cancel (self);
}

/**
 * @brief Process incoming data
 */
void gtkterm_file_transfer_process_data (GtkTermFileTransfer *self,
                                          const guint8 *data,
                                          gsize length) {
    GtkTermFileTransferInterface *iface;

    g_return_if_fail (GTKTERM_IS_FILE_TRANSFER (self));
    g_return_if_fail (data != NULL || length == 0);

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_if_fail (iface->process_data != NULL);

    iface->process_data (self, data, length);
}

/**
 * @brief Get transfer status
 */
GtkTermTransferStatus gtkterm_file_transfer_get_status (GtkTermFileTransfer *self) {
    GtkTermFileTransferInterface *iface;

    g_return_val_if_fail (GTKTERM_IS_FILE_TRANSFER (self), GTKTERM_TRANSFER_STATUS_IDLE);

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_val_if_fail (iface->get_status != NULL, GTKTERM_TRANSFER_STATUS_IDLE);

    return iface->get_status (self);
}

/**
 * @brief Get transfer statistics
 */
void gtkterm_file_transfer_get_stats (GtkTermFileTransfer *self,
                                       GtkTermTransferStats *stats) {
    GtkTermFileTransferInterface *iface;

    g_return_if_fail (GTKTERM_IS_FILE_TRANSFER (self));
    g_return_if_fail (stats != NULL);

    iface = GTKTERM_FILE_TRANSFER_GET_IFACE (self);
    g_return_if_fail (iface->get_stats != NULL);

    iface->get_stats (self, stats);
}

/**
 * @brief Initialize transfer stats structure
 */
void gtkterm_transfer_stats_init (GtkTermTransferStats *stats) {
    g_return_if_fail (stats != NULL);

    stats->bytes_transferred = 0;
    stats->total_bytes = 0;
    stats->blocks_transferred = 0;
    stats->total_blocks = 0;
    stats->errors = 0;
    stats->retries = 0;
    stats->transfer_rate = 0.0;
    stats->start_time = NULL;
    stats->end_time = NULL;
}

/**
 * @brief Clear transfer stats and free resources
 */
void gtkterm_transfer_stats_clear (GtkTermTransferStats *stats) {
    g_return_if_fail (stats != NULL);

    if (stats->start_time) {
        g_date_time_unref (stats->start_time);
        stats->start_time = NULL;
    }

    if (stats->end_time) {
        g_date_time_unref (stats->end_time);
        stats->end_time = NULL;
    }
}
