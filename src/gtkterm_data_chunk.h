/************************************************************************/
/* gtkterm_data_chunk.h                                                	*/
/* --------------------                                                	*/
/*           GTKTerm Software                                          	*/
/*                      (c) 2025 Contributors                           	*/
/*                                                                     	*/
/* ------------------------------------------------------------------- 	*/
/*                                                                     	*/
/*   Purpose                                                           	*/
/*      Data chunk structure for storing serial data with metadata    	*/
/*                                                                     	*/
/* This GtkTerm is free software: you can redistribute it and/or modify	*/
/* it under the terms of the GNU  General Public License as published  	*/
/* by the Free Software Foundation, either version 3 of the License,   	*/
/* or (at your option) any later version.							   	*/
/*																	   	*/
/* GtkTerm is distributed in the hope that it will be useful, but	   	*/
/* WITHOUT ANY WARRANTY; without even the implied warranty of 		   	*/
/* MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. 			   	*/
/* See the GNU General Public License for more details.					*/
/*																		*/
/* You should have received a copy of the GNU General Public License 	*/
/* along with GtkTerm If not, see <https://www.gnu.org/licenses/>. 		*/
/*                                                                     	*/
/************************************************************************/

#ifndef GTKTERM_DATA_CHUNK_H
#define GTKTERM_DATA_CHUNK_H

#include <glib.h>

G_BEGIN_DECLS

/**
 * @brief Direction of data transmission
 */
typedef enum {
    GTKTERM_DATA_DIRECTION_RX = 0,  /**< Received data from serial port */
    GTKTERM_DATA_DIRECTION_TX = 1   /**< Transmitted data to serial port */
} GtkTermDataDirection;

/**
 * @brief Data chunk with metadata
 *
 * Stores a chunk of serial data with timestamp and direction metadata.
 * This allows for proper logging, replay, and display formatting.
 */
typedef struct _GtkTermDataChunk {
    gint64 timestamp_us;                /**< Timestamp in microseconds since epoch */
    GtkTermDataDirection direction;     /**< Direction: RX or TX */
    guint8 *data;                       /**< Raw data bytes */
    gsize length;                       /**< Length of data */
} GtkTermDataChunk;

/**
 * @brief Create a new data chunk
 *
 * @param data Raw bytes to store (will be copied)
 * @param length Length of data
 * @param direction Direction (RX or TX)
 * @param timestamp_us Timestamp in microseconds (0 = use current time)
 *
 * @return Newly allocated data chunk (must be freed with gtkterm_data_chunk_free)
 */
GtkTermDataChunk *gtkterm_data_chunk_new (const guint8 *data,
                                          gsize length,
                                          GtkTermDataDirection direction,
                                          gint64 timestamp_us);

/**
 * @brief Free a data chunk
 *
 * @param chunk The chunk to free
 */
void gtkterm_data_chunk_free (GtkTermDataChunk *chunk);

/**
 * @brief Get human-readable timestamp string
 *
 * @param chunk The data chunk
 * @param format Format: "absolute", "relative", "delta"
 * @param reference_time Reference timestamp for relative/delta (0 = session start)
 *
 * @return Newly allocated string (must be freed with g_free)
 */
char *gtkterm_data_chunk_get_timestamp_string (const GtkTermDataChunk *chunk,
                                                const char *format,
                                                gint64 reference_time);

/**
 * @brief Get current time in microseconds
 *
 * @return Current time in microseconds since epoch
 */
gint64 gtkterm_data_chunk_get_current_time_us (void);

G_END_DECLS

#endif // GTKTERM_DATA_CHUNK_H
