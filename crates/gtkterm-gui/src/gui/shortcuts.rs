//! The single accelerator table: used for `set_accels_for_action` and the shortcuts dialog.
//!
//! Plain Ctrl+letter combinations belong to the device (Ctrl+C, Ctrl+A, ... are control
//! characters on a serial line), so application shortcuts use Ctrl+Shift or function keys.
//! The C version bound Ctrl+Shift+V four times and Ctrl+A/Q/F to app actions (ROADMAP K1).

/// (section, title, action, accelerator)
pub const SHORTCUTS: &[(&str, &str, &str, &str)] = &[
    ("Terminal", "Copy", "win.copy", "<Control><Shift>c"),
    ("Terminal", "Paste", "win.paste", "<Control><Shift>v"),
    ("Terminal", "Select all", "win.select-all", "<Control><Shift>a"),
    ("Terminal", "Search", "win.search", "<Control><Shift>f"),
    (
        "Terminal",
        "Clear screen",
        "win.clear-screen",
        "<Control><Shift>l",
    ),
    (
        "Terminal",
        "Clear scrollback",
        "win.clear-scrollback",
        "<Control><Shift>k",
    ),
    (
        "Connection",
        "Connect / disconnect",
        "win.toggle-connection",
        "F5",
    ),
    ("Connection", "Disconnect", "win.close-port", "F6"),
    ("Connection", "Toggle DTR", "win.dtr", "F7"),
    ("Connection", "Toggle RTS", "win.rts", "F8"),
    ("Connection", "Send break", "win.send-break", "<Control><Shift>b"),
    (
        "Connection",
        "Preferences",
        "win.preferences",
        "<Control><Shift>p",
    ),
    ("Data", "Send hex data", "win.send-hex", "<Control><Shift>h"),
    ("Data", "Send file", "win.send-file", "<Control><Shift>r"),
    ("Data", "Receive file", "win.receive-file", "<Control><Shift>e"),
    ("Data", "Save received data", "win.save-raw", "<Control><Shift>s"),
    ("Data", "Start logging", "win.log-start", "<Control><Shift>o"),
    (
        "View",
        "Visualization panel",
        "win.visualization",
        "<Control><Shift>g",
    ),
    ("View", "Decoded frames", "win.frames", "<Control><Shift>d"),
    (
        "View",
        "Signal monitor",
        "win.signal-monitor",
        "<Control><Shift>m",
    ),
    ("Data", "Scripts", "win.scripts", "<Control><Shift>j"),
    ("View", "Zoom in", "win.zoom-in", "<Control>plus"),
    ("View", "Zoom out", "win.zoom-out", "<Control>minus"),
    ("View", "Normal size", "win.zoom-reset", "<Control>0"),
    ("View", "Fullscreen", "win.fullscreen", "F11"),
    ("View", "Keyboard shortcuts", "win.shortcuts", "<Control>question"),
    ("Application", "About", "win.about", "F1"),
    ("Application", "Quit", "app.quit", "<Control><Shift>q"),
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
