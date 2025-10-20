/************************************************************************/
/* gtkterm_terminal.h                                               	*/
/* ------------------                                               	*/
/*           GTKTerm Software                                          	*/
/*                      (c) Julien Schmitt                             	*/
/*                                                                     	*/
/* ------------------------------------------------------------------- 	*/
/*                                                                     	*/
/*   Purpose                                                           	*/
/*      Include file for GtkTermTerminal	                 			*/
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

#ifndef GTKTERM_TERMINAL_H
#define GTKTERM_TERMINAL_H

#include <glib-object.h>
#include <gtk/gtk.h>
#include <vte/vte.h>

#include "gtkterm.h"
#include "gtkterm_serial_port.h"

/* Forward declarations */
typedef struct _GtkTermBuffer GtkTermBuffer;
typedef struct _GtkTermWindow GtkTermWindow;

/**
 * @brief The typedef for mode in which the output of the terminal can be viewed.
 *
 */
typedef enum  {
    GTKTERM_TERMINAL_VIEW_ASCII,
    GTKTERM_TERMINAL_VIEW_HEX,
    GTKTERM_TERMINAL_VIEW_BINARY,
    GTKTERM_TERMINAL_VIEW_DECIMAL,
    GTKTERM_TERMINAL_VIEW_MIXED,         /* Hex + ASCII side-by-side */
    GTKTERM_TERMINAL_VIEW_BINARY_MIXED,  /* Binary + ASCII side-by-side */
    GTKTERM_TERMINAL_VIEW_DECIMAL_MIXED  /* Decimal + ASCII side-by-side */

} GtkTermTerminalView;

/**
 * @brief The typedef for the terminal configuration.
 *
 */
typedef struct {
	
	bool block_cursor;					/** Show a block shape cursor	*/
	bool show_cursor;					/** Show cursor in window. \todo This is not possible, so remove? */
	char char_queue;            		/** character in queue			*/
	bool echo;               			/** local echo 					*/
	bool auto_lf;           			/** auto line feed				*/
	bool auto_cr;           			/** auto return					*/
	bool timestamp;						/** Show timestamp in output	*/
	int delay;                  		/** end of char delay: in ms	*/
	int rows;							/** Number of rows in terminal  */
	int columns;						/** Number of cols in terminal  */
	int scrollback;						/** Number of scrollback lines  */
	int show_index;						/** Show index in output		*/
	GtkTermTerminalView view_mode;		/** Mode to view output in		*/
	int hex_chars;						/** Number of chars in hex mode	*/	
	bool visual_bell;					/**	Visual bell					*/
	GdkRGBA foreground_color;			/** Terminal Background color	*/
	GdkRGBA background_color;			/** Terminal Foreground color   */
	PangoFontDescription *font;			/** Terminal Font				*/

} term_config_t;

G_BEGIN_DECLS

#define GTKTERM_TYPE_TERMINAL gtkterm_terminal_get_type()
G_DECLARE_FINAL_TYPE (GtkTermTerminal, gtkterm_terminal, GTKTERM, TERMINAL, VteTerminal)
typedef struct _GtkTermTerminal GtkTermTerminal;

GtkTermTerminal *gtkterm_terminal_new (char *, GtkTerm *, GtkTermWindow *);

/* Public accessor functions */
void gtkterm_terminal_set_view_mode (GtkTermTerminal *, GtkTermTerminalView);
GtkTermTerminalView gtkterm_terminal_get_view_mode (GtkTermTerminal *);
void gtkterm_terminal_set_echo (GtkTermTerminal *, bool);
bool gtkterm_terminal_get_echo (GtkTermTerminal *);
void gtkterm_terminal_set_auto_cr (GtkTermTerminal *, bool);
bool gtkterm_terminal_get_auto_cr (GtkTermTerminal *);
void gtkterm_terminal_set_auto_lf (GtkTermTerminal *, bool);
bool gtkterm_terminal_get_auto_lf (GtkTermTerminal *);
void gtkterm_terminal_set_timestamp (GtkTermTerminal *, bool);
bool gtkterm_terminal_get_timestamp (GtkTermTerminal *);
void gtkterm_terminal_set_hex_chars (GtkTermTerminal *, int);
void gtkterm_terminal_set_show_index (GtkTermTerminal *, bool);
GtkTermSerialPort *gtkterm_terminal_get_serial_port (GtkTermTerminal *);
GtkTermBuffer *gtkterm_terminal_get_buffer (GtkTermTerminal *);

G_END_DECLS

#endif // GTKTERM_TERMINAL_H