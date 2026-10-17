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

/// Forward session events to the GTK main thread. Lagged receivers resync with `Cleared`-like
/// semantics handled by the consumer (it simply continues).
pub fn events(session: &Session) -> async_channel::Receiver<SessionEvent> {
    let (tx, rx) = async_channel::unbounded();
    let mut sub = session.subscribe();
    rt().spawn(async move {
        loop {
            match sub.recv().await {
                Ok(ev) => {
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => tracing::warn!(n, "GUI lagged behind session events"),
                Err(RecvError::Closed) => break,
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
