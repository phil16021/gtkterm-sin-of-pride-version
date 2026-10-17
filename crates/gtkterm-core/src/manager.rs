//! Owner of all sessions, shared by the GUI and the MCP server.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::config::{Config, ConfigError, DEFAULT_PROFILE, TerminalConfig};
use crate::session::{Session, SessionId};

/// Session list changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerEvent {
    /// A session was created.
    Added(SessionId),
    /// A session was removed.
    Removed(SessionId),
}

/// Registry of sessions plus the shared configuration.
#[derive(Debug)]
pub struct SessionManager {
    sessions: Mutex<BTreeMap<SessionId, Session>>,
    next_id: AtomicU32,
    events: broadcast::Sender<ManagerEvent>,
    config: Mutex<Config>,
    config_path: Option<PathBuf>,
}

impl SessionManager {
    /// New manager. `config_path` is where [`SessionManager::save_config`] writes; `None` keeps
    /// the configuration in memory only (tests).
    pub fn new(config: Config, config_path: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::default(),
            next_id: AtomicU32::new(1),
            events: broadcast::channel(64).0,
            config: Mutex::new(config),
            config_path,
        })
    }

    /// Subscribe to session list changes.
    pub fn subscribe(&self) -> broadcast::Receiver<ManagerEvent> {
        self.events.subscribe()
    }

    /// Create a session named `name` with the given terminal settings.
    pub fn create(&self, name: &str, terminal: TerminalConfig) -> Session {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let session = Session::spawn(id, name, terminal);
        self.sessions
            .lock()
            .expect("sessions poisoned")
            .insert(id, session.clone());
        let _ = self.events.send(ManagerEvent::Added(id));
        session
    }

    /// Create a session for a profile from the configuration.
    pub fn create_for_profile(
        &self,
        profile: &str,
    ) -> Result<(Session, crate::config::Connection), ConfigError> {
        let p = self
            .config
            .lock()
            .expect("config poisoned")
            .profile(profile)?
            .clone();
        Ok((self.create(profile, p.terminal), p.connection))
    }

    /// Session by ID.
    pub fn get(&self, id: SessionId) -> Option<Session> {
        self.sessions.lock().expect("sessions poisoned").get(&id).cloned()
    }

    /// All sessions in creation order.
    pub fn list(&self) -> Vec<Session> {
        self.sessions
            .lock()
            .expect("sessions poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// The session with the lowest ID, or a new one for the default profile.
    pub fn first_or_create(&self) -> Session {
        if let Some(s) = self.list().into_iter().next() {
            return s;
        }
        let terminal = self
            .config
            .lock()
            .expect("config poisoned")
            .profile(DEFAULT_PROFILE)
            .map(|p| p.terminal.clone())
            .unwrap_or_default();
        self.create(DEFAULT_PROFILE, terminal)
    }

    /// Close and remove a session.
    pub async fn remove(&self, id: SessionId) -> bool {
        let Some(s) = self.sessions.lock().expect("sessions poisoned").remove(&id) else {
            return false;
        };
        let _ = s.close().await;
        let _ = self.events.send(ManagerEvent::Removed(id));
        true
    }

    /// Run `f` with the configuration.
    pub fn with_config<R>(&self, f: impl FnOnce(&Config) -> R) -> R {
        f(&self.config.lock().expect("config poisoned"))
    }

    /// Run `f` with mutable configuration (call [`SessionManager::save_config`] afterwards).
    pub fn with_config_mut<R>(&self, f: impl FnOnce(&mut Config) -> R) -> R {
        f(&mut self.config.lock().expect("config poisoned"))
    }

    /// Persist the configuration, if a path is set.
    pub fn save_config(&self) -> Result<(), ConfigError> {
        match &self.config_path {
            Some(path) => self.config.lock().expect("config poisoned").save(path),
            None => Ok(()),
        }
    }
}
