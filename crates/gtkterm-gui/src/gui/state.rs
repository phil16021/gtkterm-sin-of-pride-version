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
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8765,
            read_only: false,
            token: None,
            allow_exec: false,
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
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            width: 1180,
            height: 760,
            maximized: false,
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
    /// Load, falling back to defaults.
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Save (errors are logged, not fatal).
    pub fn save(&self) {
        let p = path();
        let result = toml::to_string_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|t| {
                if let Some(d) = p.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(&p, t)
            });
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
}
