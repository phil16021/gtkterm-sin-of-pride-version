/************************************************************************/
/* gtkterm_xmodem.c                                                     */
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

#include "gtkterm_xmodem.h"
#include <string.h>
#include <stdio.h>

/* XMODEM control characters */
#define SOH  0x01  /* Start of 128-byte block */
#define STX  0x02  /* Start of 1K-byte block */
#define EOT  0x04  /* End of transmission */
#define ACK  0x06  /* Acknowledge */
#define NAK  0x15  /* Negative acknowledge */
#define CAN  0x18  /* Cancel */
#define CTRLZ 0x1A /* EOF marker */

#define XMODEM_BLOCK_SIZE      128
#define XMODEM_1K_BLOCK_SIZE   1024
#define XMODEM_MAX_ERRORS      10
#define XMODEM_TIMEOUT_SEC     10

struct _GtkTermXModem {
    GObject parent_instance;

    /* Configuration */
    GtkTermXModemVariant variant;
    gboolean (*write_func)(const guint8 *, gsize, gpointer);
    gpointer write_user_data;

    /* Transfer state */
    GtkTermTransferDirection direction;
    GtkTermTransferStatus status;
    GFile *file;
    GFileInputStream *input_stream;
    GFileOutputStream *output_stream;

    /* Statistics */
    GtkTermTransferStats stats;

    /* Protocol state */
    guint8 block_number;
    guint8 *receive_buffer;
    gsize receive_buffer_len;
    gsize receive_buffer_pos;
    guint consecutive_errors;
    gboolean use_crc;
    gsize block_size;

    /* Timeout */
    guint timeout_id;
};

static void gtkterm_xmodem_file_transfer_interface_init (GtkTermFileTransferInterface *iface);

G_DEFINE_TYPE_WITH_CODE (GtkTermXModem, gtkterm_xmodem, G_TYPE_OBJECT,
                         G_IMPLEMENT_INTERFACE (GTKTERM_TYPE_FILE_TRANSFER,
                                                gtkterm_xmodem_file_transfer_interface_init))

/**
 * @brief Calculate checksum
 */
static guint8 calculate_checksum (const guint8 *data, gsize length) {
    guint8 sum = 0;
    for (gsize i = 0; i < length; i++) {
        sum += data[i];
    }
    return sum;
}

/**
 * @brief Calculate CRC16
 */
static guint16 calculate_crc16 (const guint8 *data, gsize length) {
    guint16 crc = 0;

    for (gsize i = 0; i < length; i++) {
        crc = crc ^ (guint16)data[i] << 8;
        for (int j = 0; j < 8; j++) {
            if (crc & 0x8000) {
                crc = crc << 1 ^ 0x1021;
            } else {
                crc = crc << 1;
            }
        }
    }

    return crc;
}

/**
 * @brief Send a byte
 */
static gboolean send_byte (GtkTermXModem *self, guint8 byte) {
    if (!self->write_func) {
        return FALSE;
    }

    return self->write_func (&byte, 1, self->write_user_data);
}

/**
 * @brief Send a block
 */
static gboolean send_block (GtkTermXModem *self, guint8 block_num, const guint8 *data, gsize data_len) {
    guint8 header = (self->block_size == XMODEM_1K_BLOCK_SIZE) ? STX : SOH;
    guint8 block[3 + XMODEM_1K_BLOCK_SIZE + 2]; // header + block# + ~block# + data + checksum/CRC
    gsize idx = 0;

    // Header
    block[idx++] = header;
    block[idx++] = block_num;
    block[idx++] = 255 - block_num;

    // Data (pad with CTRLZ if needed)
    memcpy (&block[idx], data, data_len);
    if (data_len < self->block_size) {
        memset (&block[idx + data_len], CTRLZ, self->block_size - data_len);
    }
    idx += self->block_size;

    // Checksum or CRC
    if (self->use_crc) {
        guint16 crc = calculate_crc16 (&block[3], self->block_size);
        block[idx++] = (crc >> 8) & 0xFF;
        block[idx++] = crc & 0xFF;
    } else {
        block[idx++] = calculate_checksum (&block[3], self->block_size);
    }

    return self->write_func (block, idx, self->write_user_data);
}

/**
 * @brief Finish transfer with success
 */
static void finish_transfer (GtkTermXModem *self, gboolean success, const char *message) {
    self->status = success ? GTKTERM_TRANSFER_STATUS_COMPLETED : GTKTERM_TRANSFER_STATUS_ERROR;

    if (self->stats.end_time == NULL) {
        self->stats.end_time = g_date_time_new_now_local ();

        // Calculate transfer rate
        if (self->stats.start_time) {
            GTimeSpan duration = g_date_time_difference (self->stats.end_time, self->stats.start_time);
            if (duration > 0) {
                self->stats.transfer_rate = (gdouble)self->stats.bytes_transferred /
                                           ((gdouble)duration / G_TIME_SPAN_SECOND);
            }
        }
    }

    // Cleanup
    if (self->input_stream) {
        g_object_unref (self->input_stream);
        self->input_stream = NULL;
    }

    if (self->output_stream) {
        g_object_unref (self->output_stream);
        self->output_stream = NULL;
    }

    if (self->timeout_id) {
        g_source_remove (self->timeout_id);
        self->timeout_id = 0;
    }

    g_signal_emit_by_name (self, "completed", success, message);
}

/**
 * @brief Handle timeout
 */
static gboolean on_timeout (gpointer user_data) {
    GtkTermXModem *self = GTKTERM_XMODEM (user_data);

    self->timeout_id = 0;
    self->consecutive_errors++;

    if (self->consecutive_errors >= XMODEM_MAX_ERRORS) {
        finish_transfer (self, FALSE, "Transfer timeout - too many errors");
        return G_SOURCE_REMOVE;
    }

    // Retry
    if (self->direction == GTKTERM_TRANSFER_RECEIVE) {
        send_byte (self, self->use_crc ? 'C' : NAK);
    }

    return G_SOURCE_REMOVE;
}

/**
 * @brief Start timeout timer
 */
static void start_timeout (GtkTermXModem *self) {
    if (self->timeout_id) {
        g_source_remove (self->timeout_id);
    }
    self->timeout_id = g_timeout_add_seconds (XMODEM_TIMEOUT_SEC, on_timeout, self);
}

/**
 * @brief Get protocol name
 */
static const char *xmodem_get_name (GtkTermFileTransfer *transfer) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);

    switch (self->variant) {
        case GTKTERM_XMODEM_STANDARD:
            return "XMODEM";
        case GTKTERM_XMODEM_CRC:
            return "XMODEM-CRC";
        case GTKTERM_XMODEM_1K:
            return "XMODEM-1K";
        default:
            return "XMODEM";
    }
}

/**
 * @brief Get protocol description
 */
static const char *xmodem_get_description (GtkTermFileTransfer *transfer) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);

    switch (self->variant) {
        case GTKTERM_XMODEM_STANDARD:
            return "XMODEM protocol with checksum (128-byte blocks)";
        case GTKTERM_XMODEM_CRC:
            return "XMODEM protocol with CRC (128-byte blocks)";
        case GTKTERM_XMODEM_1K:
            return "XMODEM-1K protocol with CRC (1024-byte blocks)";
        default:
            return "XMODEM protocol";
    }
}

/**
 * @brief Start sending a file
 */
static gboolean xmodem_start_send (GtkTermFileTransfer *transfer,
                                    const char *file_path,
                                    GError **error) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);

    if (self->status == GTKTERM_TRANSFER_STATUS_TRANSFERRING) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_BUSY, "Transfer already in progress");
        return FALSE;
    }

    // Open file
    self->file = g_file_new_for_path (file_path);
    self->input_stream = g_file_read (self->file, NULL, error);

    if (!self->input_stream) {
        g_object_unref (self->file);
        self->file = NULL;
        return FALSE;
    }

    // Get file size
    GFileInfo *info = g_file_query_info (self->file,
                                          G_FILE_ATTRIBUTE_STANDARD_SIZE,
                                          G_FILE_QUERY_INFO_NONE,
                                          NULL, error);
    if (info) {
        self->stats.total_bytes = g_file_info_get_size (info);
        self->stats.total_blocks = (self->stats.total_bytes + self->block_size - 1) / self->block_size;
        g_object_unref (info);
    }

    // Initialize transfer
    self->direction = GTKTERM_TRANSFER_SEND;
    self->status = GTKTERM_TRANSFER_STATUS_INITIALIZING;
    self->block_number = 1;
    self->consecutive_errors = 0;
    self->stats.start_time = g_date_time_new_now_local ();

    // Wait for receiver to send NAK or 'C'
    start_timeout (self);

    return TRUE;
}

/**
 * @brief Start receiving a file
 */
static gboolean xmodem_start_receive (GtkTermFileTransfer *transfer,
                                       const char *save_path,
                                       GError **error) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);

    if (self->status == GTKTERM_TRANSFER_STATUS_TRANSFERRING) {
        g_set_error (error, G_IO_ERROR, G_IO_ERROR_BUSY, "Transfer already in progress");
        return FALSE;
    }

    // Create output file
    self->file = g_file_new_for_path (save_path);
    self->output_stream = g_file_create (self->file,
                                          G_FILE_CREATE_REPLACE_DESTINATION,
                                          NULL, error);

    if (!self->output_stream) {
        g_object_unref (self->file);
        self->file = NULL;
        return FALSE;
    }

    // Initialize transfer
    self->direction = GTKTERM_TRANSFER_RECEIVE;
    self->status = GTKTERM_TRANSFER_STATUS_INITIALIZING;
    self->block_number = 1;
    self->consecutive_errors = 0;
    self->receive_buffer_pos = 0;
    self->stats.start_time = g_date_time_new_now_local ();

    // Request transfer with CRC if available
    if (self->variant == GTKTERM_XMODEM_CRC || self->variant == GTKTERM_XMODEM_1K) {
        self->use_crc = TRUE;
        send_byte (self, 'C');
    } else {
        self->use_crc = FALSE;
        send_byte (self, NAK);
    }

    start_timeout (self);

    return TRUE;
}

/**
 * @brief Cancel transfer
 */
static void xmodem_cancel (GtkTermFileTransfer *transfer) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);

    // Send cancel character
    send_byte (self, CAN);
    send_byte (self, CAN);

    finish_transfer (self, FALSE, "Transfer cancelled by user");
}

/**
 * @brief Process received data during send
 */
static void process_send_data (GtkTermXModem *self, const guint8 *data, gsize length) {
    for (gsize i = 0; i < length; i++) {
        guint8 byte = data[i];

        if (self->status == GTKTERM_TRANSFER_STATUS_INITIALIZING) {
            // Wait for NAK or 'C'
            if (byte == NAK) {
                self->use_crc = FALSE;
                self->status = GTKTERM_TRANSFER_STATUS_TRANSFERRING;
                // Fall through to send first block
            } else if (byte == 'C') {
                self->use_crc = TRUE;
                self->status = GTKTERM_TRANSFER_STATUS_TRANSFERRING;
                // Fall through to send first block
            } else if (byte == CAN) {
                finish_transfer (self, FALSE, "Transfer cancelled by receiver");
                return;
            } else {
                continue;
            }
        }

        if (self->status == GTKTERM_TRANSFER_STATUS_TRANSFERRING) {
            if (byte == ACK) {
                // Block acknowledged, send next
                self->consecutive_errors = 0;
                start_timeout (self);

                // Read next block
                guint8 buffer[XMODEM_1K_BLOCK_SIZE];
                gssize bytes_read = g_input_stream_read (G_INPUT_STREAM (self->input_stream),
                                                          buffer, self->block_size,
                                                          NULL, NULL);

                if (bytes_read > 0) {
                    send_block (self, self->block_number, buffer, bytes_read);
                    self->stats.bytes_transferred += bytes_read;
                    self->stats.blocks_transferred++;
                    self->block_number++;

                    g_signal_emit_by_name (self, "progress",
                                          self->stats.bytes_transferred,
                                          self->stats.total_bytes);
                } else {
                    // End of file
                    send_byte (self, EOT);
                }

            } else if (byte == NAK) {
                // Retransmit current block
                self->consecutive_errors++;
                self->stats.retries++;

                if (self->consecutive_errors >= XMODEM_MAX_ERRORS) {
                    finish_transfer (self, FALSE, "Too many errors");
                    return;
                }

                // Seek back and resend
                g_seekable_seek (G_SEEKABLE (self->input_stream),
                                -(gssize)self->block_size,
                                G_SEEK_CUR, NULL, NULL);

                guint8 buffer[XMODEM_1K_BLOCK_SIZE];
                gssize bytes_read = g_input_stream_read (G_INPUT_STREAM (self->input_stream),
                                                          buffer, self->block_size,
                                                          NULL, NULL);

                if (bytes_read > 0) {
                    send_block (self, self->block_number - 1, buffer, bytes_read);
                }

            } else if (byte == CAN) {
                finish_transfer (self, FALSE, "Transfer cancelled by receiver");
                return;
            }
        }
    }
}

/**
 * @brief Process received block
 */
static void process_received_block (GtkTermXModem *self) {
    guint8 *block = self->receive_buffer;

    // Verify block number
    guint8 block_num = block[1];
    guint8 block_num_complement = block[2];

    if (block_num != (255 - block_num_complement)) {
        self->stats.errors++;
        self->consecutive_errors++;
        send_byte (self, NAK);
        return;
    }

    // Verify checksum or CRC
    gboolean valid = FALSE;

    if (self->use_crc) {
        guint16 received_crc = (block[3 + self->block_size] << 8) | block[3 + self->block_size + 1];
        guint16 calculated_crc = calculate_crc16 (&block[3], self->block_size);
        valid = (received_crc == calculated_crc);
    } else {
        guint8 received_checksum = block[3 + self->block_size];
        guint8 calculated_checksum = calculate_checksum (&block[3], self->block_size);
        valid = (received_checksum == calculated_checksum);
    }

    if (!valid) {
        self->stats.errors++;
        self->consecutive_errors++;
        send_byte (self, NAK);
        return;
    }

    // Check for duplicate block
    if (block_num == self->block_number - 1) {
        // Duplicate - just ACK it
        send_byte (self, ACK);
        return;
    }

    if (block_num != self->block_number) {
        // Out of sequence
        self->stats.errors++;
        self->consecutive_errors++;
        send_byte (self, NAK);
        return;
    }

    // Write data to file
    gsize bytes_to_write = self->block_size;

    // Remove padding (CTRLZ) from last block
    for (gsize i = self->block_size; i > 0; i--) {
        if (block[3 + i - 1] != CTRLZ) {
            bytes_to_write = i;
            break;
        }
    }

    g_output_stream_write (G_OUTPUT_STREAM (self->output_stream),
                          &block[3], bytes_to_write,
                          NULL, NULL);

    self->stats.bytes_transferred += bytes_to_write;
    self->stats.blocks_transferred++;
    self->block_number++;
    self->consecutive_errors = 0;

    g_signal_emit_by_name (self, "progress",
                          self->stats.bytes_transferred,
                          self->stats.total_bytes);

    send_byte (self, ACK);
}

/**
 * @brief Process received data during receive
 */
static void process_receive_data (GtkTermXModem *self, const guint8 *data, gsize length) {
    for (gsize i = 0; i < length; i++) {
        guint8 byte = data[i];

        if (byte == EOT) {
            // End of transmission
            send_byte (self, ACK);
            finish_transfer (self, TRUE, "Transfer completed successfully");
            return;
        }

        if (byte == CAN) {
            finish_transfer (self, FALSE, "Transfer cancelled by sender");
            return;
        }

        if (self->receive_buffer_pos == 0) {
            // Expecting header
            if (byte == SOH) {
                self->block_size = XMODEM_BLOCK_SIZE;
                self->receive_buffer_len = 3 + XMODEM_BLOCK_SIZE + (self->use_crc ? 2 : 1);
            } else if (byte == STX) {
                self->block_size = XMODEM_1K_BLOCK_SIZE;
                self->receive_buffer_len = 3 + XMODEM_1K_BLOCK_SIZE + (self->use_crc ? 2 : 1);
            } else {
                continue; // Ignore invalid bytes
            }

            self->status = GTKTERM_TRANSFER_STATUS_TRANSFERRING;
        }

        self->receive_buffer[self->receive_buffer_pos++] = byte;

        if (self->receive_buffer_pos >= self->receive_buffer_len) {
            // Complete block received
            start_timeout (self);
            process_received_block (self);
            self->receive_buffer_pos = 0;
        }
    }
}

/**
 * @brief Process incoming data
 */
static void xmodem_process_data (GtkTermFileTransfer *transfer,
                                  const guint8 *data,
                                  gsize length) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);

    if (self->direction == GTKTERM_TRANSFER_SEND) {
        process_send_data (self, data, length);
    } else {
        process_receive_data (self, data, length);
    }
}

/**
 * @brief Get transfer status
 */
static GtkTermTransferStatus xmodem_get_status (GtkTermFileTransfer *transfer) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);
    return self->status;
}

/**
 * @brief Get transfer statistics
 */
static void xmodem_get_stats (GtkTermFileTransfer *transfer,
                               GtkTermTransferStats *stats) {
    GtkTermXModem *self = GTKTERM_XMODEM (transfer);
    *stats = self->stats;
}

/**
 * @brief Interface initialization
 */
static void gtkterm_xmodem_file_transfer_interface_init (GtkTermFileTransferInterface *iface) {
    iface->get_name = xmodem_get_name;
    iface->get_description = xmodem_get_description;
    iface->start_send = xmodem_start_send;
    iface->start_receive = xmodem_start_receive;
    iface->cancel = xmodem_cancel;
    iface->process_data = xmodem_process_data;
    iface->get_status = xmodem_get_status;
    iface->get_stats = xmodem_get_stats;
}

/**
 * @brief Dispose
 */
static void gtkterm_xmodem_dispose (GObject *object) {
    GtkTermXModem *self = GTKTERM_XMODEM (object);

    if (self->timeout_id) {
        g_source_remove (self->timeout_id);
        self->timeout_id = 0;
    }

    if (self->input_stream) {
        g_object_unref (self->input_stream);
        self->input_stream = NULL;
    }

    if (self->output_stream) {
        g_object_unref (self->output_stream);
        self->output_stream = NULL;
    }

    if (self->file) {
        g_object_unref (self->file);
        self->file = NULL;
    }

    gtkterm_transfer_stats_clear (&self->stats);

    G_OBJECT_CLASS (gtkterm_xmodem_parent_class)->dispose (object);
}

/**
 * @brief Finalize
 */
static void gtkterm_xmodem_finalize (GObject *object) {
    GtkTermXModem *self = GTKTERM_XMODEM (object);

    g_free (self->receive_buffer);

    G_OBJECT_CLASS (gtkterm_xmodem_parent_class)->finalize (object);
}

/**
 * @brief Class initialization
 */
static void gtkterm_xmodem_class_init (GtkTermXModemClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS (klass);

    object_class->dispose = gtkterm_xmodem_dispose;
    object_class->finalize = gtkterm_xmodem_finalize;
}

/**
 * @brief Instance initialization
 */
static void gtkterm_xmodem_init (GtkTermXModem *self) {
    self->variant = GTKTERM_XMODEM_STANDARD;
    self->status = GTKTERM_TRANSFER_STATUS_IDLE;
    self->block_size = XMODEM_BLOCK_SIZE;
    self->use_crc = FALSE;
    self->receive_buffer = g_malloc (3 + XMODEM_1K_BLOCK_SIZE + 2);
    self->receive_buffer_pos = 0;
    gtkterm_transfer_stats_init (&self->stats);
}

/**
 * @brief Create a new XMODEM transfer object
 */
GtkTermXModem *gtkterm_xmodem_new (GtkTermXModemVariant variant) {
    GtkTermXModem *xmodem = g_object_new (GTKTERM_TYPE_XMODEM, NULL);
    xmodem->variant = variant;

    if (variant == GTKTERM_XMODEM_1K) {
        xmodem->block_size = XMODEM_1K_BLOCK_SIZE;
        xmodem->use_crc = TRUE;
    } else if (variant == GTKTERM_XMODEM_CRC) {
        xmodem->block_size = XMODEM_BLOCK_SIZE;
        xmodem->use_crc = TRUE;
    } else {
        xmodem->block_size = XMODEM_BLOCK_SIZE;
        xmodem->use_crc = FALSE;
    }

    return xmodem;
}

/**
 * @brief Set the serial port write function
 */
void gtkterm_xmodem_set_write_func (GtkTermXModem *self,
                                     gboolean (*write_func)(const guint8 *, gsize, gpointer),
                                     gpointer user_data) {
    g_return_if_fail (GTKTERM_IS_XMODEM (self));

    self->write_func = write_func;
    self->write_user_data = user_data;
}
