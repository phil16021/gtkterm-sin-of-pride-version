//! Per-user UI state (window geometry, panel, hex history, embedded MCP settings).
//! Stored as `state.toml` next to the configuration, so no GSettings schema is needed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Embedded MCP server settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    /// Start the HTTP endpoint with the window.
    pub enabled: bool,
    /// Port on 127.0.0.1.
    pub port: u16,
    /// Expose only read-only tools.
    pub read_only: bool,
    /// Bearer token; generated on first use.
    pub token: Option<String>,
    /// Scripts run by agents may use `exec`.
    pub allow_exec: bool,
    /// Ask the user before an agent runs a write tool.
    pub confirm_writes: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8765,
            read_only: false,
            token: None,
            allow_exec: false,
            confirm_writes: true,
        }
    }
}

/// UI state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// Window width.
    pub width: i32,
    /// Window height.
    pub height: i32,
    /// Window maximized.
    pub maximized: bool,
    /// Window fullscreen.
    pub fullscreen: bool,
    /// Terminal zoom (VTE font scale).
    pub font_scale: f64,
    /// Divider position between terminal and charts.
    pub pane_position: Option<i32>,
    /// Chart panel shown.
    pub panel_visible: bool,
    /// Recently sent hex strings, newest first.
    pub hex_history: Vec<String>,
    /// Embedded MCP server.
    pub mcp: McpSettings,
    /// Show the macro button bar.
    pub macro_bar: bool,
    /// Decoded frames pane shown.
    pub frames_visible: bool,
    /// Divider position between terminal and frames pane.
    pub frames_position: Option<i32>,
    /// Last file transfer protocol (`ymodem`, ...).
    pub transfer_protocol: Option<String>,
    /// Last folder files were received into.
    pub transfer_dir: Option<PathBuf>,
    /// Scripts run from the window may use `exec`.
    pub script_allow_exec: bool,
    /// Last script file.
    pub script_path: Option<PathBuf>,
    /// Profiles open in tabs at the last exit, in tab order.
    pub open_tabs: Vec<String>,
    /// Show the send bar.
    pub send_bar: bool,
    /// Connect when the application starts (and when restoring tabs).
    pub connect_at_start: bool,
    /// Paste large or multi-line text without asking.
    pub paste_without_asking: bool,
    /// Lines sent from the send bar, newest first.
    pub send_history: Vec<String>,
    /// Line ending of the send bar (0 none, 1 CR, 2 LF, 3 CR LF).
    pub send_ending: u32,
    /// Repeat interval of the send bar in ms.
    pub send_every_ms: u32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            width: 1180,
            height: 760,
            maximized: false,
            fullscreen: false,
            font_scale: 1.0,
            pane_position: None,
            panel_visible: false,
            hex_history: Vec::new(),
            mcp: McpSettings::default(),
            macro_bar: false,
            frames_visible: false,
            frames_position: None,
            transfer_protocol: None,
            transfer_dir: None,
            script_allow_exec: false,
            script_path: None,
            open_tabs: Vec::new(),
            send_bar: false,
            connect_at_start: true,
            paste_without_asking: false,
            send_history: Vec::new(),
            send_ending: 1,
            send_every_ms: 1000,
        }
    }
}

#[cfg(test)]
thread_local! {
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Keep UI state of tests out of the real configuration directory.
#[cfg(test)]
pub fn set_test_dir(dir: &std::path::Path) {
    TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.to_owned()));
}

fn path() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|d| d.borrow().clone()) {
        return dir.join("state.toml");
    }
    gtkterm_core::config::config_dir().join("state.toml")
}

impl UiState {
    /// Load, falling back to defaults. A file that cannot be read as state is kept as
    /// `state.toml.bad` (it holds the MCP token) instead of being overwritten by the next save.
    pub fn load() -> Self {
        let p = path();
        let Ok(text) = std::fs::read_to_string(&p) else {
            return Self::default();
        };
        match toml::from_str(&text) {
            Ok(st) => st,
            Err(e) => {
                let bad = p.with_extension("toml.bad");
                tracing::warn!(path = %p.display(), error = %e, backup = %bad.display(), "UI state unreadable, using defaults");
                let _ = std::fs::rename(&p, &bad);
                Self::default()
            }
        }
    }

    /// Save atomically and readable only by the user: the file holds the MCP token.
    /// Errors are logged, not fatal.
    pub fn save(&self) {
        let p = path();
        let result = toml::to_string_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|t| gtkterm_core::fsutil::write_atomic(&p, t.as_bytes(), Some(0o600)));
        if let Err(e) = result {
            tracing::warn!(path = %p.display(), error = %e, "cannot save UI state");
        }
    }

    /// Remember a sent hex string (deduplicated, at most 20).
    pub fn remember_hex(&mut self, s: &str) {
        self.hex_history.retain(|h| h != s);
        self.hex_history.insert(0, s.to_owned());
        self.hex_history.truncate(20);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_history() {
        let mut s = UiState::default();
        for i in 0..25 {
            s.remember_hex(&format!("{i:02X}"));
        }
        s.remember_hex("05");
        assert_eq!(s.hex_history.len(), 20);
        assert_eq!(s.hex_history[0], "05");
        let back: UiState = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let partial: UiState = toml::from_str("width = 900\n[mcp]\nport = 9000\n").unwrap();
        assert_eq!(
            (partial.width, partial.height, partial.mcp.port),
            (900, 760, 9000)
        );
    }

    #[test]
    fn saved_privately() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        set_test_dir(dir.path());
        let file = dir.path().join("state.toml");
        std::fs::write(&file, "").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut s = UiState::default();
        s.mcp.token = Some("secret".into());
        s.save();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the MCP token must not be world-readable");
        assert_eq!(UiState::load().mcp.token.as_deref(), Some("secret"));
    }

    #[test]
    fn unreadable_state_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        set_test_dir(dir.path());
        std::fs::write(
            dir.path().join("state.toml"),
            "width = \"wide\"\n[mcp]\ntoken = \"keep\"\n",
        )
        .unwrap();
        assert_eq!(UiState::load(), UiState::default());
        let bad = std::fs::read_to_string(dir.path().join("state.toml.bad")).unwrap();
        assert!(bad.contains("keep"));
    }
}
