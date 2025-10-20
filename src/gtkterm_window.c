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
/*      - 2.0     : Port to Gtk4                                        */
/*      - 0.99.3 : Modified to use a VTE terminal                       */
/*      - 0.99.2 : Internationalization                                 */
/*      - 0.98 : file creation by Julien                                */
/*                                                                      */
/* This GtkTerm is free software: you can redistribute it and/or modify	*/ 
/* it under the terms of the GNU  General Public License as published  	*/
/* by the Free Software Foundation, either version 3 of the License,   	*/
/* or (at your option) any later version.							   	              */
/*																	   	                                */
/* GtkTerm is distributed in the hope that it will be useful, but	   	  */
/* WITHOUT ANY WARRANTY; without even the implied warranty of 		   	  */
/* MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. 			   	      */
/* See the GNU General Public License for more details.					        */
/*																		                                  */
/* You should have received a copy of the GNU General Public License 	  */
/* along with GtkTerm If not, see <https://www.gnu.org/licenses/>. 		  */
/*                                                                     	*/
/************************************************************************/

#include <gtk/gtk.h>
#include <vte/vte.h>
#include <glib/gi18n.h>
#include <glib/gprintf.h>
#include <sys/ioctl.h>
#define PCRE2_CODE_UNIT_WIDTH 8
#include <pcre2.h>

#include "config.h"
#include "gtkterm_defaults.h"
#include "gtkterm.h"
#include "gtkterm_window.h"
#include "gtkterm_terminal.h"
#include "gtkterm_logging.h"
#include "gtkterm_macro.h"
#include "gtkterm_dataview.h"
#include "gtkterm_buffer.h"
#include "../include/sunshine/manager.h"

#define SERIAL_SIGNALS    6

/**
 * @brief MainWindow specific variables here.
 *
 */
struct _GtkTermWindow {
  GtkApplicationWindow parent_instance;

  GtkWidget *message;                   /**< Message for the infobar                */
  GtkWidget *infobar;                   /**< Infobar                                */
  GtkBox *statusbox;                    /**< Box for statusbar messages             */
  GtkBox *status_config;                /**< Displays the actual used configuration */
  GtkWidget *menubutton;                /**< Toolbar                                */
  GMenuModel *toolmenu;                 /**< Menu                                   */
  GtkScrolledWindow *scrolled_window;   /**< Make the terminal window scrolled      */
  GtkTermTerminal *terminal_window;     /**< The terminal window (VTE)              */
  GtkTermDataView *dataview;            /**< Custom data view widget                */
  GtkWidget *view_mode_dropdown;        /**< View mode selector dropdown            */
  GtkWidget *search_bar;                /**< Searchbar container                    */
  GtkWidget *search_revealer;           /**< Search revealer for animation          */
  GtkWidget *search_entry;              /**< Search text entry                      */
  GtkWidget *search_case_sensitive;     /**< Case sensitive checkbox                */
  GtkWidget *search_regex;              /**< Regex checkbox                         */
  GtkWidget *search_hex;                /**< Hex search checkbox                    */
  GtkWidget *search_prev;               /**< Previous button                        */
  GtkWidget *search_next;               /**< Next button                            */
  GtkWidget *search_close;              /**< Close button                           */
  GActionGroup *action_group;           /**< Window action group                    */
  GtkWidget *status_config_message[3];
  GtkWidget *status_serial_signal[SERIAL_SIGNALS];
  GtkWidget *status_message;
  GtkTermLogging *logging;              /**< Logging system                         */
  GtkTermMacroManager *macro_manager;   /**< Macro system                           */

  /* Header bar controls for port configuration */
  GtkWidget *header_bar;                /**< Header bar widget                      */
  GtkWidget *port_dropdown;             /**< Port selection dropdown                */
  GtkWidget *baud_dropdown;             /**< Baud rate selection dropdown           */
  GtkWidget *connect_button;            /**< Connect/Disconnect button              */
  GtkWidget *status_label;              /**< Connection status label                */

  /* Sunshine visualization panel */
  GtkWidget *main_paned;                /**< Split pane for terminal/visualization  */
  GtkWidget *visualization_panel;       /**< Notebook for multiple charts           */
  GtkTermSunshineManager *sunshine_manager;  /**< Sunshine visualization manager */
  bool visualization_visible;           /**< Whether visualization panel is shown   */

  int width;                            /**< Window width                           */
  int height;                           /**< Window height                          */
  bool maximized;                       /**< Window minimized?                      */
  bool fullscreen;                      /**< Window maximized?                      */
} ;

G_DEFINE_TYPE (GtkTermWindow, gtkterm_window, GTK_TYPE_APPLICATION_WINDOW)

/** Internal functions            */
static void gtkterm_window_update_statusbar (GtkTermWindow *, gpointer, gpointer, gpointer, gpointer);
static void config_status_bar (GtkTermWindow *);
static void update_statusbar (GtkTermWindow *, gpointer, gpointer, gpointer);
void set_window_title (GtkTermWindow *, gpointer);
static void gtkterm_window_update_actions (GtkTermWindow *, GtkTermSerialPortState);
static void on_port_status_changed (GObject *, GParamSpec *, gpointer);
static void gtkterm_window_update_ui_state (GtkTermWindow *);
static GPtrArray *gtkterm_window_scan_ports (void);
static void on_connect_button_clicked (GtkButton *, gpointer);
static void create_header_bar (GtkTermWindow *);
static void gtkterm_window_switch_view (GtkTermWindow *, GtkTermTerminalView);
static void on_view_mode_dropdown_changed (GObject *, GParamSpec *, gpointer);

/** Menu callbacks                */
static void on_gtkterm_about (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_toggle_state (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_toggle_dark (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_toggle_visualization (GSimpleAction *, GVariant *, gpointer);

/** File menu callbacks */
static void on_gtkterm_clear_screen (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_clear_scrollback (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_send_raw (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_save_raw (GSimpleAction *, GVariant *, gpointer);

/** Edit menu callbacks */
static void on_gtkterm_copy (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_paste (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_select_all (GSimpleAction *, GVariant *, gpointer);

/** Configuration menu callbacks */
static void on_gtkterm_config_port (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_config_main_window (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_local_echo (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_crlf_auto (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_timestamp (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_macros (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_load_config (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_save_config (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_remove_config (GSimpleAction *, GVariant *, gpointer);

/** Log menu callbacks */
static void on_gtkterm_log_to_file (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_log_resume (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_log_stop (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_log_clear (GSimpleAction *, GVariant *, gpointer);

/** Control signals menu callbacks */
static void on_gtkterm_send_break (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_open_port (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_close_port (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_toggle_DTR (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_toggle_RTS (GSimpleAction *, GVariant *, gpointer);

/** View menu callbacks */
static void on_gtkterm_show_ascii (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_hex (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_binary (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_decimal (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_mixed (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_binary_mixed (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_decimal_mixed (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_view_hex (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_index (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_timestamps (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_show_direction (GSimpleAction *, GVariant *, gpointer);
static void on_gtkterm_send_hex_data (GSimpleAction *, GVariant *, gpointer);

/** Search callbacks */
static void on_gtkterm_search_show (GSimpleAction *, GVariant *, gpointer);
static void on_search_entry_changed (GtkEntry *, gpointer);
static void on_search_next_clicked (GtkButton *, gpointer);
static void on_search_prev_clicked (GtkButton *, gpointer);
static void on_search_close_clicked (GtkButton *, gpointer);
static void perform_search (GtkTermWindow *, gboolean);

/** Serial signals                */
static unsigned int signal_flags[] = {TIOCM_DTR, TIOCM_RTS, TIOCM_CTS, TIOCM_CD, TIOCM_DSR, TIOCM_RI};
static char const *serial_signal[] = {"DTR", "RTS", "CTS", "CD", "DSR", "RI"};

/** Callback for profile button click */
static void on_profile_button_clicked (GtkButton *button, gpointer user_data);

/** Callback for signal button clicks */
static void on_signal_button_clicked (GtkButton *button, gpointer user_data);

/** Menu definitions and callbacks */
static GActionEntry gtkterm_window_entries[] = {
  { "clear_screen", on_gtkterm_clear_screen, NULL, NULL, NULL },
  { "clear_scrollback", on_gtkterm_clear_scrollback, NULL, NULL, NULL },
  { "send_raw_file", on_gtkterm_send_raw, NULL, NULL, NULL },
  { "save_raw_file", on_gtkterm_save_raw, NULL, NULL, NULL },
  { "copy", on_gtkterm_copy, NULL, NULL, NULL },
  { "paste", on_gtkterm_paste, NULL, NULL, NULL },
  { "select_all", on_gtkterm_select_all, NULL, NULL, NULL },
  { "log_to_file", on_gtkterm_log_to_file, NULL, NULL, NULL },
  { "log_resume", on_gtkterm_log_resume, NULL, NULL, NULL },
  { "log_stop", on_gtkterm_log_stop, NULL, NULL, NULL },
  { "log_clear", on_gtkterm_log_clear, NULL, NULL, NULL },
  { "config_port", on_gtkterm_config_port, NULL, NULL, NULL },
  { "config_main_window", on_gtkterm_config_main_window, NULL, NULL, NULL },
  { "local_echo", on_gtkterm_local_echo, NULL, NULL, NULL },
  { "crlf_auto", on_gtkterm_crlf_auto, NULL, NULL, NULL },
  { "timestamp", on_gtkterm_timestamp, NULL, NULL, NULL },
  { "macro", on_gtkterm_macros, NULL, NULL, NULL },
  { "load_config", on_gtkterm_load_config, NULL, NULL, NULL },
  { "save_config", on_gtkterm_save_config, NULL, NULL, NULL },
  { "remove_config", on_gtkterm_remove_config, NULL, NULL, NULL },
  { "send_break", on_gtkterm_send_break, NULL, NULL, NULL },
  { "open_port", on_gtkterm_open_port, NULL, NULL, NULL },
  { "close_port", on_gtkterm_close_port, NULL, NULL, NULL },
  { "toggle_DTR", on_gtkterm_toggle_DTR, NULL, NULL, NULL },
  { "toggle_RTS", on_gtkterm_toggle_RTS, NULL, NULL, NULL },
  { "show_ascii", on_gtkterm_show_ascii, NULL, NULL, NULL },
  { "show_hex", on_gtkterm_show_hex, NULL, NULL, NULL },
  { "show_binary", on_gtkterm_show_binary, NULL, NULL, NULL },
  { "show_decimal", on_gtkterm_show_decimal, NULL, NULL, NULL },
  { "show_mixed", on_gtkterm_show_mixed, NULL, NULL, NULL },
  { "show_binary_mixed", on_gtkterm_show_binary_mixed, NULL, NULL, NULL },
  { "show_decimal_mixed", on_gtkterm_show_decimal_mixed, NULL, NULL, NULL },
  { "view_hex", on_gtkterm_view_hex, NULL, NULL, NULL },
  { "show_index", on_gtkterm_show_index, NULL, NULL, NULL },
  { "show_timestamps", on_gtkterm_show_timestamps, NULL, NULL, NULL },
  { "show_direction", on_gtkterm_show_direction, NULL, NULL, NULL },
  { "send_hex_data", on_gtkterm_send_hex_data, NULL, NULL, NULL },
  { "search", on_gtkterm_search_show, NULL, NULL, NULL },
  { "dark", on_gtkterm_toggle_state, NULL, "false", on_gtkterm_toggle_dark},
  { "toggle_visualization", on_gtkterm_toggle_state, NULL, "false", on_gtkterm_toggle_visualization}
};

/** GtkTermWindow definitions and callbacks */
static GActionEntry win_entries[] = {
  { "about", on_gtkterm_about, NULL, NULL, NULL }
};

/** \todo remove and set it with properties. */
void create_window (GApplication *app, GtkTermWindow *window) {
  /**
   * Create a new terminal window and send section and keyfile as parameter
   * GTKTERM_TERMINAL then can load the right section.
   */
  window->terminal_window = gtkterm_terminal_new (GTKTERM_APP(app)->section, GTKTERM_APP(app), window);

  /**
   * NOTE: DataView widget removed - all view modes now use VTE terminal directly
   * No need to create or maintain the separate dataview widget anymore
   */
  window->dataview = NULL;

  /** Keep strong reference to terminal widget so it doesn't get destroyed */
  g_object_ref (window->terminal_window);

  /** Make the VTE window scrollable (default view) */
  gtk_scrolled_window_set_child(window->scrolled_window, GTK_WIDGET(window->terminal_window));

  /**
   * NOTE: DataView forwarding removed - all view modes now use VTE terminal directly
   * The buffer-updated signal is already handled internally by GtkTermTerminal
   * for all view modes (ASCII, HEX, BINARY, DECIMAL, MIXED)
   */

  /** Connect to port status changes to update menu actions and UI */
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port(window->terminal_window);
  if (serial_port) {
    g_signal_connect(serial_port, "notify::port-status", G_CALLBACK(on_port_status_changed), window);
    // Initialize actions and UI based on current port status
    GtkTermSerialPortState status = gtkterm_serial_port_get_status(serial_port);
    gtkterm_window_update_actions(window, status);
    gtkterm_window_update_ui_state(window);
  }
}

/**
 * NOTE: on_buffer_updated_for_dataview() removed
 *
 * This callback was previously used to forward data to the separate DataView widget,
 * but has been removed because all view modes now use the VTE terminal directly.
 * The GtkTermTerminal handles all view modes (ASCII, HEX, BINARY, DECIMAL, MIXED)
 * internally through its own buffer_updated callback.
 */

/**
 * @brief Set the view mode for the VTE terminal
 *
 * This function sets the view mode for the terminal display.
 * All view modes (ASCII, HEX, BINARY, DECIMAL, MIXED) now use the VTE terminal
 * with custom rendering handled internally by GtkTermTerminal.
 *
 * @param window The main window
 * @param view_mode The view mode to switch to
 */
static void gtkterm_window_switch_view (GtkTermWindow *window, GtkTermTerminalView view_mode) {
    if (!window) {
        return;
    }

    g_print("DEBUG: ========================================\n");
    g_print("DEBUG: gtkterm_window_switch_view() called - view_mode=%d\n", view_mode);

    /* Get the current child of the scrolled window */
    GtkWidget *current_child = gtk_scrolled_window_get_child (window->scrolled_window);
    GtkWidget *new_child = NULL;

    g_print("DEBUG: Current child: %p (terminal=%p)\n",
            (void*)current_child,
            (void*)window->terminal_window);

    /* All modes now use VTE terminal for consistent display framework */
    g_print("DEBUG: Switching to VTE terminal (mode=%d)\n", view_mode);
    new_child = GTK_WIDGET(window->terminal_window);
    if (current_child != new_child) {
        g_print("DEBUG: Child is changing - setting to VTE terminal\n");
        /* Unparent the old child first to avoid reference counting issues */
        if (current_child) {
            gtk_scrolled_window_set_child (window->scrolled_window, NULL);
        }
        /* Now set the new child */
        gtk_scrolled_window_set_child (window->scrolled_window, new_child);
    } else {
        g_print("DEBUG: Child already is VTE terminal - no widget switch needed\n");
    }
    gtkterm_terminal_set_view_mode (window->terminal_window, view_mode);

    /* Verify the child was actually set */
    GtkWidget *final_child = gtk_scrolled_window_get_child (window->scrolled_window);
    g_print("DEBUG: Final child after switch: %p\n", (void*)final_child);
    if (final_child == GTK_WIDGET(window->terminal_window)) {
        g_print("DEBUG: *** Final child is VTE TERMINAL ***\n");
    } else {
        g_print("DEBUG: *** WARNING: Final child is UNKNOWN! ***\n");
    }
    g_print("DEBUG: ========================================\n");
}

/**
 * @brief Callback for view mode dropdown selection change
 *
 * This is called when the user selects a different view mode from the dropdown
 * in the header bar.
 *
 * @param dropdown The GtkDropDown widget
 * @param pspec The parameter spec (not used)
 * @param user_data Pointer to the GtkTermWindow
 */
static void on_view_mode_dropdown_changed (GObject *dropdown, GParamSpec *pspec, gpointer user_data) {
    GtkTermWindow *window = GTKTERM_WINDOW (user_data);
    guint selected = gtk_drop_down_get_selected (GTK_DROP_DOWN (dropdown));

    GtkTermTerminalView view_mode;
    const char *info_message = NULL;

    switch (selected) {
        case 0: // ASCII
            view_mode = GTKTERM_TERMINAL_VIEW_ASCII;
            info_message = "Switched to ASCII view mode";
            break;
        case 1: // Hexadecimal
            view_mode = GTKTERM_TERMINAL_VIEW_HEX;
            info_message = "Switched to Hexadecimal view mode";
            break;
        case 2: // Binary
            view_mode = GTKTERM_TERMINAL_VIEW_BINARY;
            info_message = "Switched to Binary view mode";
            break;
        case 3: // Decimal
            view_mode = GTKTERM_TERMINAL_VIEW_DECIMAL;
            info_message = "Switched to Decimal view mode";
            break;
        case 4: // Mixed
            view_mode = GTKTERM_TERMINAL_VIEW_MIXED;
            info_message = "Switched to Mixed (Hex+ASCII) view mode";
            break;
        default:
            view_mode = GTKTERM_TERMINAL_VIEW_ASCII;
            info_message = "Switched to ASCII view mode (default)";
            break;
    }

    gtkterm_window_switch_view (window, view_mode);
    gtkterm_show_infobar (window, (char *)info_message, GTK_MESSAGE_INFO);
}

/**
 * @brief  Shows a message into the Infobar.
 *
 * @param window The window with the infobar property.
 *
 * @param message The message we want to show
 *
 * @param message_type The type of message GTK_MESSAGE_*
 *
 */
void gtkterm_show_infobar (GtkTermWindow *window, char *message, int message_type) {

  gtk_info_bar_set_message_type (GTK_INFO_BAR(window->infobar), message_type);
  gtk_label_set_text (GTK_LABEL (window->message), message);
  gtk_widget_show (window->infobar);
}

/** \todo rewrite for gtkterm */
static void open_response_cb (GtkNativeDialog *dialog,
                  int              response_id,
                  gpointer         user_data) {

  GtkFileChooserNative *native = user_data;
  GtkWidget *message_dialog;
  GFile *file;
  char *contents;
  GError *error = NULL;

  if (response_id == GTK_RESPONSE_ACCEPT)
    {
      file = gtk_file_chooser_get_file (GTK_FILE_CHOOSER (native));

      if (g_file_load_contents (file, NULL, &contents, NULL, NULL, &error))
        {
 //         create_window (app);
 //         g_free (contents);
        }
      else
        {
          message_dialog = gtk_message_dialog_new (NULL,
                                                   GTK_DIALOG_DESTROY_WITH_PARENT,
                                                   GTK_MESSAGE_ERROR,
                                                   GTK_BUTTONS_CLOSE,
                                                   "Error loading file: \"%s\"",
                                                   error->message);
          g_signal_connect (message_dialog, "response", G_CALLBACK (gtk_window_destroy), NULL);
          gtk_widget_show (message_dialog);
          g_error_free (error);
        }
    }

  gtk_native_dialog_destroy (GTK_NATIVE_DIALOG (native));
  g_object_unref (native);
}

/**
 * @brief Clear the terminal screen
 */
static void on_gtkterm_clear_screen (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  vte_terminal_reset (VTE_TERMINAL (window->terminal_window), TRUE, FALSE);
}

/**
 * @brief Clear the scrollback buffer
 */
static void on_gtkterm_clear_scrollback (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  vte_terminal_reset (VTE_TERMINAL (window->terminal_window), FALSE, TRUE);
}

/**
 * @brief Copy selected text to clipboard
 */
static void on_gtkterm_copy (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  vte_terminal_copy_clipboard_format (VTE_TERMINAL (window->terminal_window), VTE_FORMAT_TEXT);
}

/**
 * @brief Paste text from clipboard
 */
static void on_gtkterm_paste (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  vte_terminal_paste_clipboard (VTE_TERMINAL (window->terminal_window));
}

/**
 * @brief Select all text in the terminal
 */
static void on_gtkterm_select_all (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  vte_terminal_select_all (VTE_TERMINAL (window->terminal_window));
}

/** \todo rewrite for GTKTerm */
static void on_gtkterm_send_raw (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {

  GApplication *app = user_data;
  GtkFileChooserNative *native;

  native = gtk_file_chooser_native_new ("Open File",
                                        NULL,
                                        GTK_FILE_CHOOSER_ACTION_OPEN,
                                        "_Open",
                                        "_Cancel");

  g_object_set_data_full (G_OBJECT (native), "gtkterm", g_object_ref (app), g_object_unref);
  g_signal_connect (native, "response", G_CALLBACK (open_response_cb), native);

  gtk_native_dialog_show (GTK_NATIVE_DIALOG (native));
}

/**
 * @brief Save raw data from serial port to file
 */
static void on_gtkterm_save_raw (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  /** \todo Implement save raw file functionality */
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_show_infobar (window, "Save raw file not yet implemented", GTK_MESSAGE_INFO);
}

/**
 * @brief Toggle local echo
 */
static void on_gtkterm_local_echo (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  bool current = gtkterm_terminal_get_echo (window->terminal_window);
  gtkterm_terminal_set_echo (window->terminal_window, !current);

  char *msg = g_strdup_printf ("Local echo %s", !current ? "enabled" : "disabled");
  gtkterm_show_infobar (window, msg, GTK_MESSAGE_INFO);
  g_free (msg);
}

/**
 * @brief Toggle CR/LF auto mode
 */
static void on_gtkterm_crlf_auto (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  bool current_cr = gtkterm_terminal_get_auto_cr (window->terminal_window);
  bool current_lf = gtkterm_terminal_get_auto_lf (window->terminal_window);

  // Toggle both CR and LF auto
  gtkterm_terminal_set_auto_cr (window->terminal_window, !current_cr);
  gtkterm_terminal_set_auto_lf (window->terminal_window, !current_lf);

  char *msg = g_strdup_printf ("CR/LF auto %s", !current_cr ? "enabled" : "disabled");
  gtkterm_show_infobar (window, msg, GTK_MESSAGE_INFO);
  g_free (msg);
}

/**
 * @brief Toggle timestamp display
 */
static void on_gtkterm_timestamp (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  bool current = gtkterm_terminal_get_timestamp (window->terminal_window);
  gtkterm_terminal_set_timestamp (window->terminal_window, !current);

  char *msg = g_strdup_printf ("Timestamp %s", !current ? "enabled" : "disabled");
  gtkterm_show_infobar (window, msg, GTK_MESSAGE_INFO);
  g_free (msg);
}

/**
 * @brief Port configuration dialog response callback
 */
static void on_port_config_response (GtkDialog *dialog, int response_id, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  if (response_id == GTK_RESPONSE_OK) {
    // Get all the widgets
    GtkWidget *port_entry = g_object_get_data (G_OBJECT (dialog), "port_entry");
    GtkWidget *baudrate_combo = g_object_get_data (G_OBJECT (dialog), "baudrate_combo");
    GtkWidget *bits_combo = g_object_get_data (G_OBJECT (dialog), "bits_combo");
    GtkWidget *stopbits_combo = g_object_get_data (G_OBJECT (dialog), "stopbits_combo");
    GtkWidget *parity_combo = g_object_get_data (G_OBJECT (dialog), "parity_combo");
    GtkWidget *flow_combo = g_object_get_data (G_OBJECT (dialog), "flow_combo");

    // Get the serial port from the terminal
    GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);
    if (serial_port) {
      // Get the values from the dialog
      const char *port = gtk_editable_get_text (GTK_EDITABLE (port_entry));
      int baudrate_idx = gtk_drop_down_get_selected (GTK_DROP_DOWN (baudrate_combo));
      int bits_idx = gtk_drop_down_get_selected (GTK_DROP_DOWN (bits_combo));
      int stopbits_idx = gtk_drop_down_get_selected (GTK_DROP_DOWN (stopbits_combo));
      int parity_idx = gtk_drop_down_get_selected (GTK_DROP_DOWN (parity_combo));
      int flow_idx = gtk_drop_down_get_selected (GTK_DROP_DOWN (flow_combo));

      // Baudrate values
      int baudrates[] = {300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 500000, 576000, 921600, 1000000, 2000000};
      int baudrate = (baudrate_idx >= 0 && baudrate_idx < 17) ? baudrates[baudrate_idx] : 115200;

      // Create new configuration
      port_config_t new_config;
      new_config.port = g_strdup(port);
      new_config.baudrate = baudrate;
      new_config.bits = bits_idx + 5;  // 5, 6, 7, or 8 bits
      new_config.stopbits = stopbits_idx + 1;  // 1 or 2 stopbits

      // Parity: 0=None, 1=Odd, 2=Even
      switch (parity_idx) {
        case 0:
          new_config.parity = GTKTERM_SERIAL_PORT_PARITY_NONE;
          break;
        case 1:
          new_config.parity = GTKTERM_SERIAL_PORT_PARITY_ODD;
          break;
        case 2:
          new_config.parity = GTKTERM_SERIAL_PORT_PARITY_EVEN;
          break;
        default:
          new_config.parity = GTKTERM_SERIAL_PORT_PARITY_NONE;
      }

      // Flow control: 0=None, 1=XON/XOFF, 2=RTS/CTS, 3=RS485
      switch (flow_idx) {
        case 0:
          new_config.flow_control = GTKTERM_SERIAL_PORT_FLOWCONTROL_NONE;
          break;
        case 1:
          new_config.flow_control = GTKTERM_SERIAL_PORT_FLOWCONTROL_XON_XOFF;
          break;
        case 2:
          new_config.flow_control = GTKTERM_SERIAL_PORT_FLOWCONTROL_RTS_CTS;
          break;
        case 3:
          new_config.flow_control = GTKTERM_SERIAL_PORT_FLOWCONTROL_RS485_HD;
          break;
        default:
          new_config.flow_control = GTKTERM_SERIAL_PORT_FLOWCONTROL_NONE;
      }

      // Copy RS485 settings from current config
      port_config_t *current_config = gtkterm_serial_port_get_config(serial_port);
      new_config.rs485_rts_time_before_transmit = current_config->rs485_rts_time_before_transmit;
      new_config.rs485_rts_time_after_transmit = current_config->rs485_rts_time_after_transmit;
      new_config.disable_port_lock = current_config->disable_port_lock;

      // Apply the new configuration
      GError *error = NULL;
      if (gtkterm_serial_port_reconfigure(serial_port, &new_config, &error) == 0) {
        char *msg = g_strdup_printf("Port reconfigured: %s %d-%d-%c-%d",
                                     port, baudrate, new_config.bits,
                                     (new_config.parity == GTKTERM_SERIAL_PORT_PARITY_NONE ? 'N' :
                                      new_config.parity == GTKTERM_SERIAL_PORT_PARITY_ODD ? 'O' : 'E'),
                                     new_config.stopbits);
        gtkterm_show_infobar(window, msg, GTK_MESSAGE_INFO);
        g_free(msg);
      } else {
        char *msg = g_strdup_printf("Failed to reconfigure port: %s",
                                     error ? error->message : "Unknown error");
        gtkterm_show_infobar(window, msg, GTK_MESSAGE_ERROR);
        g_free(msg);
        if (error) g_error_free(error);
      }

      g_free(new_config.port);
    }
  }

  gtk_window_destroy (GTK_WINDOW (dialog));
}

/**
 * @brief Open port configuration dialog
 */
static void on_gtkterm_config_port (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  // Get current configuration
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port(window->terminal_window);
  port_config_t *current_config = NULL;
  if (serial_port) {
    current_config = gtkterm_serial_port_get_config(serial_port);
  }

  // Create dialog
  GtkWidget *dialog = gtk_dialog_new_with_buttons (
    "Port Configuration",
    GTK_WINDOW (window),
    GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT,
    "_Cancel", GTK_RESPONSE_CANCEL,
    "_OK", GTK_RESPONSE_OK,
    NULL);

  gtk_window_set_default_size (GTK_WINDOW (dialog), 400, 300);

  // Create grid for layout
  GtkWidget *grid = gtk_grid_new ();
  gtk_grid_set_row_spacing (GTK_GRID (grid), 10);
  gtk_grid_set_column_spacing (GTK_GRID (grid), 10);
  gtk_widget_set_margin_start (grid, 20);
  gtk_widget_set_margin_end (grid, 20);
  gtk_widget_set_margin_top (grid, 20);
  gtk_widget_set_margin_bottom (grid, 20);

  // Port selection
  GtkWidget *port_label = gtk_label_new ("Port:");
  gtk_widget_set_halign (port_label, GTK_ALIGN_START);
  GtkWidget *port_entry = gtk_entry_new ();
  gtk_editable_set_text (GTK_EDITABLE (port_entry),
                         current_config ? current_config->port : "/dev/ttyUSB0");
  gtk_grid_attach (GTK_GRID (grid), port_label, 0, 0, 1, 1);
  gtk_grid_attach (GTK_GRID (grid), port_entry, 1, 0, 1, 1);

  // Baud rate
  GtkWidget *baud_label = gtk_label_new ("Baud Rate:");
  gtk_widget_set_halign (baud_label, GTK_ALIGN_START);
  const char *baudrates[] = {"300", "600", "1200", "2400", "4800", "9600", "19200", "38400",
                              "57600", "115200", "230400", "460800", "500000", "576000",
                              "921600", "1000000", "2000000", NULL};
  int baudrate_values[] = {300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200,
                           230400, 460800, 500000, 576000, 921600, 1000000, 2000000};
  GtkWidget *baudrate_combo = gtk_drop_down_new_from_strings (baudrates);

  // Find and set current baudrate
  int baudrate_idx = 9; // Default to 115200
  if (current_config) {
    for (int i = 0; i < 17; i++) {
      if (baudrate_values[i] == current_config->baudrate) {
        baudrate_idx = i;
        break;
      }
    }
  }
  gtk_drop_down_set_selected (GTK_DROP_DOWN (baudrate_combo), baudrate_idx);
  gtk_grid_attach (GTK_GRID (grid), baud_label, 0, 1, 1, 1);
  gtk_grid_attach (GTK_GRID (grid), baudrate_combo, 1, 1, 1, 1);

  // Data bits
  GtkWidget *bits_label = gtk_label_new ("Data Bits:");
  gtk_widget_set_halign (bits_label, GTK_ALIGN_START);
  const char *bits[] = {"5", "6", "7", "8", NULL};
  GtkWidget *bits_combo = gtk_drop_down_new_from_strings (bits);
  gtk_drop_down_set_selected (GTK_DROP_DOWN (bits_combo),
                               current_config ? current_config->bits - 5 : 3);
  gtk_grid_attach (GTK_GRID (grid), bits_label, 0, 2, 1, 1);
  gtk_grid_attach (GTK_GRID (grid), bits_combo, 1, 2, 1, 1);

  // Stop bits
  GtkWidget *stop_label = gtk_label_new ("Stop Bits:");
  gtk_widget_set_halign (stop_label, GTK_ALIGN_START);
  const char *stopbits[] = {"1", "2", NULL};
  GtkWidget *stopbits_combo = gtk_drop_down_new_from_strings (stopbits);
  gtk_drop_down_set_selected (GTK_DROP_DOWN (stopbits_combo),
                               current_config ? current_config->stopbits - 1 : 0);
  gtk_grid_attach (GTK_GRID (grid), stop_label, 0, 3, 1, 1);
  gtk_grid_attach (GTK_GRID (grid), stopbits_combo, 1, 3, 1, 1);

  // Parity
  GtkWidget *parity_label = gtk_label_new ("Parity:");
  gtk_widget_set_halign (parity_label, GTK_ALIGN_START);
  const char *parity[] = {"None", "Odd", "Even", NULL};
  GtkWidget *parity_combo = gtk_drop_down_new_from_strings (parity);
  int parity_idx = 0;
  if (current_config) {
    if (current_config->parity == GTKTERM_SERIAL_PORT_PARITY_ODD) parity_idx = 1;
    else if (current_config->parity == GTKTERM_SERIAL_PORT_PARITY_EVEN) parity_idx = 2;
  }
  gtk_drop_down_set_selected (GTK_DROP_DOWN (parity_combo), parity_idx);
  gtk_grid_attach (GTK_GRID (grid), parity_label, 0, 4, 1, 1);
  gtk_grid_attach (GTK_GRID (grid), parity_combo, 1, 4, 1, 1);

  // Flow control
  GtkWidget *flow_label = gtk_label_new ("Flow Control:");
  gtk_widget_set_halign (flow_label, GTK_ALIGN_START);
  const char *flow[] = {"None", "XON/XOFF", "RTS/CTS", "RS485", NULL};
  GtkWidget *flow_combo = gtk_drop_down_new_from_strings (flow);
  int flow_idx = 0;
  if (current_config) {
    flow_idx = current_config->flow_control;
  }
  gtk_drop_down_set_selected (GTK_DROP_DOWN (flow_combo), flow_idx);
  gtk_grid_attach (GTK_GRID (grid), flow_label, 0, 5, 1, 1);
  gtk_grid_attach (GTK_GRID (grid), flow_combo, 1, 5, 1, 1);

  // Store widgets as data on the dialog for retrieval
  g_object_set_data (G_OBJECT (dialog), "port_entry", port_entry);
  g_object_set_data (G_OBJECT (dialog), "baudrate_combo", baudrate_combo);
  g_object_set_data (G_OBJECT (dialog), "bits_combo", bits_combo);
  g_object_set_data (G_OBJECT (dialog), "stopbits_combo", stopbits_combo);
  g_object_set_data (G_OBJECT (dialog), "parity_combo", parity_combo);
  g_object_set_data (G_OBJECT (dialog), "flow_combo", flow_combo);

  // Add grid to dialog content area
  GtkWidget *content_area = gtk_dialog_get_content_area (GTK_DIALOG (dialog));
  gtk_box_append (GTK_BOX (content_area), grid);

  // Connect response signal
  g_signal_connect (dialog, "response", G_CALLBACK (on_port_config_response), window);

  // Show dialog
  gtk_widget_show (dialog);
}

/**
 * @brief Open main window configuration dialog
 */
static void on_gtkterm_config_main_window (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  /** \todo Implement main window configuration dialog */
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_show_infobar (window, "Main window configuration dialog not yet implemented", GTK_MESSAGE_INFO);
}

/**
 * @brief Macro editor dialog context
 */
typedef struct {
  GtkTermWindow *window;
  GtkWidget *dialog;
  GtkWidget *macro_list;
  GtkWidget *macro_name_entry;
  GtkWidget *macro_shortcut_entry;
  GtkWidget *commands_text_view;
  GtkTextBuffer *commands_buffer;
  GtkWidget *new_macro_button;
  GtkWidget *delete_macro_button;
  GtkWidget *record_button;
  GtkWidget *play_button;
  GtkWidget *save_macro_button;
  GtkSingleSelection *selection_model;
  GListStore *list_store;
  GtkTermMacro *current_macro;
} MacroEditorContext;

/**
 * @brief Update the macro details panel
 */
static void macro_editor_update_details (MacroEditorContext *ctx) {
  if (!ctx->current_macro) {
    gtk_editable_set_text (GTK_EDITABLE (ctx->macro_name_entry), "");
    gtk_editable_set_text (GTK_EDITABLE (ctx->macro_shortcut_entry), "");
    gtk_text_buffer_set_text (ctx->commands_buffer, "", -1);
    gtk_widget_set_sensitive (ctx->delete_macro_button, FALSE);
    gtk_widget_set_sensitive (ctx->play_button, FALSE);
    gtk_widget_set_sensitive (ctx->save_macro_button, FALSE);
    return;
  }

  gtk_editable_set_text (GTK_EDITABLE (ctx->macro_name_entry),
                         gtkterm_macro_get_name (ctx->current_macro));

  const char *shortcut = gtkterm_macro_get_shortcut (ctx->current_macro);
  gtk_editable_set_text (GTK_EDITABLE (ctx->macro_shortcut_entry),
                         shortcut ? shortcut : "");

  /* Build commands text */
  GString *commands_str = g_string_new (NULL);
  GPtrArray *commands = gtkterm_macro_get_commands (ctx->current_macro);

  for (guint i = 0; i < commands->len; i++) {
    GtkTermMacroCommand *cmd = g_ptr_array_index (commands, i);

    if (cmd->type == GTKTERM_MACRO_CMD_SEND) {
      g_string_append_printf (commands_str, "send: %s\n", cmd->data);
    } else if (cmd->type == GTKTERM_MACRO_CMD_WAIT) {
      g_string_append_printf (commands_str, "wait: %u ms\n", cmd->delay_ms);
    }
  }

  gtk_text_buffer_set_text (ctx->commands_buffer, commands_str->str, -1);
  g_string_free (commands_str, TRUE);

  gtk_widget_set_sensitive (ctx->delete_macro_button, TRUE);
  gtk_widget_set_sensitive (ctx->play_button, TRUE);
  gtk_widget_set_sensitive (ctx->save_macro_button, TRUE);
}

/**
 * @brief Macro list item factory setup
 */
static void macro_list_setup (GtkSignalListItemFactory *factory,
                               GtkListItem *list_item,
                               gpointer user_data) {
  GtkWidget *label = gtk_label_new (NULL);
  gtk_label_set_xalign (GTK_LABEL (label), 0.0);
  gtk_list_item_set_child (list_item, label);
}

/**
 * @brief Macro list item factory bind
 */
static void macro_list_bind (GtkSignalListItemFactory *factory,
                              GtkListItem *list_item,
                              gpointer user_data) {
  GtkTermMacro *macro = gtk_list_item_get_item (list_item);
  GtkWidget *label = gtk_list_item_get_child (list_item);

  const char *name = gtkterm_macro_get_name (macro);
  const char *shortcut = gtkterm_macro_get_shortcut (macro);

  char *display_text;
  if (shortcut && *shortcut) {
    display_text = g_strdup_printf ("%s (%s)", name, shortcut);
  } else {
    display_text = g_strdup (name);
  }

  gtk_label_set_text (GTK_LABEL (label), display_text);
  g_free (display_text);
}

/**
 * @brief Macro list selection changed
 */
static void on_macro_list_selection_changed (GtkSingleSelection *selection,
                                              GParamSpec *pspec,
                                              gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;
  guint selected = gtk_single_selection_get_selected (selection);

  /* Release the previous macro reference if it exists */
  if (ctx->current_macro) {
    g_object_unref (ctx->current_macro);
    ctx->current_macro = NULL;
  }

  /* Get the newly selected macro (this returns a new reference) */
  if (selected != GTK_INVALID_LIST_POSITION) {
    ctx->current_macro = g_list_model_get_item (G_LIST_MODEL (selection), selected);
  }

  macro_editor_update_details (ctx);
}

/**
 * @brief New macro button clicked
 */
static void on_new_macro_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  /* Create a new macro with default name */
  guint count = g_list_model_get_n_items (G_LIST_MODEL (ctx->list_store));
  char *name = g_strdup_printf ("New Macro %u", count + 1);

  GtkTermMacro *macro = gtkterm_macro_new (name, NULL);
  g_free (name);

  /* Add to manager and list */
  gtkterm_macro_manager_add_macro (ctx->window->macro_manager, macro);
  g_list_store_append (ctx->list_store, macro);

  /* Select the new macro */
  guint new_pos = g_list_model_get_n_items (G_LIST_MODEL (ctx->list_store)) - 1;
  gtk_single_selection_set_selected (ctx->selection_model, new_pos);
}

/**
 * @brief Delete macro button clicked
 */
static void on_delete_macro_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  if (!ctx->current_macro) {
    return;
  }

  guint selected = gtk_single_selection_get_selected (ctx->selection_model);
  const char *name = gtkterm_macro_get_name (ctx->current_macro);

  /* Remove from manager and list */
  gtkterm_macro_manager_remove_macro (ctx->window->macro_manager, name);
  g_list_store_remove (ctx->list_store, selected);

  /* Release the current macro reference */
  g_object_unref (ctx->current_macro);
  ctx->current_macro = NULL;
  macro_editor_update_details (ctx);
}

/**
 * @brief Send function for macro playback
 */
static void macro_send_func (const char *data, gsize length, gpointer user_data) {
  GtkTermWindow *window = (GtkTermWindow *) user_data;
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);

  if (serial_port) {
    int chars_transmitted = 0;
    g_signal_emit (serial_port, gtkterm_signals[SIGNAL_GTKTERM_SERIAL_DATA_TRANSMIT],
                   0, (char *)data, (int)length, &chars_transmitted);
  }
}

/**
 * @brief Play macro button clicked
 */
static void on_play_macro_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  if (!ctx->current_macro) {
    return;
  }

  const char *name = gtkterm_macro_get_name (ctx->current_macro);
  gtkterm_macro_manager_play_macro (ctx->window->macro_manager,
                                    name,
                                    macro_send_func,
                                    ctx->window);
}

/**
 * @brief Record macro button clicked
 */
static void on_record_macro_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  if (!ctx->current_macro) {
    gtkterm_show_infobar (ctx->window,
                          "Please select or create a macro first",
                          GTK_MESSAGE_INFO);
    return;
  }

  if (gtkterm_macro_manager_is_recording (ctx->window->macro_manager)) {
    gtkterm_macro_manager_stop_recording (ctx->window->macro_manager);
    gtk_button_set_label (button, "_Record");
    gtkterm_show_infobar (ctx->window, "Recording stopped", GTK_MESSAGE_INFO);
  } else {
    const char *name = gtkterm_macro_get_name (ctx->current_macro);
    gtkterm_macro_manager_start_recording (ctx->window->macro_manager, name);
    gtk_button_set_label (button, "Stop _Recording");
    gtkterm_show_infobar (ctx->window, "Recording started", GTK_MESSAGE_INFO);
  }
}

/**
 * @brief Parse commands from text buffer
 */
static void parse_commands_text (MacroEditorContext *ctx, GtkTermMacro *macro) {
  GtkTextIter start, end;
  gtk_text_buffer_get_bounds (ctx->commands_buffer, &start, &end);
  char *text = gtk_text_buffer_get_text (ctx->commands_buffer, &start, &end, FALSE);

  gtkterm_macro_clear_commands (macro);

  char **lines = g_strsplit (text, "\n", -1);
  for (int i = 0; lines[i] != NULL; i++) {
    char *line = g_strstrip (lines[i]);

    if (g_str_has_prefix (line, "send:")) {
      char *data = g_strstrip (line + 5);
      gtkterm_macro_add_command (macro, GTKTERM_MACRO_CMD_SEND, data, 0);
    } else if (g_str_has_prefix (line, "wait:")) {
      char *delay_str = g_strstrip (line + 5);
      guint delay_ms = atoi (delay_str);
      gtkterm_macro_add_command (macro, GTKTERM_MACRO_CMD_WAIT, NULL, delay_ms);
    }
  }

  g_strfreev (lines);
  g_free (text);
}

/**
 * @brief Save macro button clicked
 */
static void on_save_macro_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  if (!ctx->current_macro) {
    return;
  }

  /* Update macro properties */
  const char *name = gtk_editable_get_text (GTK_EDITABLE (ctx->macro_name_entry));
  const char *shortcut = gtk_editable_get_text (GTK_EDITABLE (ctx->macro_shortcut_entry));

  gtkterm_macro_set_name (ctx->current_macro, name);
  gtkterm_macro_set_shortcut (ctx->current_macro, shortcut);

  /* Parse and update commands */
  parse_commands_text (ctx, ctx->current_macro);

  /* Save all macros to file */
  char *config_dir = g_build_filename (g_get_user_config_dir (), "gtkterm", NULL);
  g_mkdir_with_parents (config_dir, 0755);

  char *macros_file = g_build_filename (config_dir, "macros.json", NULL);
  GError *error = NULL;

  if (!gtkterm_macro_manager_save_to_file (ctx->window->macro_manager, macros_file, &error)) {
    char *msg = g_strdup_printf ("Failed to save macros: %s", error->message);
    gtkterm_show_infobar (ctx->window, msg, GTK_MESSAGE_ERROR);
    g_free (msg);
    g_error_free (error);
  } else {
    gtkterm_show_infobar (ctx->window, "Macro saved successfully", GTK_MESSAGE_INFO);
  }

  g_free (macros_file);
  g_free (config_dir);

  /* Refresh the list to show updated name/shortcut */
  guint selected = gtk_single_selection_get_selected (ctx->selection_model);
  g_list_store_remove (ctx->list_store, selected);
  g_list_store_insert (ctx->list_store, selected, ctx->current_macro);
  gtk_single_selection_set_selected (ctx->selection_model, selected);
}

/**
 * @brief Add Send button clicked
 */
static void on_add_send_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  GtkTextIter end;
  gtk_text_buffer_get_end_iter (ctx->commands_buffer, &end);
  gtk_text_buffer_insert (ctx->commands_buffer, &end, "send: \n", -1);
}

/**
 * @brief Add Wait button clicked
 */
static void on_add_wait_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  GtkTextIter end;
  gtk_text_buffer_get_end_iter (ctx->commands_buffer, &end);
  gtk_text_buffer_insert (ctx->commands_buffer, &end, "wait: 100 ms\n", -1);
}

/**
 * @brief Clear commands button clicked
 */
static void on_clear_commands_clicked (GtkButton *button, gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;
  gtk_text_buffer_set_text (ctx->commands_buffer, "", -1);
}

/**
 * @brief Cleanup when dialog is destroyed
 */
static void on_macro_editor_dialog_close (gpointer user_data) {
  MacroEditorContext *ctx = (MacroEditorContext *) user_data;

  g_print ("DEBUG: on_macro_editor_dialog_close() called\n");

  /* Release the current macro reference if it exists */
  if (ctx->current_macro) {
    g_print ("DEBUG: Unreffing current_macro\n");
    g_object_unref (ctx->current_macro);
    ctx->current_macro = NULL;
  }

  /* Note: selection_model and list_store are owned by the GtkListView widget.
   * When we set the model with gtk_list_view_set_model(), the list view takes
   * ownership. We should NOT unref them here - the dialog's widget tree will
   * clean them up automatically when the dialog is destroyed.
   *
   * We only stored the pointers in ctx for convenience, but we don't own them.
   */
  g_print ("DEBUG: Models owned by list view, will be cleaned up automatically\n");

  /* Free the context */
  g_print ("DEBUG: Freeing context\n");
  g_free (ctx);
  g_print ("DEBUG: on_macro_editor_dialog_close() complete\n");
}

/**
 * @brief Open macros configuration dialog
 */
static void on_gtkterm_macros (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  /* Create context */
  MacroEditorContext *ctx = g_new0 (MacroEditorContext, 1);
  ctx->window = window;

  /* Load dialog from resources */
  GtkBuilder *builder = gtk_builder_new_from_resource ("/com/github/jeija/gtkterm/macro_editor.ui");
  ctx->dialog = GTK_WIDGET (gtk_builder_get_object (builder, "macro_editor_dialog"));

  /* Get UI elements */
  ctx->macro_list = GTK_WIDGET (gtk_builder_get_object (builder, "macro_list"));
  ctx->macro_name_entry = GTK_WIDGET (gtk_builder_get_object (builder, "macro_name_entry"));
  ctx->macro_shortcut_entry = GTK_WIDGET (gtk_builder_get_object (builder, "macro_shortcut_entry"));
  ctx->commands_text_view = GTK_WIDGET (gtk_builder_get_object (builder, "commands_text_view"));
  ctx->commands_buffer = gtk_text_view_get_buffer (GTK_TEXT_VIEW (ctx->commands_text_view));

  ctx->new_macro_button = GTK_WIDGET (gtk_builder_get_object (builder, "new_macro_button"));
  ctx->delete_macro_button = GTK_WIDGET (gtk_builder_get_object (builder, "delete_macro_button"));
  ctx->record_button = GTK_WIDGET (gtk_builder_get_object (builder, "record_button"));
  ctx->play_button = GTK_WIDGET (gtk_builder_get_object (builder, "play_button"));
  ctx->save_macro_button = GTK_WIDGET (gtk_builder_get_object (builder, "save_macro_button"));

  GtkWidget *add_send_button = GTK_WIDGET (gtk_builder_get_object (builder, "add_send_button"));
  GtkWidget *add_wait_button = GTK_WIDGET (gtk_builder_get_object (builder, "add_wait_button"));
  GtkWidget *clear_commands_button = GTK_WIDGET (gtk_builder_get_object (builder, "clear_commands_button"));

  /* Setup macro list */
  ctx->list_store = g_list_store_new (GTKTERM_TYPE_MACRO);

  /* Populate list with existing macros */
  GPtrArray *macros = gtkterm_macro_manager_get_all_macros (window->macro_manager);
  for (guint i = 0; i < macros->len; i++) {
    GtkTermMacro *macro = g_ptr_array_index (macros, i);
    g_list_store_append (ctx->list_store, macro);
  }

  /* Create selection model */
  ctx->selection_model = gtk_single_selection_new (G_LIST_MODEL (ctx->list_store));
  gtk_single_selection_set_can_unselect (ctx->selection_model, TRUE);

  /* Create list item factory */
  GtkListItemFactory *factory = gtk_signal_list_item_factory_new ();
  g_signal_connect (factory, "setup", G_CALLBACK (macro_list_setup), NULL);
  g_signal_connect (factory, "bind", G_CALLBACK (macro_list_bind), NULL);

  /* Setup list view */
  gtk_list_view_set_model (GTK_LIST_VIEW (ctx->macro_list),
                           GTK_SELECTION_MODEL (ctx->selection_model));
  gtk_list_view_set_factory (GTK_LIST_VIEW (ctx->macro_list), factory);

  /* Connect signals */
  g_signal_connect (ctx->selection_model, "notify::selected",
                    G_CALLBACK (on_macro_list_selection_changed), ctx);
  g_signal_connect (ctx->new_macro_button, "clicked",
                    G_CALLBACK (on_new_macro_clicked), ctx);
  g_signal_connect (ctx->delete_macro_button, "clicked",
                    G_CALLBACK (on_delete_macro_clicked), ctx);
  g_signal_connect (ctx->record_button, "clicked",
                    G_CALLBACK (on_record_macro_clicked), ctx);
  g_signal_connect (ctx->play_button, "clicked",
                    G_CALLBACK (on_play_macro_clicked), ctx);
  g_signal_connect (ctx->save_macro_button, "clicked",
                    G_CALLBACK (on_save_macro_clicked), ctx);
  g_signal_connect (add_send_button, "clicked",
                    G_CALLBACK (on_add_send_clicked), ctx);
  g_signal_connect (add_wait_button, "clicked",
                    G_CALLBACK (on_add_wait_clicked), ctx);
  g_signal_connect (clear_commands_button, "clicked",
                    G_CALLBACK (on_clear_commands_clicked), ctx);

  /* Initialize with no selection */
  ctx->current_macro = NULL;
  macro_editor_update_details (ctx);

  /* Attach context to dialog - it will be freed when dialog is destroyed */
  g_object_set_data_full (G_OBJECT (ctx->dialog), "macro-editor-context", ctx,
                          (GDestroyNotify) on_macro_editor_dialog_close);

  /* Show dialog */
  gtk_window_set_transient_for (GTK_WINDOW (ctx->dialog), GTK_WINDOW (window));
  gtk_window_present (GTK_WINDOW (ctx->dialog));

  g_object_unref (builder);
}

/**
 * @brief Load configuration
 */
static void on_gtkterm_load_config (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  /** \todo Implement load configuration dialog */
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_show_infobar (window, "Load configuration dialog not yet implemented", GTK_MESSAGE_INFO);
}

/**
 * @brief Save configuration
 */
static void on_gtkterm_save_config (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  /** \todo Implement save configuration dialog */
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_show_infobar (window, "Save configuration dialog not yet implemented", GTK_MESSAGE_INFO);
}

/**
 * @brief Remove configuration
 */
static void on_gtkterm_remove_config (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  /** \todo Implement remove configuration dialog */
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_show_infobar (window, "Remove configuration dialog not yet implemented", GTK_MESSAGE_INFO);
}

/**
 * @brief Callback for log file chooser
 */
static void on_log_file_response_cb (GtkNativeDialog *dialog,
                  int              response_id,
                  gpointer         user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  GtkFileChooserNative *native = GTK_FILE_CHOOSER_NATIVE (dialog);

  if (response_id == GTK_RESPONSE_ACCEPT) {
    GFile *file = gtk_file_chooser_get_file (GTK_FILE_CHOOSER (native));
    char *filename = g_file_get_path (file);

    GError *error = NULL;
    if (gtkterm_logging_start (window->logging, filename, &error)) {
      char *msg = g_strdup_printf ("Logging to %s", filename);
      gtkterm_show_infobar (window, msg, GTK_MESSAGE_INFO);
      g_free (msg);
    } else {
      char *msg = g_strdup_printf ("Failed to start logging: %s",
                                   error ? error->message : "Unknown error");
      gtkterm_show_infobar (window, msg, GTK_MESSAGE_ERROR);
      g_free (msg);
      if (error) g_error_free (error);
    }

    g_free (filename);
    g_object_unref (file);
  }

  gtk_native_dialog_destroy (GTK_NATIVE_DIALOG (native));
  g_object_unref (native);
}

/**
 * @brief Log to file
 */
static void on_gtkterm_log_to_file (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  GtkFileChooserNative *native;
  GtkFileChooserAction file_action = GTK_FILE_CHOOSER_ACTION_SAVE;

  native = gtk_file_chooser_native_new ("Save Log File",
                                        GTK_WINDOW (window),
                                        file_action,
                                        "_Save",
                                        "_Cancel");

  g_signal_connect (native, "response", G_CALLBACK (on_log_file_response_cb), window);
  gtk_native_dialog_show (GTK_NATIVE_DIALOG (native));
}

/**
 * @brief Resume logging
 */
static void on_gtkterm_log_resume (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  if (gtkterm_logging_get_state (window->logging) == GTKTERM_LOGGING_PAUSED) {
    gtkterm_logging_resume (window->logging);
    gtkterm_show_infobar (window, "Logging resumed", GTK_MESSAGE_INFO);
  } else {
    gtkterm_show_infobar (window, "Logging is not paused", GTK_MESSAGE_WARNING);
  }
}

/**
 * @brief Stop/Pause logging
 */
static void on_gtkterm_log_stop (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  GtkTermLoggingState state = gtkterm_logging_get_state (window->logging);
  if (state == GTKTERM_LOGGING_ACTIVE) {
    gtkterm_logging_pause (window->logging);
    gtkterm_show_infobar (window, "Logging paused", GTK_MESSAGE_INFO);
  } else if (state == GTKTERM_LOGGING_PAUSED) {
    gtkterm_logging_stop (window->logging);
    gtkterm_show_infobar (window, "Logging stopped", GTK_MESSAGE_INFO);
  } else {
    gtkterm_show_infobar (window, "Logging is not active", GTK_MESSAGE_WARNING);
  }
}

/**
 * @brief Clear log
 */
static void on_gtkterm_log_clear (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  if (gtkterm_logging_get_state (window->logging) != GTKTERM_LOGGING_STOPPED) {
    gtkterm_logging_clear (window->logging);
    gtkterm_show_infobar (window, "Log file cleared", GTK_MESSAGE_INFO);
  } else {
    gtkterm_show_infobar (window, "No log file is active", GTK_MESSAGE_WARNING);
  }
}

/**
 * @brief Send break signal
 */
static void on_gtkterm_send_break (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);

  if (serial_port && gtkterm_serial_port_get_status (serial_port) == GTKTERM_SERIAL_PORT_CONNECTED) {
    gtkterm_serial_port_send_break (serial_port);
    gtkterm_show_infobar (window, "Break signal sent", GTK_MESSAGE_INFO);
  } else {
    gtkterm_show_infobar (window, "Port not connected", GTK_MESSAGE_WARNING);
  }
}

/**
 * @brief Open serial port
 */
static void on_gtkterm_open_port (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);

  g_print("DEBUG: Menu - Open Port action triggered\n");

  if (serial_port) {
    int result = gtkterm_serial_port_open_port (serial_port);
    g_print("DEBUG: Menu - gtkterm_serial_port_open_port returned %d\n", result);

    if (result == true) {
      gtkterm_show_infobar (window, "Port opened successfully", GTK_MESSAGE_INFO);
    } else {
      GError *error = gtkterm_serial_port_get_error(serial_port);
      char *msg = g_strdup_printf("Failed to open port: %s",
                                  error ? error->message : "Unknown error");
      gtkterm_show_infobar (window, msg, GTK_MESSAGE_ERROR);
      g_free(msg);
    }
  }
}

/**
 * @brief Close serial port
 */
static void on_gtkterm_close_port (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);

  if (serial_port) {
    int result = gtkterm_serial_port_close_port (serial_port);
    if (result == 0) {
      gtkterm_show_infobar (window, "Port closed successfully", GTK_MESSAGE_INFO);
    } else {
      gtkterm_show_infobar (window, "Failed to close port", GTK_MESSAGE_ERROR);
    }
  }
}

/**
 * @brief Toggle DTR signal
 */
static void on_gtkterm_toggle_DTR (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);

  if (serial_port && gtkterm_serial_port_get_status (serial_port) == GTKTERM_SERIAL_PORT_CONNECTED) {
    gtkterm_serial_port_toggle_DTR (serial_port);
    gtkterm_show_infobar (window, "DTR toggled", GTK_MESSAGE_INFO);
  } else {
    gtkterm_show_infobar (window, "Port not connected", GTK_MESSAGE_WARNING);
  }
}

/**
 * @brief Toggle RTS signal
 */
static void on_gtkterm_toggle_RTS (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);

  if (serial_port && gtkterm_serial_port_get_status (serial_port) == GTKTERM_SERIAL_PORT_CONNECTED) {
    gtkterm_serial_port_toggle_RTS (serial_port);
    gtkterm_show_infobar (window, "RTS toggled", GTK_MESSAGE_INFO);
  } else {
    gtkterm_show_infobar (window, "Port not connected", GTK_MESSAGE_WARNING);
  }
}

/**
 * @brief Show ASCII view mode
 */
static void on_gtkterm_show_ascii (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_window_switch_view (window, GTKTERM_TERMINAL_VIEW_ASCII);
  gtkterm_show_infobar (window, "Switched to ASCII view mode", GTK_MESSAGE_INFO);
}

/**
 * @brief Show hexadecimal view mode
 */
static void on_gtkterm_show_hex (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_window_switch_view (window, GTKTERM_TERMINAL_VIEW_HEX);
  gtkterm_show_infobar (window, "Switched to hexadecimal view mode", GTK_MESSAGE_INFO);
}

/**
 * @brief Show binary view mode
 */
static void on_gtkterm_show_binary (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_window_switch_view (window, GTKTERM_TERMINAL_VIEW_BINARY);
  gtkterm_show_infobar (window, "Switched to binary view mode", GTK_MESSAGE_INFO);
}

/**
 * @brief Show decimal view mode
 */
static void on_gtkterm_show_decimal (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_window_switch_view (window, GTKTERM_TERMINAL_VIEW_DECIMAL);
  gtkterm_show_infobar (window, "Switched to decimal view mode", GTK_MESSAGE_INFO);
}

/**
 * @brief Show mixed (hex + ASCII) view mode
 */
static void on_gtkterm_show_mixed (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_window_switch_view (window, GTKTERM_TERMINAL_VIEW_MIXED);
  gtkterm_show_infobar (window, "Switched to mixed (hex+ASCII) view mode", GTK_MESSAGE_INFO);
}

/**
 * @brief Show mixed (binary + ASCII) view mode
 */
static void on_gtkterm_show_binary_mixed (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_window_switch_view (window, GTKTERM_TERMINAL_VIEW_BINARY_MIXED);
  gtkterm_show_infobar (window, "Switched to mixed (binary+ASCII) view mode", GTK_MESSAGE_INFO);
}

/**
 * @brief Show mixed (decimal + ASCII) view mode
 */
static void on_gtkterm_show_decimal_mixed (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_window_switch_view (window, GTKTERM_TERMINAL_VIEW_DECIMAL_MIXED);
  gtkterm_show_infobar (window, "Switched to mixed (decimal+ASCII) view mode", GTK_MESSAGE_INFO);
}

/**
 * @brief Toggle timestamp display
 */
static void on_gtkterm_show_timestamps (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  if (window->dataview) {
    gboolean current = gtkterm_dataview_get_show_timestamps (window->dataview);
    gtkterm_dataview_set_show_timestamps (window->dataview, !current);

    if (!current) {
      gtkterm_show_infobar (window, "Timestamps enabled", GTK_MESSAGE_INFO);
    } else {
      gtkterm_show_infobar (window, "Timestamps disabled", GTK_MESSAGE_INFO);
    }
  }
}

/**
 * @brief Toggle direction (TX/RX) display
 */
static void on_gtkterm_show_direction (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  if (window->dataview) {
    GtkTermDataViewOptions options;
    gtkterm_dataview_get_options (window->dataview, &options);
    options.show_direction = !options.show_direction;
    gtkterm_dataview_set_options (window->dataview, &options);

    if (options.show_direction) {
      gtkterm_show_infobar (window, "Direction indicators enabled", GTK_MESSAGE_INFO);
    } else {
      gtkterm_show_infobar (window, "Direction indicators disabled", GTK_MESSAGE_INFO);
    }
  }
}

/**
 * @brief Perform search in the terminal
 *
 * @param window The window
 * @param forward TRUE to search forward, FALSE to search backward
 */
static void perform_search (GtkTermWindow *window, gboolean forward) {
  const char *search_text = gtk_editable_get_text (GTK_EDITABLE (window->search_entry));

  if (!search_text || strlen(search_text) == 0) {
    return;
  }

  gboolean case_sensitive = gtk_check_button_get_active (GTK_CHECK_BUTTON (window->search_case_sensitive));
  gboolean is_regex = gtk_check_button_get_active (GTK_CHECK_BUTTON (window->search_regex));
  gboolean is_hex = gtk_check_button_get_active (GTK_CHECK_BUTTON (window->search_hex));

  VteTerminal *vte = VTE_TERMINAL (window->terminal_window);

  // Convert hex pattern if needed
  GString *pattern = g_string_new (search_text);
  if (is_hex) {
    // Parse hex pattern "48 65 6C 6C 6F" or "48656C6C6F"
    GString *hex_pattern = g_string_new ("");
    const char *p = search_text;
    while (*p) {
      if (g_ascii_isxdigit(*p)) {
        char hex_byte[3] = {0};
        hex_byte[0] = *p++;
        if (*p && g_ascii_isxdigit(*p)) {
          hex_byte[1] = *p++;
        }
        unsigned int byte = 0;
        sscanf(hex_byte, "%x", &byte);
        g_string_append_c (hex_pattern, (char)byte);
      } else {
        p++; // Skip spaces and other chars
      }
    }
    g_string_free (pattern, TRUE);
    pattern = hex_pattern;
  }

  // Set up search using VTE's regex search
  VteRegex *regex = NULL;
  GError *error = NULL;

  guint32 flags = PCRE2_MULTILINE | PCRE2_UTF;
  if (!case_sensitive) {
    flags |= PCRE2_CASELESS;
  }

  if (is_regex || is_hex) {
    regex = vte_regex_new_for_search (pattern->str, pattern->len, flags, &error);
  } else {
    // Escape special regex characters for literal search
    char *escaped = g_regex_escape_string (pattern->str, pattern->len);
    regex = vte_regex_new_for_search (escaped, -1, flags, &error);
    g_free (escaped);
  }

  if (error) {
    gtkterm_show_infobar (window, error->message, GTK_MESSAGE_ERROR);
    g_error_free (error);
    g_string_free (pattern, TRUE);
    return;
  }

  // Set the search regex
  vte_terminal_search_set_regex (vte, regex, 0);
  vte_terminal_search_set_wrap_around (vte, TRUE);

  // Perform the search
  gboolean found = forward ?
    vte_terminal_search_find_next (vte) :
    vte_terminal_search_find_previous (vte);

  if (!found) {
    gtkterm_show_infobar (window, "Pattern not found", GTK_MESSAGE_INFO);
  }

  vte_regex_unref (regex);
  g_string_free (pattern, TRUE);
}

/**
 * @brief Show/hide search bar
 */
static void on_gtkterm_search_show (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  gboolean is_visible = gtk_revealer_get_reveal_child (GTK_REVEALER (window->search_revealer));
  gtk_revealer_set_reveal_child (GTK_REVEALER (window->search_revealer), !is_visible);

  if (!is_visible) {
    // Focus search entry when showing
    gtk_widget_grab_focus (window->search_entry);
  }
}

/**
 * @brief Search entry text changed callback
 */
static void on_search_entry_changed (GtkEntry *entry, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  // Auto-search as user types
  perform_search (window, TRUE);
}

/**
 * @brief Find next occurrence
 */
static void on_search_next_clicked (GtkButton *button, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  perform_search (window, TRUE);
}

/**
 * @brief Find previous occurrence
 */
static void on_search_prev_clicked (GtkButton *button, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  perform_search (window, FALSE);
}

/**
 * @brief Close search bar
 */
static void on_search_close_clicked (GtkButton *button, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtk_revealer_set_reveal_child (GTK_REVEALER (window->search_revealer), FALSE);

  // Clear search highlight
  VteTerminal *vte = VTE_TERMINAL (window->terminal_window);
  vte_terminal_search_set_regex (vte, NULL, 0);
}

/**
 * @brief Change hex view format (number of bytes per line)
 */
static void on_gtkterm_view_hex (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  // Parameter should be a string like "8", "10", "16", "24", or "32"
  if (parameter) {
    const char *hex_chars_str = g_variant_get_string (parameter, NULL);
    int hex_chars = atoi (hex_chars_str);

    if (hex_chars == 8 || hex_chars == 10 || hex_chars == 16 || hex_chars == 24 || hex_chars == 32) {
      gtkterm_terminal_set_hex_chars (window->terminal_window, hex_chars);
      char *msg = g_strdup_printf ("Hex view format set to %d chars per line", hex_chars);
      gtkterm_show_infobar (window, msg, GTK_MESSAGE_INFO);
      g_free (msg);
    } else {
      gtkterm_show_infobar (window, "Invalid hex format (use 8, 10, 16, 24, or 32)", GTK_MESSAGE_WARNING);
    }
  }
}

/**
 * @brief Toggle index display in hex mode
 */
static void on_gtkterm_show_index (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  /* Toggle show_index for both VTE terminal and DataView */
  gtkterm_terminal_set_show_index (window->terminal_window, true);

  if (window->dataview) {
    GtkTermDataViewOptions options;
    gtkterm_dataview_get_options (window->dataview, &options);
    options.show_index = !options.show_index;
    gtkterm_dataview_set_options (window->dataview, &options);

    if (options.show_index) {
      gtkterm_show_infobar (window, "Byte index/offset display enabled", GTK_MESSAGE_INFO);
    } else {
      gtkterm_show_infobar (window, "Byte index/offset display disabled", GTK_MESSAGE_INFO);
    }
  } else {
    gtkterm_show_infobar (window, "Show index toggled", GTK_MESSAGE_INFO);
  }
}

/**
 * @brief Send hexadecimal data dialog
 */
static void on_gtkterm_send_hex_data (GSimpleAction *action,
               GVariant      *parameter,
               gpointer       user_data) {
  /** \todo Implement send hex data dialog */
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gtkterm_show_infobar (window, "Send hex data dialog not yet implemented", GTK_MESSAGE_INFO);
}

/** \todo rewrite for GTKTerm */
static void on_gtkterm_toggle_state (GSimpleAction *action,
                 GVariant      *parameter,
                 gpointer       user_data) {
  GVariant *state;

//  show_action_dialog (action);

  state = g_action_get_state (G_ACTION (action));
  g_action_change_state (G_ACTION (action), g_variant_new_boolean (!g_variant_get_boolean (state)));
  g_variant_unref (state);
}

/** \todo rewrite for GTKTerm */
static void on_gtkterm_toggle_radio (GSimpleAction *action,
                GVariant      *parameter,
                gpointer       user_data) {

//  show_action_infobar (action, parameter, user_data);

  g_action_change_state (G_ACTION (action), parameter);
}

/**
 * @brief  Show the About dialog
 * 
 * @param action Not used.
 * 
 * @param parameter Not used.
 * 
 * @param user_data Pointer to the GtkTermWindow.
 * 
 */
static void on_gtkterm_about (GSimpleAction *action,
                GVariant      *parameter,
                gpointer       user_data) {

  GtkWidget *window = user_data;
  char *os_name;
  char *os_version;
  GString *s;

  const char *authors[] = {
    "Julien Schimtt", 
    "Zach Davis", 
    "Florian Euchner", 
    "Stephan Enderlein",
    "Kevin Picot",
    "Jens Georg",
    NULL};

  const char *comments =  _("GTKTerm is a simple GTK+ terminal used to communicate with the serial port.");

  s = g_string_new ("");

  os_name = g_get_os_info (G_OS_INFO_KEY_NAME);
  os_version = g_get_os_info (G_OS_INFO_KEY_VERSION_ID);

  if (os_name && os_version)
    g_string_append_printf (s, "OS\t%s %s\n\n", os_name, os_version);
  
  g_string_append (s, "System libraries\n");
  g_string_append_printf (s, "\tGLib\t%d.%d.%d\n",
                          glib_major_version,
                          glib_minor_version,
                          glib_micro_version);
  
  g_string_append_printf (s, "\tPango\t%s\n", pango_version_string ());
  g_string_append_printf (s, "\tGTK \t%d.%d.%d\n", gtk_get_major_version (),
                          gtk_get_minor_version (), gtk_get_micro_version ());

  GdkTexture *logo = gdk_texture_new_from_resource ("/com/github/jeija/gtkterm/gtkterm_48x48.png");

  gtk_show_about_dialog (GTK_WINDOW (window),
                         "program-name", "GTKTerm",
                         "version", PACKAGE_VERSION,
	                       "copyright", "Copyright © Julien Schimtt",
	                       "license-type", GTK_LICENSE_LGPL_3_0, 
	                       "website", "https://github.com/Jeija/gtkterm",
	                       "website-label", "https://github.com/Jeija/gtkterm",
                         "comments", "Program to demonstrate GTK functions.",
                         "authors", authors,
                         "logo-icon-name", "com.github.jeija.gtkterm",
                         "title", "GTKTerm",
                        "comments", comments,
	                       "logo", logo,
                         "system-information", s->str,
                         NULL);
}

/**
 * @brief Set the serial signals (DTR etc) in the status bar
 * 
 * @param window The GtkTermWindow with the statusbar.
 * 
 * @param port_signals The port signals to set.
 * 
 * @param user_data Not used.
 * 
 */
static void gtkterm_window_set_signals (GtkTermWindow *window, unsigned int port_signals, gpointer user_data) {

    for (int i = 0; i < SERIAL_SIGNALS; i++) {

        bool active = (port_signals & signal_flags[i]) != 0;
        gtk_widget_set_sensitive (window->status_serial_signal[i], active);
    }
}

/**
 * @brief Set the statusbar with all relevant fields.
 * 
 * @param window The GtkTermWindow with the statusbar.
 * 
 */
void config_status_bar (GtkTermWindow *window) {
    GtkWidget *widget;

    /** Fields for the configuration and port */
    for (int i = 0; i < 3; i++) {
        if (i == 0) {
            /* Make profile name clickable */
            widget = gtk_button_new_with_label ("");
            gtk_button_set_has_frame (GTK_BUTTON(widget), FALSE);
            g_signal_connect (widget, "clicked", G_CALLBACK(on_profile_button_clicked), window);
        } else {
            widget = gtk_label_new ("");
        }
        gtk_box_append (GTK_BOX (window->status_config), widget);
        gtk_widget_set_halign (GTK_WIDGET (widget), GTK_ALIGN_START);
        window->status_config_message[i] = widget;
    }

    /**
     * Fill in the serial signals
     * DTR and RTS are clickable (output signals we can control)
     * CTS, CD, DSR, RI are read-only (input signals)
     */
    for (int i = 0; i < 6; i++) {
        if (i == 0 || i == 1) {  // DTR and RTS
            widget = gtk_button_new_with_label (serial_signal[i]);
            gtk_button_set_has_frame (GTK_BUTTON(widget), FALSE);
            g_object_set_data (G_OBJECT(widget), "signal-index", GINT_TO_POINTER(i));
            g_signal_connect (widget, "clicked", G_CALLBACK(on_signal_button_clicked), window);
        } else {
            widget = gtk_label_new (serial_signal[i]);
        }
        gtk_box_append (GTK_BOX (window->statusbox), widget);
        gtk_widget_set_sensitive (GTK_WIDGET (widget), FALSE);
        gtk_widget_set_halign (GTK_WIDGET (widget), GTK_ALIGN_END);
        window->status_serial_signal[i] = widget;
    }
}

/**
 * @brief Updates the statusbar with the active terminal configuration.
 * 
 * @param window The GtkTermWindow with the statusbar.
 * 
 * @param section The active section of the terminal
 * 
 * @param serial_config_string The connectionstring of the serial port
 * 
 * @param serial_status The status of the serial port.
 * 
 */
static void update_statusbar (GtkTermWindow *window, gpointer section, gpointer serial_config_string, gpointer serial_status) {
  char *msg;

  msg = g_strdup_printf ("[%s]", (char *)section);

  /* First element is a button, update its label */
  gtk_button_set_label (GTK_BUTTON(window->status_config_message[0]), msg);

  /* Other elements are labels */
  gtk_label_set_label (GTK_LABEL(window->status_config_message[1]), serial_config_string);
  gtk_label_set_label (GTK_LABEL(window->status_config_message[2]), serial_status);

  g_free (msg);
}

/**
 * @brief Sets title of the window.
 * 
 * The title of the window is concatenated with the serial options
 * of the active terminal window.
 * 
 * @param window The GtkTermWindow for which we set the title
 * 
 * @param serial_config_string The connectionstring of the serial port.
 * 
 */
void set_window_title (GtkTermWindow *window, gpointer serial_config_string) {

  char *msg;

  msg = g_strdup_printf ("GtkTerm - %s", (char *)serial_config_string);

  gtk_window_set_title (GTK_WINDOW(window), msg);

  g_free (msg);  
}

/**
 * @brief Update action states based on port status
 *
 * @param window The GtkTermWindow
 *
 * @param port_status The current port status
 */
static void gtkterm_window_update_actions (GtkTermWindow *window, GtkTermSerialPortState port_status) {
  bool port_connected = (port_status == GTKTERM_SERIAL_PORT_CONNECTED);
  GSimpleAction *action;

  // Actions that require a connected port
  const char *requires_connection[] = {
    "send_raw_file",
    "save_raw_file",
    "send_break",
    "close_port",
    "toggle_DTR",
    "toggle_RTS",
    "send_hex_data",
    NULL
  };

  // Enable/disable actions based on connection state
  for (int i = 0; requires_connection[i] != NULL; i++) {
    action = G_SIMPLE_ACTION(g_action_map_lookup_action(G_ACTION_MAP(window->action_group), requires_connection[i]));
    if (action) {
      g_simple_action_set_enabled(action, port_connected);
    }
  }

  // Open port action should be enabled when NOT connected
  action = G_SIMPLE_ACTION(g_action_map_lookup_action(G_ACTION_MAP(window->action_group), "open_port"));
  if (action) {
    g_simple_action_set_enabled(action, !port_connected);
  }

  // These actions should always be enabled
  const char *always_enabled[] = {
    "clear_screen",
    "clear_scrollback",
    "copy",
    "paste",
    "select_all",
    "config_port",
    "config_main_window",
    "local_echo",
    "crlf_auto",
    "timestamp",
    "macro",
    "load_config",
    "save_config",
    "remove_config",
    "log_to_file",
    "log_resume",
    "log_stop",
    "log_clear",
    "show_ascii",
    "show_hex",
    "show_binary",
    "show_decimal",
    "view_hex",
    "show_index",
    "search",
    NULL
  };

  for (int i = 0; always_enabled[i] != NULL; i++) {
    action = G_SIMPLE_ACTION(g_action_map_lookup_action(G_ACTION_MAP(window->action_group), always_enabled[i]));
    if (action) {
      g_simple_action_set_enabled(action, true);
    }
  }
}

/**
 * @brief Callback for port status changes
 *
 * @param serial_port The serial port object
 *
 * @param pspec Parameter spec (not used)
 *
 * @param user_data The GtkTermWindow
 */
static void on_port_status_changed (GObject *serial_port, GParamSpec *pspec, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW(user_data);
  GtkTermSerialPort *port = GTKTERM_SERIAL_PORT(serial_port);
  GtkTermSerialPortState status = gtkterm_serial_port_get_status(port);

  gtkterm_window_update_actions(window, status);
  gtkterm_window_update_ui_state(window);
}

/**
 * @brief Scan for available serial ports
 *
 * @return GPtrArray* NULL-terminated array of port names for dropdown
 */
static GPtrArray *gtkterm_window_scan_ports (void) {
  GPtrArray *ports = g_ptr_array_new();
  GDir *dir;
  const char *filename;

  /* Scan /dev for serial devices */
  dir = g_dir_open("/dev", 0, NULL);
  if (dir) {
    while ((filename = g_dir_read_name(dir)) != NULL) {
      /* Check for common serial port patterns */
      if (g_str_has_prefix(filename, "ttyUSB") ||
          g_str_has_prefix(filename, "ttyACM") ||
          g_str_has_prefix(filename, "ttyS")) {
        char *full_path = g_strdup_printf("/dev/%s", filename);
        g_ptr_array_add(ports, full_path);
      }
    }
    g_dir_close(dir);
  }

  /* Scan /dev/pts for virtual PTY devices (for testing with simulators) */
  dir = g_dir_open("/dev/pts", 0, NULL);
  if (dir) {
    while ((filename = g_dir_read_name(dir)) != NULL) {
      /* Only include numeric PTY devices (skip ptmx and other special files) */
      if (g_ascii_isdigit(filename[0])) {
        char *full_path = g_strdup_printf("/dev/pts/%s", filename);
        /* Check if the device is accessible (readable and writable) */
        if (access(full_path, R_OK | W_OK) == 0) {
          g_ptr_array_add(ports, full_path);
        } else {
          g_free(full_path);
        }
      }
    }
    g_dir_close(dir);
  }

  /* If no ports found, add a default entry */
  if (ports->len == 0) {
    g_ptr_array_add(ports, g_strdup("/dev/ttyUSB0"));
  }

  /* NULL-terminate the array for gtk_drop_down_new_from_strings */
  g_ptr_array_add(ports, NULL);

  return ports;
}

/**
 * @brief Handle connect/disconnect button click
 *
 * @param button The button that was clicked
 *
 * @param user_data The GtkTermWindow
 */
static void on_connect_button_clicked (GtkButton *button, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW(user_data);

  g_print("DEBUG: Connect button clicked\n");

  if (!window->terminal_window) {
    g_print("DEBUG: No terminal window!\n");
    return;
  }

  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port(window->terminal_window);
  if (!serial_port) {
    g_print("DEBUG: No serial port object!\n");
    return;
  }

  GtkTermSerialPortState status = gtkterm_serial_port_get_status(serial_port);
  g_print("DEBUG: Current port status: %d\n", status);

  if (status == GTKTERM_SERIAL_PORT_CONNECTED) {
    /* Disconnect */
    g_print("DEBUG: Disconnecting port...\n");
    gtkterm_serial_port_close_port(serial_port);
  } else {
    /* Connect - get selected port and baudrate */
    guint port_idx = gtk_drop_down_get_selected(GTK_DROP_DOWN(window->port_dropdown));
    guint baud_idx = gtk_drop_down_get_selected(GTK_DROP_DOWN(window->baud_dropdown));

    g_print("DEBUG: Selected port index: %u, baud index: %u\n", port_idx, baud_idx);

    /* Get port name from dropdown using GtkStringList */
    GtkStringList *port_list = GTK_STRING_LIST(gtk_drop_down_get_model(GTK_DROP_DOWN(window->port_dropdown)));
    const char *port_name = gtk_string_list_get_string(port_list, port_idx);
    if (!port_name) {
      g_print("DEBUG: No port name from dropdown, using default\n");
      port_name = "/dev/ttyUSB0";
    }

    g_print("DEBUG: Port name: %s\n", port_name);

    /* Baudrate values */
    int baudrates[] = {300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200,
                       230400, 460800, 500000, 576000, 921600, 1000000, 2000000};
    int baudrate = (baud_idx < 17) ? baudrates[baud_idx] : 115200;

    g_print("DEBUG: Baudrate: %d\n", baudrate);

    /* Get current config and update it */
    port_config_t *config = gtkterm_serial_port_get_config(serial_port);
    if (config) {
      g_print("DEBUG: Got config, updating...\n");
      /* Update port and baudrate */
      if (g_strcmp0(config->port, port_name) != 0) {
        g_free(config->port);
        config->port = g_strdup(port_name);
      }
      config->baudrate = baudrate;

      g_print("DEBUG: Config - port: %s, baudrate: %ld, bits: %d, stopbits: %d, parity: %d\n",
              config->port, (long)config->baudrate, config->bits, config->stopbits, config->parity);

      /* Try to open the port directly */
      g_print("DEBUG: Calling gtkterm_serial_port_open_port...\n");
      int result = gtkterm_serial_port_open_port(serial_port);
      g_print("DEBUG: gtkterm_serial_port_open_port returned %d\n", result);

      if (result != true) {
        GError *error = gtkterm_serial_port_get_error(serial_port);
        char *msg = g_strdup_printf("Failed to connect: %s",
                                    error ? error->message : "Unknown error");
        g_print("DEBUG: Connection failed: %s\n", msg);
        gtkterm_show_infobar(window, msg, GTK_MESSAGE_ERROR);
        g_free(msg);
      } else {
        g_print("DEBUG: Connection successful!\n");
        char *msg = g_strdup_printf("Connected to %s at %d baud", port_name, baudrate);
        gtkterm_show_infobar(window, msg, GTK_MESSAGE_INFO);
        g_free(msg);
      }
    } else {
      g_print("DEBUG: No config available!\n");
    }
  }
}

/**
 * @brief Update UI state based on connection status
 *
 * @param window The GtkTermWindow
 */
static void gtkterm_window_update_ui_state (GtkTermWindow *window) {
  if (!window->terminal_window || !window->connect_button) {
    return;
  }

  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port(window->terminal_window);
  if (!serial_port) {
    return;
  }

  GtkTermSerialPortState status = gtkterm_serial_port_get_status(serial_port);
  bool connected = (status == GTKTERM_SERIAL_PORT_CONNECTED);

  /* Update button label */
  gtk_button_set_label(GTK_BUTTON(window->connect_button), connected ? "Disconnect" : "Connect");

  /* Update status label */
  if (window->status_label) {
    const char *status_text = connected ? "Connected" : "Disconnected";
    gtk_label_set_text(GTK_LABEL(window->status_label), status_text);
  }

  /* Enable/disable port and baud dropdowns */
  gtk_widget_set_sensitive(window->port_dropdown, !connected);
  gtk_widget_set_sensitive(window->baud_dropdown, !connected);
}

/**
 * @brief Create and setup the header bar
 *
 * @param window The GtkTermWindow
 */
static void create_header_bar (GtkTermWindow *window) {
  /* Create header bar */
  window->header_bar = gtk_header_bar_new();
  gtk_window_set_titlebar(GTK_WINDOW(window), window->header_bar);

  /* Create a box for connection controls */
  GtkWidget *control_box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
  gtk_widget_set_margin_start(control_box, 6);
  gtk_widget_set_margin_end(control_box, 6);

  /* Port label and dropdown */
  GtkWidget *port_label = gtk_label_new("Port:");
  gtk_box_append(GTK_BOX(control_box), port_label);

  /* Scan for ports and create dropdown */
  GPtrArray *ports = gtkterm_window_scan_ports();
  window->port_dropdown = gtk_drop_down_new_from_strings((const char * const *)ports->pdata);
  gtk_drop_down_set_selected(GTK_DROP_DOWN(window->port_dropdown), 0);
  gtk_box_append(GTK_BOX(control_box), window->port_dropdown);

  /* Free the port array (dropdown has copied the strings) */
  for (guint i = 0; i < ports->len - 1; i++) {
    g_free(g_ptr_array_index(ports, i));
  }
  g_ptr_array_free(ports, TRUE);

  /* Baud rate label and dropdown */
  GtkWidget *baud_label = gtk_label_new("Baud:");
  gtk_box_append(GTK_BOX(control_box), baud_label);

  const char *baudrates[] = {"300", "600", "1200", "2400", "4800", "9600", "19200", "38400",
                              "57600", "115200", "230400", "460800", "500000", "576000",
                              "921600", "1000000", "2000000", NULL};
  window->baud_dropdown = gtk_drop_down_new_from_strings(baudrates);
  gtk_drop_down_set_selected(GTK_DROP_DOWN(window->baud_dropdown), 9); // 115200
  gtk_box_append(GTK_BOX(control_box), window->baud_dropdown);

  /* Connect button */
  window->connect_button = gtk_button_new_with_label("Connect");
  gtk_widget_add_css_class(window->connect_button, "suggested-action");
  g_signal_connect(window->connect_button, "clicked", G_CALLBACK(on_connect_button_clicked), window);
  gtk_box_append(GTK_BOX(control_box), window->connect_button);

  /* Status label */
  window->status_label = gtk_label_new("Disconnected");
  gtk_widget_add_css_class(window->status_label, "dim-label");
  gtk_box_append(GTK_BOX(control_box), window->status_label);

  /* Add control box to header bar */
  gtk_header_bar_pack_start(GTK_HEADER_BAR(window->header_bar), control_box);

  /* Create view mode selector box */
  GtkWidget *view_box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
  gtk_widget_set_margin_start(view_box, 12);
  gtk_widget_set_margin_end(view_box, 6);

  GtkWidget *view_label = gtk_label_new("View:");
  gtk_box_append(GTK_BOX(view_box), view_label);

  const char *view_modes[] = {"ASCII", "Hexadecimal", "Binary", "Decimal", "Mixed (Hex+ASCII)", NULL};
  window->view_mode_dropdown = gtk_drop_down_new_from_strings(view_modes);
  gtk_drop_down_set_selected(GTK_DROP_DOWN(window->view_mode_dropdown), 0); // ASCII by default
  g_signal_connect(window->view_mode_dropdown, "notify::selected", G_CALLBACK(on_view_mode_dropdown_changed), window);
  gtk_box_append(GTK_BOX(view_box), window->view_mode_dropdown);

  gtk_header_bar_pack_start(GTK_HEADER_BAR(window->header_bar), view_box);

  /* Keep the menu button on the right */
  if (window->menubutton) {
    /* Unparent the menubutton from its current parent in the template */
    gtk_widget_unparent(window->menubutton);
    gtk_header_bar_pack_end(GTK_HEADER_BAR(window->header_bar), window->menubutton);
  }
}

/**
 * @brief Callbackfunction for updating window title and statusbus
 *
 * @param window The GtkTermWindow which we update
 *
 * @param section The active section of the terminal
 *
 * @param serial_config_string The connectionstring of the serial port
 *
 * @param serial_status The status of the serial port.
 *
 * @param user_data Not used.
 *
 */
static void gtkterm_window_update_statusbar (GtkTermWindow *window, gpointer section, gpointer serial_config_string,
                                            gpointer serial_status, gpointer user_data) {

  set_window_title (window, serial_config_string);
  update_statusbar (window, section, serial_config_string, serial_status);
}

/**
 * @brief Toggles the dark mode setting.
 * 
 * @param action The action interface.
 * 
 * @param state The new dark-mode setting
 * 
 * @param user_data Not used.
 * 
 */
static void on_gtkterm_toggle_dark (GSimpleAction *action,
                    GVariant      *state,
                    gpointer       user_data) {

  GtkSettings *settings = gtk_settings_get_default ();

  g_object_set (G_OBJECT (settings),
                "gtk-application-prefer-dark-theme",
                g_variant_get_boolean (state),
                NULL);

  g_simple_action_set_state (action, state);
}

/**
 * @brief Toggles the Sunshine visualization panel
 *
 * Shows or hides the visualization panel and adjusts the paned position
 */
static void on_gtkterm_toggle_visualization (GSimpleAction *action,
                                             GVariant      *state,
                                             gpointer       user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  gboolean show = g_variant_get_boolean (state);

  window->visualization_visible = show;

  if (show) {
    /* Show the visualization panel */
    gtk_widget_set_visible (window->visualization_panel, TRUE);
    /* Set paned position to show both terminal and visualization */
    gtk_paned_set_position (GTK_PANED (window->main_paned), 700);

    /* Create Sunshine manager if not already created */
    if (!window->sunshine_manager) {
      GtkTermBuffer *buffer = gtkterm_terminal_get_buffer(window->terminal_window);
      window->sunshine_manager = gtkterm_sunshine_manager_new(
          window->visualization_panel, buffer);

      g_print("Sunshine: Manager created\n");
    }

    /* Scan buffer history for Sunshine packets */
    gtkterm_sunshine_manager_scan_history(window->sunshine_manager);
  } else {
    /* Hide the visualization panel */
    gtk_widget_set_visible (window->visualization_panel, FALSE);
    /* Give full space to terminal */
    gtk_paned_set_position (GTK_PANED (window->main_paned),
                            gtk_widget_get_width (window->main_paned));
  }

  g_simple_action_set_state (action, state);
}

/**
 * @brief Toggles the radio option in the menubar
 * 
 * @param action The action interface.
 * 
 * @param state The new radio setting
 * 
 * @param user_data Not used.
 * 
 */
static void on_gtkterm_toggle_radio_state (GSimpleAction *action,
                    GVariant      *state,
                    gpointer       user_data) {

  g_simple_action_set_state (action, state);
}

/**
 * @brief Callback when profile button is clicked in footer
 */
static void on_profile_button_clicked (GtkButton *button, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);

  g_print("DEBUG: Profile button clicked\n");

  /* Show a popup menu with profile options */
  GtkWidget *popover = gtk_popover_new ();
  gtk_widget_set_parent (popover, GTK_WIDGET(button));

  GtkWidget *box = gtk_box_new (GTK_ORIENTATION_VERTICAL, 6);
  gtk_widget_set_margin_start (box, 12);
  gtk_widget_set_margin_end (box, 12);
  gtk_widget_set_margin_top (box, 12);
  gtk_widget_set_margin_bottom (box, 12);

  /* Add menu items */
  GtkWidget *load_btn = gtk_button_new_with_label ("Load Configuration...");
  GtkWidget *save_btn = gtk_button_new_with_label ("Save Configuration...");
  GtkWidget *delete_btn = gtk_button_new_with_label ("Delete Configuration...");

  gtk_box_append (GTK_BOX(box), load_btn);
  gtk_box_append (GTK_BOX(box), save_btn);
  gtk_box_append (GTK_BOX(box), delete_btn);

  /* Connect signals to trigger existing menu actions */
  gtk_actionable_set_action_name (GTK_ACTIONABLE(load_btn), "gtkterm_window.load_config");
  gtk_actionable_set_action_name (GTK_ACTIONABLE(save_btn), "gtkterm_window.save_config");
  gtk_actionable_set_action_name (GTK_ACTIONABLE(delete_btn), "gtkterm_window.remove_config");

  /* Close popover when any button is clicked */
  g_signal_connect_swapped (load_btn, "clicked", G_CALLBACK(gtk_popover_popdown), popover);
  g_signal_connect_swapped (save_btn, "clicked", G_CALLBACK(gtk_popover_popdown), popover);
  g_signal_connect_swapped (delete_btn, "clicked", G_CALLBACK(gtk_popover_popdown), popover);

  gtk_popover_set_child (GTK_POPOVER(popover), box);
  gtk_popover_popup (GTK_POPOVER(popover));
}

/**
 * @brief Callback when DTR/RTS signal button is clicked in footer
 */
static void on_signal_button_clicked (GtkButton *button, gpointer user_data) {
  GtkTermWindow *window = GTKTERM_WINDOW (user_data);
  int signal_index = GPOINTER_TO_INT(g_object_get_data (G_OBJECT(button), "signal-index"));

  g_print("DEBUG: Signal button clicked: %s (index %d)\n", serial_signal[signal_index], signal_index);

  GtkTermSerialPort *serial_port = gtkterm_terminal_get_serial_port (window->terminal_window);
  if (!serial_port) {
    g_print("DEBUG: No serial port available\n");
    return;
  }

  GtkTermSerialPortState status = gtkterm_serial_port_get_status(serial_port);
  if (status != GTKTERM_SERIAL_PORT_CONNECTED) {
    gtkterm_show_infobar (window, "Port must be connected to toggle signals", GTK_MESSAGE_WARNING);
    return;
  }

  /* Toggle the signal */
  if (signal_index == 0) {
    /* DTR */
    gtkterm_serial_port_toggle_DTR (serial_port);
    g_print("DEBUG: Toggled DTR\n");
  } else if (signal_index == 1) {
    /* RTS */
    gtkterm_serial_port_toggle_RTS (serial_port);
    g_print("DEBUG: Toggled RTS\n");
  }
}

/** To be implemented */
static void clicked_cb (GtkWidget *widget, GtkTermWindow *window) {

  gtk_widget_hide (window->infobar);
}

/**
 * @brief Stores the setting of the window in the Gnome Settings.
 * 
 * @param window The window we store the settings for.
 * 
 */
static void gtkterm_window_store_state (GtkTermWindow *window) {

  GSettings *settings;

  settings = g_settings_new ("com.github.jeija.gtkterm");
  g_settings_set (settings, "window-size", "(ii)", window->width, window->height);
  g_settings_set_boolean (settings, "maximized", window->maximized);
  g_settings_set_boolean (settings, "fullscreen", window->fullscreen);
  g_object_unref (settings);
}

/**
 * @brief Loads the setting of the window from the Gnome Settings.
 * 
 * @param window The window we load the settings for.
 * 
 */
static void gtkterm_window_load_state (GtkTermWindow *window) {
  GSettings *settings;

  settings = g_settings_new ("com.github.jeija.gtkterm");
  g_settings_get (settings, "window-size", "(ii)", &window->width, &window->height);
  window->maximized = g_settings_get_boolean (settings, "maximized");
  window->fullscreen = g_settings_get_boolean (settings, "fullscreen");
  g_object_unref (settings);
}

/**
 * @brief Initialize the window with the actions, tools etc.
 *
 * @param window The window we are initializing.
 *
 */
static void gtkterm_window_init (GtkTermWindow *window) {

  GtkWidget *popover;
  GtkBuilder *builder;
  GMenuModel *menu;

  window->width = -1;
  window->height = -1;
  window->maximized = FALSE;
  window->fullscreen = FALSE;
  window->visualization_visible = FALSE;
  window->sunshine_manager = NULL;
  window->logging = gtkterm_logging_new ();
  window->macro_manager = gtkterm_macro_manager_new ();

  /* Load macros from file if it exists */
  char *config_dir = g_build_filename (g_get_user_config_dir (), "gtkterm", NULL);
  char *macros_file = g_build_filename (config_dir, "macros.json", NULL);
  if (g_file_test (macros_file, G_FILE_TEST_EXISTS)) {
    GError *error = NULL;
    if (!gtkterm_macro_manager_load_from_file (window->macro_manager, macros_file, &error)) {
      g_warning ("Failed to load macros from %s: %s", macros_file, error->message);
      g_error_free (error);
    }
  }
  g_free (macros_file);
  g_free (config_dir);

  gtk_widget_init_template (GTK_WIDGET (window));

  /* Create header bar with connection controls */
  create_header_bar(window);

  /* Load the actual menu from menu.ui */
  builder = gtk_builder_new ();
  gtk_builder_add_from_resource (builder, "/com/github/jeija/gtkterm/menu.ui", NULL);
  menu = G_MENU_MODEL (gtk_builder_get_object (builder, "gtkterm_menubar"));

  popover = gtk_popover_menu_new_from_model (menu);
  gtk_menu_button_set_popover (GTK_MENU_BUTTON (window->menubutton), popover);

  g_object_unref (builder);

  /* Connect search widget signals */
  g_signal_connect (window->search_entry, "changed", G_CALLBACK (on_search_entry_changed), window);
  g_signal_connect (window->search_next, "clicked", G_CALLBACK (on_search_next_clicked), window);
  g_signal_connect (window->search_prev, "clicked", G_CALLBACK (on_search_prev_clicked), window);
  g_signal_connect (window->search_close, "clicked", G_CALLBACK (on_search_close_clicked), window);

  window->action_group = G_ACTION_GROUP(g_simple_action_group_new ());

  /** \todo: Rename it */
  g_action_map_add_action_entries (G_ACTION_MAP (window->action_group),
                                   win_entries,
                                   G_N_ELEMENTS (win_entries),
                                   window);
  gtk_widget_insert_action_group (GTK_WIDGET (window), "gtkterm_window", window->action_group);
  g_action_map_add_action_entries (G_ACTION_MAP (window->action_group),
                                   gtkterm_window_entries,
                                   G_N_ELEMENTS (gtkterm_window_entries),
                                   window);

  /* Initialize actions with proper enabled states (disconnected state) */
  gtkterm_window_update_actions(window, GTKTERM_SERIAL_PORT_DISCONNECTED);

  /*
   *  This means we have to do 'it by hand not from the UI' and store the GtkLabels for later use.
   */
  config_status_bar (window);
}

/**
 * @brief Constructs the window.
 * 
 * @param object The window object we are constructing.
 * 
 */
static void gtkterm_window_constructed (GObject *object) {
  GtkTermWindow *window = (GtkTermWindow *)object;

  gtkterm_window_load_state (window);

  gtk_window_set_default_size (GTK_WINDOW (window), window->width, window->height); 

  /** 
   * Create a new terminal window and send section and keyfile as parameter
   * GTKTERM_TERMINAL then can load the right section.
   */
  //window->terminal_window = gtkterm_terminal_new (GTKTERM_APP(app)->section, GTKTERM_APP(app), window);

  /** Make the VTE window scrollable */
  //gtk_scrolled_window_set_child(window->scrolled_window, GTK_WIDGET(window->terminal_window));  

  /** Connect to the terminal_changed so we can update the statusbar and window title */
  g_signal_connect (window, "terminal-changed", G_CALLBACK(gtkterm_window_update_statusbar), NULL);	
  g_signal_connect (window, "serial-signals-changed", G_CALLBACK(gtkterm_window_set_signals), NULL);	   

  if (window->maximized)
    gtk_window_maximize (GTK_WINDOW (window));

  if (window->fullscreen)
    gtk_window_fullscreen (GTK_WINDOW (window));

  G_OBJECT_CLASS (gtkterm_window_parent_class)->constructed (object);
}

/**
 * @brief Assigns size an position to the widget.
 * 
 * @param widget The widget.
 * 
 * @param width The width of the widget.
 *  
 * @param height The height of the widget.
 * 
 * @param baseline  The baseline for this widget.
 * 
 */
static void gtkterm_window_size_allocate (GtkWidget *widget,
                                       int width,
                                       int height,
                                       int baseline) {
                                        
  GtkTermWindow *window = (GtkTermWindow *)widget; 

  GTK_WIDGET_CLASS (gtkterm_window_parent_class)->size_allocate (widget,
                                                                  width,
                                                                  height,
                                                                  baseline);

  if (!window->maximized && !window->fullscreen)
    gtk_window_get_default_size (GTK_WINDOW (window), &window->width, &window->height);
}

/**
 * @brief Called when the surface state is changed (min/max).
 * 
 * @param widget The widget.
 * 
 */
static void surface_state_changed (GtkWidget *widget) {
  GtkTermWindow *window = (GtkTermWindow *)widget;
  GdkToplevelState new_state;

  new_state = gdk_toplevel_get_state (GDK_TOPLEVEL (gtk_native_get_surface (GTK_NATIVE (widget))));
  window->maximized = (new_state & GDK_TOPLEVEL_STATE_MAXIMIZED) != 0;
  window->fullscreen = (new_state & GDK_TOPLEVEL_STATE_FULLSCREEN) != 0;
}

/**
 * @brief Windows realize.
 * 
 * Operations to finish realizing the widget.
 * 
 * @param widget The widget.
 * 
 */
static void gtkterm_window_realize (GtkWidget *widget) {
  GTK_WIDGET_CLASS (gtkterm_window_parent_class)->realize (widget);

  g_signal_connect_swapped (gtk_native_get_surface (GTK_NATIVE (widget)), "notify::state",
                            G_CALLBACK (surface_state_changed), widget);
}

/**
 * @brief Windows unrealize.
 * 
 * Operations when removing the widget.
 * 
 * @param widget The widget.
 * 
 */
static void gtkterm_window_unrealize (GtkWidget *widget) {
  g_signal_handlers_disconnect_by_func (gtk_native_get_surface (GTK_NATIVE (widget)),
                                        surface_state_changed, widget);

  GTK_WIDGET_CLASS (gtkterm_window_parent_class)->unrealize (widget);
}

/**
 * @brief Called when distroying the window
 * 
 * This is used to clean up an freeing the variables in the window structure.
 * 
 * @param object The object.
 * 
 */
static void gtkterm_window_dispose (GObject *object) {
  GtkTermWindow *window = (GtkTermWindow *)object;

  gtkterm_window_store_state (window);

  /* Release the strong references we took on the view widgets */
  if (window->terminal_window) {
    g_object_unref (window->terminal_window);
    window->terminal_window = NULL;
  }
  if (window->dataview) {
    g_object_unref (window->dataview);
    window->dataview = NULL;
  }

  /* Release Sunshine manager */
  if (window->sunshine_manager) {
    g_object_unref (window->sunshine_manager);
    window->sunshine_manager = NULL;
  }

  G_OBJECT_CLASS (gtkterm_window_parent_class)->dispose (object);
}

/**
 * @brief Initializing the window class
 * 
 * Setting the signals, the UI and callback functions
 * 
 * @param class The window class
 * 
 */
static void gtkterm_window_class_init (GtkTermWindowClass *class) {
  GObjectClass *object_class = G_OBJECT_CLASS (class);
  GtkWidgetClass *widget_class = GTK_WIDGET_CLASS (class);

  object_class->constructed = gtkterm_window_constructed;
  object_class->dispose = gtkterm_window_dispose;

  widget_class->size_allocate = gtkterm_window_size_allocate;
  widget_class->realize = gtkterm_window_realize;
  widget_class->unrealize = gtkterm_window_unrealize;

  gtkterm_signals[SIGNAL_GTKTERM_TERMINAL_CHANGED] = g_signal_new ("terminal-changed",
                                            GTKTERM_TYPE_GTKTERM_WINDOW,
                                            G_SIGNAL_RUN_FIRST,
                                            0,
                                            NULL,
                                            NULL,
                                            NULL,
                                            G_TYPE_NONE,
                                            3,
                                            G_TYPE_POINTER,
                                            G_TYPE_POINTER,
                                            G_TYPE_POINTER,
                                            NULL);

  gtkterm_signals[SIGNAL_GTKTERM_SERIAL_SIGNALS_CHANGED] = g_signal_new ("serial-signals-changed",
                                            GTKTERM_TYPE_GTKTERM_WINDOW,
                                            G_SIGNAL_RUN_FIRST,
                                            0,
                                            NULL,
                                            NULL,
                                            NULL,
                                            G_TYPE_NONE,
                                            1,
                                            G_TYPE_INT,
                                            NULL);                                            

  gtk_widget_class_set_template_from_resource (widget_class, "/com/github/jeija/gtkterm/gtkterm_main.ui");
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, message);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, menubutton);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, toolmenu);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, infobar);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, main_paned);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, scrolled_window);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, visualization_panel);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, statusbox);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, status_config);

  /* Bind search widgets */
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_revealer);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_bar);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_entry);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_case_sensitive);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_regex);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_hex);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_prev);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_next);
  gtk_widget_class_bind_template_child (widget_class, GtkTermWindow, search_close);   

  gtk_widget_class_bind_template_callback (widget_class, clicked_cb);
}
