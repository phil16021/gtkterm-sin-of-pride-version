//! The single accelerator table: used for `set_accels_for_action` and the shortcuts dialog.
//!
//! Plain Ctrl+letter combinations belong to the device (Ctrl+C, Ctrl+A, ... are control
//! characters on a serial line), so application shortcuts use Ctrl+Shift or function keys.
//! The C version bound Ctrl+Shift+V four times and Ctrl+A/Q/F to app actions (ROADMAP K1).

/// (section, title, action, accelerator); section and title are translated where shown.
pub const SHORTCUTS: &[(&str, &str, &str, &str)] = &[
    (
        crate::n_!("Terminal"),
        crate::n_!("Copy"),
        "win.copy",
        "<Control><Shift>c",
    ),
    (
        crate::n_!("Terminal"),
        crate::n_!("Paste"),
        "win.paste",
        "<Control><Shift>v",
    ),
    (
        crate::n_!("Terminal"),
        crate::n_!("Select all"),
        "win.select-all",
        "<Control><Shift>a",
    ),
    (
        crate::n_!("Terminal"),
        crate::n_!("Search"),
        "win.search",
        "<Control><Shift>f",
    ),
    (
        crate::n_!("Terminal"),
        crate::n_!("Clear screen"),
        "win.clear-screen",
        "<Control><Shift>l",
    ),
    (
        crate::n_!("Terminal"),
        crate::n_!("Clear scrollback"),
        "win.clear-scrollback",
        "<Control><Shift>k",
    ),
    (
        crate::n_!("Connection"),
        crate::n_!("Connect / disconnect"),
        "win.toggle-connection",
        "F5",
    ),
    (
        crate::n_!("Connection"),
        crate::n_!("Disconnect"),
        "win.close-port",
        "F6",
    ),
    (
        crate::n_!("Connection"),
        crate::n_!("Toggle DTR"),
        "win.dtr",
        "F7",
    ),
    (
        crate::n_!("Connection"),
        crate::n_!("Toggle RTS"),
        "win.rts",
        "F8",
    ),
    (
        crate::n_!("Connection"),
        crate::n_!("Send break"),
        "win.send-break",
        "<Control><Shift>b",
    ),
    (
        crate::n_!("Connection"),
        crate::n_!("Preferences"),
        "win.preferences",
        "<Control><Shift>p",
    ),
    (
        crate::n_!("Data"),
        crate::n_!("Send hex data"),
        "win.send-hex",
        "<Control><Shift>h",
    ),
    (
        crate::n_!("Data"),
        crate::n_!("Send file"),
        "win.send-file",
        "<Control><Shift>r",
    ),
    (
        crate::n_!("Data"),
        crate::n_!("Receive file"),
        "win.receive-file",
        "<Control><Shift>e",
    ),
    (
        crate::n_!("Data"),
        crate::n_!("Save received data"),
        "win.save-raw",
        "<Control><Shift>s",
    ),
    (
        crate::n_!("Data"),
        crate::n_!("Start logging"),
        "win.log-start",
        "<Control><Shift>o",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Visualization panel"),
        "win.visualization",
        "<Control><Shift>g",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Decoded frames"),
        "win.frames",
        "<Control><Shift>d",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Signal monitor"),
        "win.signal-monitor",
        "<Control><Shift>m",
    ),
    (
        crate::n_!("Data"),
        crate::n_!("Scripts"),
        "win.scripts",
        "<Control><Shift>j",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Zoom in"),
        "win.zoom-in",
        "<Control>plus",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Zoom out"),
        "win.zoom-out",
        "<Control>minus",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Normal size"),
        "win.zoom-reset",
        "<Control>0",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Fullscreen"),
        "win.fullscreen",
        "F11",
    ),
    (
        crate::n_!("View"),
        crate::n_!("Keyboard shortcuts"),
        "win.shortcuts",
        "<Control>question",
    ),
    (crate::n_!("Application"), crate::n_!("About"), "win.about", "F1"),
    (
        crate::n_!("Application"),
        crate::n_!("Quit"),
        "app.quit",
        "<Control><Shift>q",
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn no_duplicates_and_no_plain_ctrl_letters() {
        let mut accels = HashSet::new();
        let mut actions = HashSet::new();
        for (_, _, action, accel) in SHORTCUTS {
            assert!(
                accels.insert(accel.to_lowercase()),
                "duplicate accelerator {accel}"
            );
            assert!(actions.insert(*action), "duplicate action {action}");
            if accel.starts_with("<Control>") && !accel.contains("<Shift>") {
                let key = accel.trim_start_matches("<Control>");
                assert!(
                    !(key.len() == 1 && key.chars().all(|c| c.is_ascii_alphabetic())),
                    "{accel} would steal a control character from the device"
                );
            }
            assert!(
                !accel.contains("<Super>space"),
                "Super+Space is reserved for the keyboard layout"
            );
        }
    }
}
