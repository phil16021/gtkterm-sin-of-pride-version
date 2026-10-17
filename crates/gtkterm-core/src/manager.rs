//! Owner of all sessions, shared by the GUI and the MCP server.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::{Config, ConfigError, DEFAULT_PROFILE, TerminalConfig};
use crate::macros::{MacroError, MacroReport, MacroStore, run_macro};
use crate::session::{Session, SessionId};

/// Session list changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerEvent {
    /// A session was created.
    Added(SessionId),
    /// A session was removed.
    Removed(SessionId),
    /// The macro store changed.
    MacrosChanged,
    /// A macro started in a session.
    MacroStarted {
        /// Session.
        session: SessionId,
        /// Macro name.
        name: String,
    },
    /// A macro finished.
    MacroFinished {
        /// Session.
        session: SessionId,
        /// Macro name.
        name: String,
        /// `None` on success, otherwise the error message.
        error: Option<String>,
    },
}

/// Registry of sessions plus the shared configuration.
#[derive(Debug)]
pub struct SessionManager {
    sessions: Mutex<BTreeMap<SessionId, Session>>,
    next_id: AtomicU32,
    events: broadcast::Sender<ManagerEvent>,
    config: Mutex<Config>,
    config_path: Option<PathBuf>,
    macros: Mutex<MacroStore>,
    macros_path: Mutex<Option<PathBuf>>,
    running: Mutex<HashMap<SessionId, (String, CancellationToken)>>,
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
            macros: Mutex::default(),
            macros_path: Mutex::default(),
            running: Mutex::default(),
        })
    }

    /// Use `store`, persisted at `path` (`None`: memory only).
    pub fn set_macros(&self, store: MacroStore, path: Option<PathBuf>) {
        *self.macros.lock().expect("macros poisoned") = store;
        *self.macros_path.lock().expect("macros poisoned") = path;
        let _ = self.events.send(ManagerEvent::MacrosChanged);
    }

    /// Run `f` with the macro store.
    pub fn with_macros<R>(&self, f: impl FnOnce(&MacroStore) -> R) -> R {
        f(&self.macros.lock().expect("macros poisoned"))
    }

    /// Change the macro store and persist it; notifies subscribers.
    pub fn update_macros<R>(
        &self,
        f: impl FnOnce(&mut MacroStore) -> Result<R, MacroError>,
    ) -> Result<R, MacroError> {
        let (result, snapshot) = {
            let mut store = self.macros.lock().expect("macros poisoned");
            let r = f(&mut store)?;
            (r, store.clone())
        };
        if let Some(path) = self.macros_path.lock().expect("macros poisoned").clone() {
            snapshot.save(&path)?;
        }
        let _ = self.events.send(ManagerEvent::MacrosChanged);
        Ok(result)
    }

    /// Name of the macro running in `session`, if any.
    pub fn running_macro(&self, session: SessionId) -> Option<String> {
        self.running
            .lock()
            .expect("running poisoned")
            .get(&session)
            .map(|(n, _)| n.clone())
    }

    /// Cancel the macro running in `session`. Returns false if none runs.
    pub fn stop_macro(&self, session: SessionId) -> bool {
        self.running
            .lock()
            .expect("running poisoned")
            .get(&session)
            .map(|(_, t)| t.cancel())
            .is_some()
    }

    /// Run the macro `name` in `session` (one at a time per session) and wait for the result.
    pub async fn run_macro(&self, session: &Session, name: &str) -> Result<MacroReport, MacroError> {
        let m = self
            .with_macros(|s| s.get(name).cloned())
            .ok_or_else(|| MacroError::NotFound(name.to_owned()))?;
        let id = session.id();
        let token = CancellationToken::new();
        {
            let mut running = self.running.lock().expect("running poisoned");
            if let Some((other, _)) = running.get(&id) {
                return Err(MacroError::Busy(other.clone()));
            }
            running.insert(id, (m.name.clone(), token.clone()));
        }
        let _ = self.events.send(ManagerEvent::MacroStarted {
            session: id,
            name: m.name.clone(),
        });
        let result = run_macro(session, &m, &token).await;
        self.running.lock().expect("running poisoned").remove(&id);
        let _ = self.events.send(ManagerEvent::MacroFinished {
            session: id,
            name: m.name.clone(),
            error: result.as_ref().err().map(ToString::to_string),
        });
        result
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
        let session = self.create(profile, p.terminal);
        session.apply_decoder_config(p.decoder);
        Ok((session, p.connection))
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
