/************************************************************************/
/* gtkterm_terminal.c                                                   */
/* ------------------                                                   */
/*           GTKTerm Software                                           */
/*                      (c) Julien Schmitt                              */
/*                                                                      */
/* -------------------------------------------------------------------  */
/*                                                                      */
/*   Purpose                                                            */
/*      Handles all VTE in/output to/from serial port                   */
/*                                                                      */
/*   ChangeLog                                                          */
/*       - 2.0   : Port to Gtk4                                         */
/*       - 1.01  : The put_hexadecimal partly function rewritten.       */
/*                 The vte_terminal_get_cursor_position function does   */
/*                 not return always the actual column.                 */
/*                 Now it uses an internal column-index (virt_col_pos). */
/*                 (Willem van den Akker)                               */
/*      - 0.99.7 : Changed keyboard shortcuts to <ctrl><shift>          */
/*	               (Ken Peek)                                           */
/*      - 0.99.6 : Added scrollbar and copy/paste (Zach Davis)          */
/*      - 0.99.5 : Make package buildable on pure *BSD by changing the  */
/*                 include to asm/termios.h by sys/ttycom.h             */
/*                 Print message without converting it into the locale  */
/*                 in show_message()                                    */
/*                 Set backspace key binding to backspace so that the   */
/*                 backspace works. It would even be nicer if the       */
/*                 behaviour of this key could be configured !          */
/*      - 0.99.4 : - Sebastien Bacher -                                 */
/*                 Added functions for CR LF auto mode                  */
/*                 Fixed put_text() to have \r\n for the VTE Widget     */
/*                 Rewritten put_hexadecimal() function                 */
/*                 - Julien -                                           */
/*                 Modified send_serial to return the actual number of  */
/*                 bytes written, and also only display exactly what    */
/*                 is written                                           */
/*      - 0.99.3 : Modified to use a VTE terminal                       */
/*      - 0.99.2 : Internationalization                                 */
/*      - 0.99.0 : \b byte now handled correctly by the ascii widget    */
/*                 SUPPR (0x7F) also prints correctly                   */
/*                 adapted for macros                                   */
/*                 modified "about" dialog                              */
/*      - 0.98.6 : fixed possible buffer overrun in hex send            */
/*                 new "Send break" option                              */
/*      - 0.98.5 : icons in the menu                                    */
/*                 bug fixed with local echo and hexadecimal            */
/*                 modified hexadecimal send separator, and bug fixed   */
/*      - 0.98.4 : new hexadecimal display / send                       */
/*      - 0.98.3 : put_text() modified to fit with 0x0D 0x0A            */
/*      - 0.98.2 : added local echo by Julien                           */
/*      - 0.98 : file creation by Julien                                */
/*                                                                      */
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

#include <stdio.h>
#include <stdbool.h>
#include <sys/stat.h>
#include <glib.h>
#include <glib/gi18n.h>
#include <glib/gprintf.h>
#include <glib-object.h>
#include <gtk/gtk.h>
#include <gio/gio.h>
#include <pango/pango-font.h>

#include "gtkterm_defaults.h"
#include "gtkterm_window.h"
#include "gtkterm_serial_port.h"
#include "gtkterm_terminal.h"
#include "gtkterm_buffer.h"
#include "macros.h"
#include "gtkterm_configuration.h"

typedef struct  {
    GtkTermBuffer *term_buffer;     /**< Terminal buffer for serial port and terminal                       */
    GtkTermSerialPort *serial_port; /**< The active serial port for this terminal                           */
    term_config_t *term_conf;       /**< The configuration loaded from the keyfile                          */
    port_config_t *port_conf;       /**< Port configuration used in this terminal                           */
    macro_t       *macros;          /**< \todo convert macros -> object                                     */
    GActionGroup *action_group;     /**< Terminal action group                                              */
 
    char *section;           		/**< Section used in this terminal for configuration from config file   */
	GtkTerm *app;                   /**< Pointer to the app for getting [section] and keyfile               */
    GtkTermWindow *main_window;     /**< Pointer to the main window for updating the statusbar on changes   */

} GtkTermTerminalPrivate;

struct _GtkTermTerminal {

    VteTerminal vte_object;         /**< The actual terminal object                                         */
};

struct _GtkTermTerminalClass {

    VteTerminalClass vte_class; 
};

G_DEFINE_TYPE_WITH_PRIVATE  (GtkTermTerminal, gtkterm_terminal, VTE_TYPE_TERMINAL)

enum { 
	PROP_0, 
	PROP_SECTION,
	PROP_GTKTERM_APP,
    PROP_MAIN_WINDOW,
	N_PROPS 
};

static GParamSpec *gtkterm_terminal_properties[N_PROPS] = {NULL};


void gtkterm_terminal_view_ascii (GtkTermTerminal *, char *, uint);
void gtkterm_terminal_view_hex (GtkTermTerminal *, char *, uint);
void gtkterm_terminal_view_binary (GtkTermTerminal *, char *, uint);
void gtkterm_terminal_view_decimal (GtkTermTerminal *, char *, uint);

/**
 * @brief Create a new terminal object.
 * 
 * This also binds the parameter to the properties of the terminal.
 * 
 * @param section The section for the configuration in this terminal
 * 
 * @param gtkterm_app The GTKTerm application
 * 
 * @param main_window The main_window this terminal is attached to.
 * 
 * @return The terminal object.
 * 
 */
GtkTermTerminal *gtkterm_terminal_new (char *section, GtkTerm *gtkterm_app, GtkTermWindow *main_window) {

    return g_object_new (GTKTERM_TYPE_TERMINAL, "section", section, "app", gtkterm_app, "main_window", main_window, NULL);
}

/**
 * @brief Callback when new data from the VTE widget is received.
 * 
 * If echo is enabled the string is also send to the buffer. Which will update
 * the terminal by sending the new data signal.
 * 
 * @param widget The VTE widget.
 * 
 * @param text The text which is entered
 * 
 * @param length The length of the text
 * 
 * @param ptr Not used.
 * 
 */
static void gtkterm_terminal_vte_data_received (VteTerminal *widget, char *text, unsigned int length, gpointer ptr) {
    GtkTermTerminal *self = GTKTERM_TERMINAL(ptr);
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    int chars_transmitted = 0; 

    g_signal_emit(priv->serial_port, gtkterm_signals[SIGNAL_GTKTERM_SERIAL_DATA_TRANSMIT], 0, text, length, &chars_transmitted);

    /** On echo send data to the buffer which will update the terminal */
    if (chars_transmitted > 0 && priv->term_conf->echo) {
        /** Convert to GBytes, needed for the buffer */
        GBytes *data = g_bytes_new (text, chars_transmitted);
	    
        /** Send signal to buffer when new data is arrived */
        g_signal_emit (self, gtkterm_signals[SIGNAL_GTKTERM_VTE_DATA_RECEIVED], 0, data);
    }
}

/**
 * @brief When signalsof the serial port is changed we get a signal and have to update GtkTermWindow.
 * 
 * @param object The serial port.
 * 
 * @param pspec Not used.
 * 
 * @param user_data The active terminal.
 * 
 */
static void gtkterm_terminal_port_signals_changed (GObject *object, GParamSpec *pspec, gpointer user_data) {
    GtkTermTerminal *self = GTKTERM_TERMINAL(user_data);
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);

    g_signal_emit (priv->main_window, gtkterm_signals[SIGNAL_GTKTERM_SERIAL_SIGNALS_CHANGED], 0, 
                   gtkterm_serial_port_get_signals (GTKTERM_SERIAL_PORT (object)));  
}

/**
 * @brief When the status of the serial port is changed we get a signal and have to update GtkTermWindow.
 * 
 * @param object The serial port.
 * 
 * @param pspec Not used.
 * 
 * @param user_data The active terminal.
 * 
 */
static void gtkterm_terminal_port_status_changed (GObject *object, GParamSpec *pspec, gpointer user_data) {
    char *serial_string;
    GError *error;
    GtkTermTerminal *self = GTKTERM_TERMINAL(user_data);
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    GtkTermSerialPortState status = gtkterm_serial_port_get_status (GTKTERM_SERIAL_PORT (object));

    if (status == GTKTERM_SERIAL_PORT_ERROR) {
        char *error_string;
        error = gtkterm_serial_port_get_error (GTKTERM_SERIAL_PORT (object));

        if (error != NULL) 
            error_string = g_strdup_printf (_("Serial port error : %s"), error->message);
        else
            error_string = g_strdup (_("Serial port error : Unknown error"));

        /** \todo convert to notify signal on message... */
        gtkterm_show_infobar (priv->main_window, error_string, GTK_MESSAGE_ERROR); 

        g_free(error_string);
    }

	/** Update the statusbar and main window title */
    serial_string = gtkterm_serial_port_get_string (priv->serial_port);
    g_signal_emit (priv->main_window, gtkterm_signals[SIGNAL_GTKTERM_TERMINAL_CHANGED], 
                                      0, 
                                      priv->section, 
                                      serial_string, 
                                      GtkTermSerialPortStateString[status]);    
    g_free(serial_string);
}

/**
 * @brief When the buffer is updated the terminal is notified data new data is available.
 * 
 * Depending of the setting the output will be in ASCII for HEX.
 * 
 * @param object Not used.
 * 
 * @param data The new string of data in the buffer.
 * 
 * @param length The length of the new string
 * 
 * @param user_data The terminal.
 * 
 */
 static void gtkterm_terminal_buffer_updated (GObject *object, gpointer data, unsigned int length, gpointer user_data) {
     GtkTermTerminal *self = GTKTERM_TERMINAL(user_data);
     GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);

    g_print("DEBUG: [TERMINAL] gtkterm_terminal_buffer_updated() - view_mode=%d, length=%u\n",
            priv->term_conf->view_mode, length);

    /* Route data to the appropriate view function based on current view mode */
    switch (priv->term_conf->view_mode) {
        case GTKTERM_TERMINAL_VIEW_ASCII:
            g_print("DEBUG: [TERMINAL] Writing to VTE in ASCII mode\n");
            gtkterm_terminal_view_ascii (self, data, length);
            break;
        case GTKTERM_TERMINAL_VIEW_HEX:
        case GTKTERM_TERMINAL_VIEW_MIXED:
            g_print("DEBUG: [TERMINAL] Writing to VTE in HEX/MIXED mode\n");
            gtkterm_terminal_view_hex (self, data, length);
            break;
        case GTKTERM_TERMINAL_VIEW_BINARY:
        case GTKTERM_TERMINAL_VIEW_BINARY_MIXED:
            g_print("DEBUG: [TERMINAL] Writing to VTE in BINARY mode\n");
            gtkterm_terminal_view_binary (self, data, length);
            break;
        case GTKTERM_TERMINAL_VIEW_DECIMAL:
        case GTKTERM_TERMINAL_VIEW_DECIMAL_MIXED:
            g_print("DEBUG: [TERMINAL] Writing to VTE in DECIMAL mode\n");
            gtkterm_terminal_view_decimal (self, data, length);
            break;
        default:
            g_print("DEBUG: [TERMINAL] Unknown view mode: %d\n", priv->term_conf->view_mode);
            gtkterm_terminal_view_ascii (self, data, length);
            break;
    }
}

/**
 * @brief Constructs the terminal.
 * 
 * Setup signals, initialize terminal etc.
 * 
 * @param object The terminal object we are constructing.
 * 
 */
static void gtkterm_terminal_constructed (GObject *object) {
    GError *error;
    GtkTermTerminal *self = GTKTERM_TERMINAL(object);
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);

    /** Check if the config file exists, if not it will be created */
    g_signal_emit(priv->app->config, gtkterm_signals[SIGNAL_GTKTERM_CONFIG_CHECK_FILE], 0);

    if ((gtkterm_configuration_get_status(priv->app->config)) != GTKTERM_CONFIGURATION_SUCCESS) {
        error = gtkterm_configuration_get_error (priv->app->config);
        /** \todo: convert to notify on message */        
	    gtkterm_show_infobar (priv->main_window, error->message, GTK_MESSAGE_INFO); 
    }

	/** 
     * Load the configuration from [Section] into the port and terminal config.
     * Take [section] as input, term/port conf are the pointers to the return values.
     */
    g_signal_emit(priv->app->config, gtkterm_signals[SIGNAL_GTKTERM_CONFIG_TERMINAL], 0, priv->section, &priv->term_conf);
    g_signal_emit(priv->app->config, gtkterm_signals[SIGNAL_GTKTERM_CONFIG_SERIAL], 0, priv->section, &priv->port_conf);
    
    /** 
     * Create the serial port. The buffer will be a propertie, so they can exchange data without
     * interference of the terminal.
     */
    priv->serial_port = gtkterm_serial_port_new (priv->port_conf);

    /** Create a buffer for the terminal */
    priv->term_buffer = gtkterm_buffer_new (priv->serial_port, self, priv->term_conf);

    g_signal_connect (G_OBJECT(priv->serial_port), "notify::port-status", G_CALLBACK(gtkterm_terminal_port_status_changed), self);
    g_signal_connect (G_OBJECT(priv->serial_port), "notify::port-signals", G_CALLBACK(gtkterm_terminal_port_signals_changed), self);
    g_signal_connect (G_OBJECT(priv->term_buffer), "buffer-updated", G_CALLBACK(gtkterm_terminal_buffer_updated), self);    

    /** Send initial notify to update the status bar */
	g_object_notify(G_OBJECT(priv->serial_port), "port-status");		

  	/**  
     * Set terminal properties.
     * \todo: make configurable from the config file
     */
	vte_terminal_set_scroll_on_output(VTE_TERMINAL(self), FALSE);
	vte_terminal_set_scroll_on_keystroke(VTE_TERMINAL(self), TRUE);
	vte_terminal_set_mouse_autohide(VTE_TERMINAL(self), TRUE);
	vte_terminal_set_backspace_binding(VTE_TERMINAL(self), VTE_ERASE_ASCII_BACKSPACE);

    vte_terminal_set_cursor_shape (VTE_TERMINAL(self), priv->term_conf->block_cursor ? VTE_CURSOR_SHAPE_BLOCK : VTE_CURSOR_SHAPE_IBEAM);
    vte_terminal_set_font (VTE_TERMINAL (self), priv->term_conf->font);	
    vte_terminal_set_scrollback_lines (VTE_TERMINAL (self), priv->term_conf->scrollback);
    vte_terminal_set_audible_bell (VTE_TERMINAL (self), priv->term_conf->visual_bell);
    vte_terminal_set_color_background (VTE_TERMINAL (self), &priv->term_conf->background_color);
    vte_terminal_set_color_foreground (VTE_TERMINAL (self), &priv->term_conf->foreground_color);
    vte_terminal_set_size(VTE_TERMINAL (self), priv->term_conf->columns, priv->term_conf->rows);	

    G_OBJECT_CLASS (gtkterm_terminal_parent_class)->constructed (object);
}

/**
 * @brief Called when distroying the terminal
 * 
 * This is used to clean up an freeing the variables in the terminal structure.
 * 
 * @param object The object.
 * 
 */
static void gtkterm_terminal_dispose (GObject *object) {
    GtkTermTerminal *self = GTKTERM_TERMINAL(object);
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);	
	
    g_free(priv->section);
    g_free(priv->term_conf);
    g_free(priv->port_conf);
}

/**
 * @brief Set the property of the GtkTermTerminal structure
 * 
 * This is used to initialize the variables when creating a new terminal
 * 
 * @param object The object.
 * 
 * @param prop_id The id of the property to set.
 * 
 * @param value The value for the property
 * 
 * @param pspec Metadata for property setting.
 * 
 */
static void gtkterm_terminal_set_property (GObject *object,
                             unsigned int prop_id,
                             const GValue *value,
                             GParamSpec *pspec)
{
    GtkTermTerminal *self = GTKTERM_TERMINAL(object);
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);

    switch (prop_id) {
    	case PROP_SECTION:
        	priv->section = g_value_dup_string (value);
        	break;

    	case PROP_GTKTERM_APP:
        	priv->app = g_value_dup_object (value); 
        	break;

    	case PROP_MAIN_WINDOW:
        	priv->main_window = g_value_dup_object (value); 
        	break;			


    	default:
        	G_OBJECT_WARN_INVALID_PROPERTY_ID (object, prop_id, pspec);
    }
}

/**
 * @brief Initializing the terminal class
 * 
 * Setting the properties and callback functions
 * 
 * @param class The terminal class
 * 
 */
static void gtkterm_terminal_class_init (GtkTermTerminalClass *class) {

  	GObjectClass *object_class = G_OBJECT_CLASS (class);
    object_class->set_property = gtkterm_terminal_set_property;
    object_class->constructed = gtkterm_terminal_constructed;
  	object_class->dispose = gtkterm_terminal_dispose;	

    /** Connect the serial port */
  	gtkterm_signals[SIGNAL_GTKTERM_SERIAL_CONNECT] = g_signal_new ("serial-connect",
                                                GTKTERM_TYPE_SERIAL_PORT,
                                                G_SIGNAL_RUN_FIRST,
                                                0,
                                                NULL,
                                                NULL,
                                                NULL,
                                                G_TYPE_NONE,
                                                0,
                                                NULL); 

    /** We received data from the VTE widget and send it to the serial port */
  	gtkterm_signals[SIGNAL_GTKTERM_SERIAL_DATA_TRANSMIT] = g_signal_new ("serial-data-transmit",
                                                GTKTERM_TYPE_SERIAL_PORT,
                                                G_SIGNAL_RUN_FIRST,
                                                0,
                                                NULL,
                                                NULL,
                                                NULL,
                                                G_TYPE_INT,
                                                2,
                                                G_TYPE_POINTER,
                                                G_TYPE_INT,
                                                NULL);                                                 

    gtkterm_signals[SIGNAL_GTKTERM_VTE_DATA_RECEIVED] = g_signal_new ("vte-data-received",
                                                   GTKTERM_TYPE_TERMINAL,
                                                   G_SIGNAL_RUN_FIRST,
                                                   0,
                                                   NULL,
                                                   NULL,
                                                   NULL,
                                                   G_TYPE_NONE,
                                                   1,
                                                   G_TYPE_BYTES,
                                                   0);

	/** 
     * Parameters to hand over at creation of the object
	 * We need the section to load the config from the keyfile.
     */
  	gtkterm_terminal_properties[PROP_SECTION] = g_param_spec_string (
        "section",
        "section",
        "section",
        NULL,
        G_PARAM_WRITABLE | G_PARAM_CONSTRUCT_ONLY);

  	gtkterm_terminal_properties[PROP_GTKTERM_APP] = g_param_spec_object (
        "app",
        "app",
        "app",
        GTKTERM_TYPE_APP,
        G_PARAM_WRITABLE | G_PARAM_CONSTRUCT_ONLY);

  	gtkterm_terminal_properties[PROP_MAIN_WINDOW] = g_param_spec_object (
        "main_window",
        "main_window",
        "main_window",
        GTKTERM_TYPE_GTKTERM_WINDOW,
        G_PARAM_WRITABLE | G_PARAM_CONSTRUCT_ONLY);		        

    g_object_class_install_properties (object_class, N_PROPS, gtkterm_terminal_properties);
}

/**
 * @brief Initialize the terminal with the actions, etc.
 * 
 * @param self The terminal we are initializing.
 * 
 */
/**
 * @brief Wrapper to copy text to clipboard
 */
static void terminal_copy_callback (GtkWidget *button, gpointer user_data) {
    VteTerminal *terminal = VTE_TERMINAL(user_data);
    vte_terminal_copy_clipboard_format(terminal, VTE_FORMAT_TEXT);
}

/**
 * @brief Callback for right-click on terminal to show context menu
 */
static void on_terminal_right_click (GtkGestureClick *gesture,
                                      int n_press,
                                      double x,
                                      double y,
                                      gpointer user_data) {
    GtkTermTerminal *self = GTKTERM_TERMINAL(user_data);

    if (n_press != 1) return;

    /* Create popover for context menu */
    GtkWidget *popover = gtk_popover_new ();
    gtk_widget_set_parent (popover, GTK_WIDGET(self));

    /* Position the popover at the click location */
    GdkRectangle rect = { (int)x, (int)y, 1, 1 };
    gtk_popover_set_pointing_to (GTK_POPOVER(popover), &rect);

    /* Create menu box */
    GtkWidget *box = gtk_box_new (GTK_ORIENTATION_VERTICAL, 0);

    /* Add Copy button */
    GtkWidget *copy_btn = gtk_button_new_with_label ("Copy");
    gtk_widget_add_css_class (copy_btn, "flat");
    g_signal_connect (copy_btn, "clicked",
                      G_CALLBACK(terminal_copy_callback),
                      self);
    g_signal_connect_swapped (copy_btn, "clicked",
                              G_CALLBACK(gtk_popover_popdown),
                              popover);
    gtk_box_append (GTK_BOX(box), copy_btn);

    /* Add Paste button */
    GtkWidget *paste_btn = gtk_button_new_with_label ("Paste");
    gtk_widget_add_css_class (paste_btn, "flat");
    g_signal_connect_swapped (paste_btn, "clicked",
                              G_CALLBACK(vte_terminal_paste_clipboard),
                              self);
    g_signal_connect_swapped (paste_btn, "clicked",
                              G_CALLBACK(gtk_popover_popdown),
                              popover);
    gtk_box_append (GTK_BOX(box), paste_btn);

    /* Add Select All button */
    GtkWidget *select_all_btn = gtk_button_new_with_label ("Select All");
    gtk_widget_add_css_class (select_all_btn, "flat");
    g_signal_connect_swapped (select_all_btn, "clicked",
                              G_CALLBACK(vte_terminal_select_all),
                              self);
    g_signal_connect_swapped (select_all_btn, "clicked",
                              G_CALLBACK(gtk_popover_popdown),
                              popover);
    gtk_box_append (GTK_BOX(box), select_all_btn);

    gtk_popover_set_child (GTK_POPOVER(popover), box);
    gtk_popover_popup (GTK_POPOVER(popover));
}

static void gtkterm_terminal_init (GtkTermTerminal *self) {
    /** Set the menu actions for this terminal */

    g_signal_connect_after (G_OBJECT (self), "commit", G_CALLBACK (gtkterm_terminal_vte_data_received), self);

    /* Add right-click context menu */
    GtkGesture *right_click = gtk_gesture_click_new ();
    gtk_gesture_single_set_button (GTK_GESTURE_SINGLE(right_click), GDK_BUTTON_SECONDARY);
    g_signal_connect (right_click, "pressed", G_CALLBACK(on_terminal_right_click), self);
    gtk_widget_add_controller (GTK_WIDGET(self), GTK_EVENT_CONTROLLER(right_click));

}

/**
 * @brief Outputs a string to the terminal widget in ASCII.
 * 
 * @param self The terminal.
 * 
 * @param data The string of data we want to show
 * 
 * @param length The length of the string
 * 
 */
void gtkterm_terminal_view_ascii (GtkTermTerminal *self, char *data, unsigned int length) {

    vte_terminal_feed (VTE_TERMINAL(self), data, length);
}

/**
 * @brief Outputs a string to the terminal widget in HEX layout.
 *
 * Displays data in hexadecimal format with optional index and ASCII representation.
 * Format: [index:] HEX HEX HEX ...  ASCII
 *
 * @param self The terminal.
 *
 * @param data The string of data we want to show
 *
 * @param length The length of the string
 *
 */
void gtkterm_terminal_view_hex (GtkTermTerminal *self, char *data, unsigned int length) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    static unsigned int byte_index = 0;
    static unsigned int column_position = 0;
    static unsigned char line_buffer[128]; // Buffer to store current line's bytes
    GString *output = g_string_sized_new(length * 5); // Estimate: each byte needs ~5 chars

    bool show_index = priv->term_conf->show_index;
    bool is_mixed_mode = (priv->term_conf->view_mode == GTKTERM_TERMINAL_VIEW_MIXED);

    int hex_chars;
    if (is_mixed_mode) {
        // Calculate dynamic bytes per line based on terminal width
        // Formula: terminal_width = hex_part + separator + ascii_part + index
        // hex_part = bytes * 3 (e.g., "5B ")
        // separator = 3 (" | ")
        // ascii_part = bytes * 1
        // index = 10 if shown ("00000000: ")
        // Total: bytes * 4 + 3 + (index ? 10 : 0) = terminal_width

        glong terminal_width = vte_terminal_get_column_count(VTE_TERMINAL(self));
        int index_width = show_index ? 10 : 0;
        int available_width = terminal_width - 3 - index_width; // Subtract separator and index
        hex_chars = available_width / 4; // Each byte needs 4 chars total (3 hex + 1 ascii)

        // Clamp to reasonable limits
        if (hex_chars < 8) hex_chars = 8;
        if (hex_chars > 64) hex_chars = 64;
    } else {
        // Hex-only mode: use configured value
        hex_chars = priv->term_conf->hex_chars;
    }

    for (unsigned int i = 0; i < length; i++) {
        unsigned char byte = (unsigned char)data[i];

        // Print index at start of line
        if (column_position == 0 && show_index) {
            g_string_append_printf(output, "%08X: ", byte_index);
        }

        // Store byte in line buffer
        line_buffer[column_position] = byte;

        // Print hex byte
        g_string_append_printf(output, "%02X ", byte);

        column_position++;
        byte_index++;

        // Check if this is a line break (LF after CR, or just LF)
        bool is_line_break = (byte == 0x0A);

        // Force output at line break or when buffer is full
        if (is_line_break || column_position >= hex_chars) {
            // Pad with spaces if line is not full (for mixed mode alignment)
            if (is_mixed_mode && column_position < hex_chars) {
                for (int j = column_position; j < hex_chars; j++) {
                    g_string_append(output, "   "); // 3 spaces per missing byte
                }
            }

            // Add separator and ASCII representation for mixed mode
            if (is_mixed_mode) {
                g_string_append(output, "| ");

                // Print ASCII chars from line buffer
                for (int j = 0; j < column_position; j++) {
                    unsigned char ch = line_buffer[j];
                    if (ch >= 0x20 && ch <= 0x7E) {
                        g_string_append_c(output, ch);
                    } else {
                        g_string_append_c(output, '.');
                    }
                }
            }

            g_string_append(output, "\r\n");

            // Add extra blank line after line break for readability
            if (is_line_break) {
                g_string_append(output, "\r\n");
            }

            column_position = 0;
        }
    }

    // Feed the formatted output to VTE terminal
    if (output->len > 0) {
        vte_terminal_feed(VTE_TERMINAL(self), output->str, output->len);
    }

    g_string_free(output, TRUE);
}

/**
 * @brief Outputs a string to the terminal widget in BINARY layout.
 *
 * Displays data in binary format (8 bits per byte) with optional ASCII representation.
 * Format: [index:] BBBBBBBB BBBBBBBB ...  ASCII
 *
 * @param self The terminal.
 *
 * @param data The string of data we want to show
 *
 * @param length The length of the string
 *
 */
void gtkterm_terminal_view_binary (GtkTermTerminal *self, char *data, unsigned int length) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    static unsigned int byte_index = 0;
    static unsigned int column_position = 0;
    static unsigned char line_buffer[128]; // Buffer to store current line's bytes
    GString *output = g_string_sized_new(length * 10); // Each byte needs ~10 chars (8 bits + space)

    bool show_index = priv->term_conf->show_index;
    bool is_mixed_mode = (priv->term_conf->view_mode == GTKTERM_TERMINAL_VIEW_BINARY_MIXED);

    int bytes_per_line;
    if (is_mixed_mode) {
        // Calculate dynamic bytes per line based on terminal width for mixed mode
        // Formula: bytes * 9 (binary "01010101 ") + 3 (" | ") + bytes * 1 (ascii) + index
        glong terminal_width = vte_terminal_get_column_count(VTE_TERMINAL(self));
        int index_width = show_index ? 10 : 0;
        int available_width = terminal_width - 3 - index_width;
        bytes_per_line = available_width / 10; // Each byte needs 10 chars total (9 binary + 1 ascii)

        // Clamp to reasonable limits
        if (bytes_per_line < 4) bytes_per_line = 4;
        if (bytes_per_line > 32) bytes_per_line = 32;
    } else {
        // Binary-only mode: use terminal width for binary only
        glong terminal_width = vte_terminal_get_column_count(VTE_TERMINAL(self));
        int index_width = show_index ? 10 : 0;
        int available_width = terminal_width - index_width;
        bytes_per_line = available_width / 9; // Each byte needs 9 chars (8 bits + space)

        // Clamp to reasonable limits
        if (bytes_per_line < 4) bytes_per_line = 4;
        if (bytes_per_line > 64) bytes_per_line = 64;
    }

    for (unsigned int i = 0; i < length; i++) {
        unsigned char byte = (unsigned char)data[i];

        // Print index at start of line
        if (column_position == 0 && show_index) {
            g_string_append_printf(output, "%08X: ", byte_index);
        }

        // Store byte in line buffer
        line_buffer[column_position] = byte;

        // Print binary byte (8 bits)
        for (int bit = 7; bit >= 0; bit--) {
            g_string_append_c(output, (byte & (1 << bit)) ? '1' : '0');
        }
        g_string_append_c(output, ' ');

        column_position++;
        byte_index++;

        // Check for line break
        bool is_line_break = (byte == 0x0A);

        // Force output at line break or when buffer is full
        if (is_line_break || column_position >= bytes_per_line) {
            // Add separator and ASCII representation for mixed mode only
            if (is_mixed_mode) {
                // Pad with spaces if line is not full
                if (column_position < bytes_per_line) {
                    for (int j = column_position; j < bytes_per_line; j++) {
                        g_string_append(output, "         "); // 9 spaces per missing byte (8 bits + space)
                    }
                }

                g_string_append(output, "| ");

                // Print ASCII chars from line buffer
                for (int j = 0; j < column_position; j++) {
                    unsigned char ch = line_buffer[j];
                    if (ch >= 0x20 && ch <= 0x7E) {
                        g_string_append_c(output, ch);
                    } else {
                        g_string_append_c(output, '.');
                    }
                }
            }

            g_string_append(output, "\r\n");

            // Add extra blank line after line break
            if (is_line_break) {
                g_string_append(output, "\r\n");
            }

            column_position = 0;
        }
    }

    // Feed the formatted output to VTE terminal
    if (output->len > 0) {
        vte_terminal_feed(VTE_TERMINAL(self), output->str, output->len);
    }

    g_string_free(output, TRUE);
}

/**
 * @brief Outputs a string to the terminal widget in DECIMAL layout.
 *
 * Displays data in decimal format with optional ASCII representation.
 * Format: [index:] DDD DDD DDD ...  ASCII
 *
 * @param self The terminal.
 *
 * @param data The string of data we want to show
 *
 * @param length The length of the string
 *
 */
void gtkterm_terminal_view_decimal (GtkTermTerminal *self, char *data, unsigned int length) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    static unsigned int byte_index = 0;
    static unsigned int column_position = 0;
    static unsigned char line_buffer[128]; // Buffer to store current line's bytes
    GString *output = g_string_sized_new(length * 5); // Each byte needs ~5 chars (3 digits + space)

    bool show_index = priv->term_conf->show_index;
    bool is_mixed_mode = (priv->term_conf->view_mode == GTKTERM_TERMINAL_VIEW_DECIMAL_MIXED);

    int bytes_per_line;
    if (is_mixed_mode) {
        // Calculate dynamic bytes per line based on terminal width for mixed mode
        // Formula: bytes * 4 (decimal "255 ") + 3 (" | ") + bytes * 1 (ascii) + index
        glong terminal_width = vte_terminal_get_column_count(VTE_TERMINAL(self));
        int index_width = show_index ? 10 : 0;
        int available_width = terminal_width - 3 - index_width;
        bytes_per_line = available_width / 5; // Each byte needs 5 chars total (4 decimal + 1 ascii)

        // Clamp to reasonable limits
        if (bytes_per_line < 8) bytes_per_line = 8;
        if (bytes_per_line > 64) bytes_per_line = 64;
    } else {
        // Decimal-only mode: use terminal width for decimal only
        glong terminal_width = vte_terminal_get_column_count(VTE_TERMINAL(self));
        int index_width = show_index ? 10 : 0;
        int available_width = terminal_width - index_width;
        bytes_per_line = available_width / 4; // Each byte needs 4 chars (3 digits + space)

        // Clamp to reasonable limits
        if (bytes_per_line < 8) bytes_per_line = 8;
        if (bytes_per_line > 80) bytes_per_line = 80;
    }

    for (unsigned int i = 0; i < length; i++) {
        unsigned char byte = (unsigned char)data[i];

        // Print index at start of line
        if (column_position == 0 && show_index) {
            g_string_append_printf(output, "%08X: ", byte_index);
        }

        // Store byte in line buffer
        line_buffer[column_position] = byte;

        // Print decimal byte (0-255)
        g_string_append_printf(output, "%3d ", byte);

        column_position++;
        byte_index++;

        // Check for line break
        bool is_line_break = (byte == 0x0A);

        // Force output at line break or when buffer is full
        if (is_line_break || column_position >= bytes_per_line) {
            // Add separator and ASCII representation for mixed mode only
            if (is_mixed_mode) {
                // Pad with spaces if line is not full
                if (column_position < bytes_per_line) {
                    for (int j = column_position; j < bytes_per_line; j++) {
                        g_string_append(output, "    "); // 4 spaces per missing byte
                    }
                }

                g_string_append(output, "| ");

                // Print ASCII chars from line buffer
                for (int j = 0; j < column_position; j++) {
                    unsigned char ch = line_buffer[j];
                    if (ch >= 0x20 && ch <= 0x7E) {
                        g_string_append_c(output, ch);
                    } else {
                        g_string_append_c(output, '.');
                    }
                }
            }

            g_string_append(output, "\r\n");

            // Add extra blank line after line break
            if (is_line_break) {
                g_string_append(output, "\r\n");
            }

            column_position = 0;
        }
    }

    // Feed the formatted output to VTE terminal
    if (output->len > 0) {
        vte_terminal_feed(VTE_TERMINAL(self), output->str, output->len);
    }

    g_string_free(output, TRUE);
}

/**
 * Public accessor functions for terminal configuration
 */

void gtkterm_terminal_set_view_mode (GtkTermTerminal *self, GtkTermTerminalView mode) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    priv->term_conf->view_mode = mode;
}

GtkTermTerminalView gtkterm_terminal_get_view_mode (GtkTermTerminal *self) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    return priv->term_conf->view_mode;
}

void gtkterm_terminal_set_echo (GtkTermTerminal *self, bool echo) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    priv->term_conf->echo = echo;
}

bool gtkterm_terminal_get_echo (GtkTermTerminal *self) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    return priv->term_conf->echo;
}

void gtkterm_terminal_set_auto_cr (GtkTermTerminal *self, bool auto_cr) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    priv->term_conf->auto_cr = auto_cr;
}

bool gtkterm_terminal_get_auto_cr (GtkTermTerminal *self) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    return priv->term_conf->auto_cr;
}

void gtkterm_terminal_set_auto_lf (GtkTermTerminal *self, bool auto_lf) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    priv->term_conf->auto_lf = auto_lf;
}

bool gtkterm_terminal_get_auto_lf (GtkTermTerminal *self) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    return priv->term_conf->auto_lf;
}

void gtkterm_terminal_set_timestamp (GtkTermTerminal *self, bool timestamp) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    priv->term_conf->timestamp = timestamp;
}

bool gtkterm_terminal_get_timestamp (GtkTermTerminal *self) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    return priv->term_conf->timestamp;
}

void gtkterm_terminal_set_hex_chars (GtkTermTerminal *self, int hex_chars) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    priv->term_conf->hex_chars = hex_chars;
}

void gtkterm_terminal_set_show_index (GtkTermTerminal *self, bool show_index) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    priv->term_conf->show_index = show_index;
}

GtkTermSerialPort *gtkterm_terminal_get_serial_port (GtkTermTerminal *self) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    return priv->serial_port;
}

/**
 * @brief Get the buffer from the terminal
 */
GtkTermBuffer *gtkterm_terminal_get_buffer (GtkTermTerminal *self) {
    GtkTermTerminalPrivate *priv = gtkterm_terminal_get_instance_private (self);
    return priv->term_buffer;
}
