//! Bridge between the GTK main loop and the tokio I/O runtime.

use std::future::Future;
use std::sync::OnceLock;

use gtkterm_core::session::{Session, SessionEvent};
use tokio::sync::broadcast::error::RecvError;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The I/O runtime (created on first use).
pub fn rt() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("gtkterm-io")
            .build()
            .expect("cannot start the I/O runtime")
    })
}

/// Run `fut` on the I/O runtime and hand its result to `done` on the GTK main thread.
pub fn spawn<F, T>(fut: F, done: impl FnOnce(T) + 'static)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = rt().spawn(fut);
    glib::spawn_future_local(async move {
        if let Ok(v) = handle.await {
            done(v);
        }
    });
}

/// Fire and forget on the I/O runtime.
pub fn spawn_io<F>(fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    rt().spawn(fut);
}

/// Events queued for the GTK thread. When it falls behind, the session's broadcast drops
/// the oldest events instead of this queue growing without bound.
const GUI_QUEUE: usize = 1024;

/// Forward session events to the GTK main thread. If the window cannot keep up, skipped
/// events are reported as an `Info` notice (the session history, logs and MCP keep the data).
pub fn events(session: &Session) -> async_channel::Receiver<SessionEvent> {
    let (tx, rx) = async_channel::bounded(GUI_QUEUE);
    let mut sub = session.subscribe();
    rt().spawn(async move {
        loop {
            let ev = match sub.recv().await {
                Ok(ev) => ev,
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(n, "GUI lagged behind session events");
                    SessionEvent::Info(super::i18n::gettext_f(
                        "{n} updates skipped: the window could not keep up",
                        &[("n", &n)],
                    ))
                }
                Err(RecvError::Closed) => break,
            };
            if tx.send(ev).await.is_err() {
                break;
            }
        }
    });
    rx
}

/// Forward a tokio mpsc receiver to the GTK main thread.
pub fn forward<T: Send + 'static>(mut rx: tokio::sync::mpsc::Receiver<T>) -> async_channel::Receiver<T> {
    let (tx, out) = async_channel::unbounded();
    rt().spawn(async move {
        while let Some(v) = rx.recv().await {
            if tx.send(v).await.is_err() {
                break;
            }
        }
    });
    out
}

/// Forward a broadcast receiver to the GTK main thread (lagged messages are skipped).
pub fn forward_broadcast<T: Clone + Send + 'static>(
    mut rx: tokio::sync::broadcast::Receiver<T>,
) -> async_channel::Receiver<T> {
    let (tx, out) = async_channel::unbounded();
    rt().spawn(async move {
        loop {
            match rx.recv().await {
                Ok(v) => {
                    if tx.send(v).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
    out
}
