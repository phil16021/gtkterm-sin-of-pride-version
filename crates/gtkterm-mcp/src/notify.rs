//! Change notifications for MCP clients: session and manager events become
//! `notifications/resources/updated` (for subscribed URIs, at most [`PER_SECOND`] per URI) and
//! `notifications/resources/list_changed` (sessions appear or go).
//!
//! Two ways in: `subscriptions/listen` (MCP 2026-07-28) and the legacy
//! `resources/subscribe` (2025-11-25), which collects URIs per connection.

use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gtkterm_core::manager::{ManagerEvent, SessionManager};
use gtkterm_core::session::{Session, SessionEvent};
use gtkterm_core::sunshine::SunshineUpdate;
use rmcp::service::{Peer, RoleServer, SubscriptionSink};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::{CancellationToken, DropGuard};

/// Most notifications per URI and second.
const PER_SECOND: u32 = 10;
/// Resources that are not per session and change.
const GLOBAL: [&str; 3] = ["gtkterm://ports", "gtkterm://profiles", "gtkterm://macros"];
/// Per-session resources (`gtkterm://session/{id}/…`).
const PER_SESSION: [&str; 5] = ["status", "rx", "signals", "frames", "sunshine"];

/// One change.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    Updated(String),
    ListChanged,
}

/// Whether `uri` names a resource that sends updates.
pub(crate) fn watchable(uri: &str) -> bool {
    if GLOBAL.contains(&uri) {
        return true;
    }
    let Some((id, what)) = uri
        .strip_prefix("gtkterm://session/")
        .and_then(|r| r.split_once('/'))
    else {
        return false;
    };
    id.parse::<u32>().is_ok()
        && (PER_SESSION.contains(&what)
            || what
                .strip_prefix("sunshine/")
                .is_some_and(|p| !p.is_empty() && !p.contains('/')))
}

fn session_uri(id: u32, what: &str) -> String {
    format!("gtkterm://session/{id}/{what}")
}

/// Resources a session event changes.
fn session_changes(id: u32, ev: &SessionEvent) -> Vec<String> {
    let what: &[&str] = match ev {
        SessionEvent::State(_) | SessionEvent::Modem(_) | SessionEvent::Transfer(_) => &["status"],
        SessionEvent::Data(_) => &["rx", "status"],
        SessionEvent::Sunshine(SunshineUpdate::ChartAdded(chart) | SunshineUpdate::ChartUpdated(chart)) => {
            return vec![
                session_uri(id, "sunshine"),
                session_uri(id, &format!("sunshine/{chart}")),
            ];
        }
        SessionEvent::Sunshine(SunshineUpdate::ChartsChanged) => &["sunshine"],
        SessionEvent::Info(_) | SessionEvent::Sunshine(SunshineUpdate::Rejected { .. }) => &[],
        SessionEvent::Cleared => &["rx", "status", "sunshine", "frames"],
        SessionEvent::Frames(_) | SessionEvent::Decoder(_) => &["frames"],
        SessionEvent::Signals(_) => &["signals"],
    };
    what.iter().map(|w| session_uri(id, w)).collect()
}

/// Everything about session `id` (after lost events).
fn all_of(id: u32) -> Vec<String> {
    PER_SESSION.iter().map(|w| session_uri(id, w)).collect()
}

/// Forward the events of `session` until it goes away or `stop` fires.
fn forward_session(session: &Session, tx: mpsc::UnboundedSender<Change>, stop: CancellationToken) {
    let id = session.id();
    let mut events = session.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = tokio::select! {
                () = stop.cancelled() => return,
                ev = events.recv() => ev,
            };
            let uris = match ev {
                Ok(ev) => session_changes(id, &ev),
                Err(broadcast::error::RecvError::Lagged(_)) => all_of(id),
                Err(broadcast::error::RecvError::Closed) => return,
            };
            for uri in uris {
                if tx.send(Change::Updated(uri)).is_err() {
                    return;
                }
            }
        }
    });
}

/// Collect the changes of all sessions (present and future) into one channel.
fn changes(manager: &Arc<SessionManager>, stop: &CancellationToken) -> mpsc::UnboundedReceiver<Change> {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut events = manager.subscribe();
    for s in manager.list() {
        forward_session(&s, tx.clone(), stop.child_token());
    }
    let manager = Arc::clone(manager);
    let stop = stop.clone();
    tokio::spawn(async move {
        loop {
            let ev = tokio::select! {
                () = stop.cancelled() => return,
                ev = events.recv() => ev,
            };
            let changes = match ev {
                Ok(ManagerEvent::Added(id)) => {
                    if let Some(s) = manager.get(id) {
                        forward_session(&s, tx.clone(), stop.child_token());
                    }
                    vec![Change::ListChanged, Change::Updated(GLOBAL[0].into())]
                }
                Ok(ManagerEvent::Removed(_)) => vec![Change::ListChanged, Change::Updated(GLOBAL[0].into())],
                Ok(ManagerEvent::ConfigSaved) => {
                    GLOBAL[..2].iter().map(|u| Change::Updated((*u).into())).collect()
                }
                Ok(ManagerEvent::MacrosChanged) => vec![Change::Updated(GLOBAL[2].into())],
                Ok(_) => Vec::new(),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let mut all: Vec<_> = GLOBAL.iter().map(|u| Change::Updated((*u).into())).collect();
                    all.push(Change::ListChanged);
                    all
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            for c in changes {
                if tx.send(c).is_err() {
                    return;
                }
            }
        }
    });
    rx
}

/// Where notifications go.
enum Target {
    /// A `subscriptions/listen` stream (it filters by the accepted URIs itself).
    Listen(SubscriptionSink),
    /// A legacy client: the URIs it subscribed to.
    Legacy(Peer<RoleServer>, Arc<Mutex<HashSet<String>>>),
}

impl Target {
    fn wants(&self, uri: &str, accepted: &HashSet<String>) -> bool {
        match self {
            Self::Listen(_) => accepted.contains(uri),
            Self::Legacy(_, uris) => uris.lock().unwrap_or_else(PoisonError::into_inner).contains(uri),
        }
    }

    /// `Err` when the client is gone.
    async fn send(&self, change: &Change) -> Result<(), ()> {
        match (self, change) {
            (Self::Listen(sink), Change::Updated(uri)) => {
                sink.notify_resource_updated(uri.clone()).await.map_err(drop)
            }
            (Self::Listen(sink), Change::ListChanged) => {
                sink.notify_resource_list_changed().await.map_err(drop)
            }
            (Self::Legacy(peer, _), Change::Updated(uri)) => peer
                .notify_resource_updated(rmcp::model::ResourceUpdatedNotificationParam::new(uri.clone()))
                .await
                .map_err(drop),
            (Self::Legacy(peer, _), Change::ListChanged) => {
                peer.notify_resource_list_changed().await.map_err(drop)
            }
        }
    }
}

/// Send the changes `target` wants until `stop` fires or the client is gone. `accepted` are the
/// URIs of a listen stream; `list_changed` whether list changes are wanted.
async fn run(
    manager: Arc<SessionManager>,
    target: Target,
    accepted: HashSet<String>,
    list_changed: bool,
    stop: CancellationToken,
) {
    let mut rx = changes(&manager, &stop);
    let mut pending = BTreeSet::new();
    let mut list = false;
    let mut tick = tokio::time::interval(Duration::from_secs(1) / PER_SECOND);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = stop.cancelled() => break,
            c = rx.recv() => match c {
                None => break,
                Some(Change::ListChanged) => list |= list_changed,
                Some(Change::Updated(uri)) => {
                    if target.wants(&uri, &accepted) {
                        pending.insert(uri);
                    }
                }
            },
            _ = tick.tick(), if list || !pending.is_empty() => {
                let mut batch: Vec<Change> = std::mem::take(&mut pending).into_iter().map(Change::Updated).collect();
                if std::mem::take(&mut list) {
                    batch.insert(0, Change::ListChanged);
                }
                for c in &batch {
                    if target.send(c).await.is_err() {
                        stop.cancel();
                        break;
                    }
                }
            }
        }
    }
    stop.cancel();
}

/// Serve one `subscriptions/listen` stream until it is cancelled.
pub(crate) async fn listen(
    manager: Arc<SessionManager>,
    sink: SubscriptionSink,
    accepted: &rmcp::model::SubscriptionFilter,
    stop: CancellationToken,
) {
    let uris = accepted
        .resource_subscriptions
        .iter()
        .flatten()
        .cloned()
        .collect();
    let list = accepted.resources_list_changed == Some(true);
    run(manager, Target::Listen(sink), uris, list, stop).await;
}

/// Legacy `resources/subscribe` state of one connection. Cloning (the HTTP transport clones
/// the server for every new client) starts with no subscriptions.
#[derive(Debug, Default)]
pub(crate) struct LegacySubscriptions {
    uris: Arc<Mutex<HashSet<String>>>,
    /// Stops the notifier when the connection's server goes away.
    running: Mutex<Option<DropGuard>>,
}

impl Clone for LegacySubscriptions {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl LegacySubscriptions {
    /// Add `uri`, starting the notifier for `peer` on the first subscription.
    pub(crate) fn subscribe(&self, manager: &Arc<SessionManager>, peer: Peer<RoleServer>, uri: String) {
        self.uris
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(uri);
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.is_none() {
            let stop = CancellationToken::new();
            let target = Target::Legacy(peer, Arc::clone(&self.uris));
            tokio::spawn(run(
                Arc::clone(manager),
                target,
                HashSet::new(),
                true,
                stop.clone(),
            ));
            *running = Some(stop.drop_guard());
        }
    }

    /// Remove `uri`; the notifier stops with the last one.
    pub(crate) fn unsubscribe(&self, uri: &str) {
        let mut uris = self.uris.lock().unwrap_or_else(PoisonError::into_inner);
        uris.remove(uri);
        if uris.is_empty() {
            self.running.lock().unwrap_or_else(PoisonError::into_inner).take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchable_uris() {
        for ok in [
            "gtkterm://ports",
            "gtkterm://macros",
            "gtkterm://session/1/rx",
            "gtkterm://session/12/sunshine",
            "gtkterm://session/1/sunshine/P0",
        ] {
            assert!(watchable(ok), "{ok}");
        }
        for bad in [
            "gtkterm://docs/sunshine-protocol",
            "gtkterm://session/x/rx",
            "gtkterm://session/1/nope",
            "gtkterm://session/1/sunshine/",
            "https://example.com",
        ] {
            assert!(!watchable(bad), "{bad}");
        }
    }

    #[test]
    fn chart_updates_name_the_chart() {
        let got = session_changes(
            3,
            &SessionEvent::Sunshine(SunshineUpdate::ChartUpdated("imu".into())),
        );
        assert_eq!(
            got,
            ["gtkterm://session/3/sunshine", "gtkterm://session/3/sunshine/imu"]
        );
        assert!(session_changes(3, &SessionEvent::Info("x".into())).is_empty());
    }
}
