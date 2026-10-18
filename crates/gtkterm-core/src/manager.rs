//! Owner of all sessions, shared by the GUI and the MCP server.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::{Config, ConfigError, DEFAULT_PROFILE, TerminalConfig};
use crate::macros::{MacroError, MacroReport, MacroStore, run_macro};
use crate::script::{Location, Script, ScriptError, ScriptEvent, ScriptOptions, ScriptReport, run_script};
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
    /// The configuration (profiles) was saved.
    ConfigSaved,
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
    /// A script started in a session.
    ScriptStarted {
        /// Session.
        session: SessionId,
        /// Script name.
        name: String,
    },
    /// A script reached a line (throttled to 20 per second).
    ScriptLine {
        /// Session.
        session: SessionId,
        /// Where.
        at: Location,
    },
    /// A script's `log` output.
    ScriptLog {
        /// Session.
        session: SessionId,
        /// Message.
        message: String,
    },
    /// A script finished.
    ScriptFinished {
        /// Session.
        session: SessionId,
        /// Script name.
        name: String,
        /// `None` on success, otherwise the error message.
        error: Option<String>,
    },
}

/// Lock, recovering from poisoning (a panic elsewhere must not take the manager down).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The per-session "one macro or script at a time" slot. Released when the run ends, also
/// when its future is dropped (an MCP call that timed out or was cancelled).
struct RunSlot<'a> {
    manager: &'a SessionManager,
    session: SessionId,
    name: String,
    script: bool,
    token: CancellationToken,
    released: bool,
}

impl RunSlot<'_> {
    fn finish(mut self, error: Option<String>) {
        self.release(error);
    }

    fn release(&mut self, error: Option<String>) {
        if std::mem::replace(&mut self.released, true) {
            return;
        }
        lock(&self.manager.running).remove(&self.session);
        let (session, name) = (self.session, self.name.clone());
        let ev = if self.script {
            ManagerEvent::ScriptFinished { session, name, error }
        } else {
            ManagerEvent::MacroFinished { session, name, error }
        };
        let _ = self.manager.events.send(ev);
    }
}

impl Drop for RunSlot<'_> {
    fn drop(&mut self) {
        if !self.released {
            self.token.cancel();
            self.release(Some("cancelled".into()));
        }
    }
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
            events: broadcast::channel(1024).0,
            config: Mutex::new(config),
            config_path,
            macros: Mutex::default(),
            macros_path: Mutex::default(),
            running: Mutex::default(),
        })
    }

    /// Use `store`, persisted at `path` (`None`: memory only).
    pub fn set_macros(&self, store: MacroStore, path: Option<PathBuf>) {
        *lock(&self.macros) = store;
        *lock(&self.macros_path) = path;
        let _ = self.events.send(ManagerEvent::MacrosChanged);
    }

    /// Run `f` with the macro store.
    pub fn with_macros<R>(&self, f: impl FnOnce(&MacroStore) -> R) -> R {
        f(&lock(&self.macros))
    }

    /// Change the macro store and persist it; notifies subscribers.
    pub fn update_macros<R>(
        &self,
        f: impl FnOnce(&mut MacroStore) -> Result<R, MacroError>,
    ) -> Result<R, MacroError> {
        // Saved under the lock: concurrent changes (GUI and MCP) cannot write an older
        // snapshot last, and a failed save leaves memory unchanged.
        let result = {
            let mut store = lock(&self.macros);
            let mut changed = store.clone();
            let r = f(&mut changed)?;
            if let Some(path) = lock(&self.macros_path).clone() {
                changed.save(&path)?;
            }
            *store = changed;
            r
        };
        let _ = self.events.send(ManagerEvent::MacrosChanged);
        Ok(result)
    }

    /// Name of the macro running in `session`, if any.
    pub fn running_macro(&self, session: SessionId) -> Option<String> {
        lock(&self.running).get(&session).map(|(n, _)| n.clone())
    }

    /// Cancel the macro or script running in `session`. Returns false if none runs.
    pub fn stop_macro(&self, session: SessionId) -> bool {
        lock(&self.running)
            .get(&session)
            .map(|(_, t)| t.cancel())
            .is_some()
    }

    /// Take the session's run slot for `name`, or return the name of what already runs.
    fn acquire(&self, session: SessionId, name: &str, script: bool) -> Result<RunSlot<'_>, String> {
        let token = CancellationToken::new();
        {
            let mut running = lock(&self.running);
            if let Some((other, _)) = running.get(&session) {
                return Err(other.clone());
            }
            running.insert(session, (name.to_owned(), token.clone()));
        }
        let name = name.to_owned();
        let ev = if script {
            ManagerEvent::ScriptStarted {
                session,
                name: name.clone(),
            }
        } else {
            ManagerEvent::MacroStarted {
                session,
                name: name.clone(),
            }
        };
        let _ = self.events.send(ev);
        Ok(RunSlot {
            manager: self,
            session,
            name,
            script,
            token,
            released: false,
        })
    }

    /// Run the macro `name` in `session` (one at a time per session) and wait for the result.
    pub async fn run_macro(&self, session: &Session, name: &str) -> Result<MacroReport, MacroError> {
        let m = self
            .with_macros(|s| s.get(name).cloned())
            .ok_or_else(|| MacroError::NotFound(name.to_owned()))?;
        let slot = self
            .acquire(session.id(), &m.name, false)
            .map_err(MacroError::Busy)?;
        let result = run_macro(session, &m, &slot.token).await;
        slot.finish(result.as_ref().err().map(ToString::to_string));
        result
    }

    /// Run a script in `session` (one macro or script at a time per session) and wait for it.
    pub async fn run_script(
        &self,
        session: &Session,
        script: &Script,
        opts: &ScriptOptions,
    ) -> Result<ScriptReport, ScriptError> {
        let id = session.id();
        let slot = self.acquire(id, script.name(), true).map_err(ScriptError::Busy)?;
        let token = slot.token.clone();
        let events = self.events.clone();
        let mut last_line: Option<std::time::Instant> = None;
        let result = run_script(session, script, opts, &token, |ev| match ev {
            ScriptEvent::Line(at) => {
                if last_line.is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(50)) {
                    last_line = Some(std::time::Instant::now());
                    let _ = events.send(ManagerEvent::ScriptLine { session: id, at });
                }
            }
            ScriptEvent::Log(message) => {
                let _ = events.send(ManagerEvent::ScriptLog { session: id, message });
            }
        })
        .await;
        slot.finish(result.as_ref().err().map(ToString::to_string));
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
        lock(&self.sessions).insert(id, session.clone());
        let _ = self.events.send(ManagerEvent::Added(id));
        session
    }

    /// Create a session for a profile from the configuration.
    pub fn create_for_profile(
        &self,
        profile: &str,
    ) -> Result<(Session, crate::config::Connection), ConfigError> {
        let p = lock(&self.config).profile(profile)?.clone();
        let session = self.create(profile, p.terminal);
        session.apply_decoder_config(p.decoder);
        Ok((session, p.connection))
    }

    /// Session by ID.
    pub fn get(&self, id: SessionId) -> Option<Session> {
        lock(&self.sessions).get(&id).cloned()
    }

    /// All sessions in creation order.
    pub fn list(&self) -> Vec<Session> {
        lock(&self.sessions).values().cloned().collect()
    }

    /// The session with the lowest ID, or a new one for the default profile.
    pub fn first_or_create(&self) -> Session {
        if let Some(s) = self.list().into_iter().next() {
            return s;
        }
        let terminal = lock(&self.config)
            .profile(DEFAULT_PROFILE)
            .map(|p| p.terminal.clone())
            .unwrap_or_default();
        self.create(DEFAULT_PROFILE, terminal)
    }

    /// Close and remove a session.
    pub async fn remove(&self, id: SessionId) -> bool {
        let Some(s) = lock(&self.sessions).remove(&id) else {
            return false;
        };
        let _ = s.close().await;
        let _ = self.events.send(ManagerEvent::Removed(id));
        true
    }

    /// Run `f` with the configuration.
    pub fn with_config<R>(&self, f: impl FnOnce(&Config) -> R) -> R {
        f(&lock(&self.config))
    }

    /// Run `f` with mutable configuration (call [`SessionManager::save_config`] afterwards).
    pub fn with_config_mut<R>(&self, f: impl FnOnce(&mut Config) -> R) -> R {
        f(&mut lock(&self.config))
    }

    /// Persist the configuration, if a path is set.
    pub fn save_config(&self) -> Result<(), ConfigError> {
        if let Some(path) = &self.config_path {
            lock(&self.config).save(path)?;
        }
        let _ = self.events.send(ManagerEvent::ConfigSaved);
        Ok(())
    }
}
