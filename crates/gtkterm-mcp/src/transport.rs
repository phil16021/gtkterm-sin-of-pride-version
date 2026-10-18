//! Transports: stdio for agents that spawn the process, Streamable HTTP for the running GUI.

use std::net::SocketAddr;

use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::GtkTermMcp;

/// Failure while serving.
#[derive(Debug, thiserror::Error)]
#[error("MCP server: {0}")]
pub struct ServeError(String);

/// Serve one client over stdin/stdout until it disconnects.
pub async fn serve_stdio(server: GtkTermMcp) -> Result<(), ServeError> {
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| ServeError(e.to_string()))?;
    running.waiting().await.map_err(|e| ServeError(e.to_string()))?;
    Ok(())
}

/// Running HTTP endpoint.
#[derive(Debug)]
pub struct HttpHandle {
    /// Bound address.
    pub addr: SocketAddr,
    /// URL to give to MCP clients.
    pub url: String,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl HttpHandle {
    /// Stop serving.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

/// Security settings for the HTTP endpoint.
#[derive(Debug, Clone)]
pub struct HttpSecurity {
    /// Required `Authorization: Bearer <token>`; `None` disables the check (tests only).
    pub token: Option<String>,
    /// Accepted `Origin` header values (requests without Origin are allowed: non-browser clients).
    pub allowed_origins: Vec<String>,
}

impl Default for HttpSecurity {
    fn default() -> Self {
        Self {
            token: None,
            allowed_origins: vec!["http://localhost".into(), "http://127.0.0.1".into()],
        }
    }
}

fn check(headers: &HeaderMap, sec: &HttpSecurity) -> Result<(), (StatusCode, &'static str)> {
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let ok = sec
            .allowed_origins
            .iter()
            .any(|o| origin == o || origin.starts_with(&format!("{o}:")));
        if !ok {
            return Err((StatusCode::FORBIDDEN, "origin not allowed"));
        }
    }
    if let Some(token) = &sec.token {
        let expected = format!("Bearer {token}");
        let given = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !constant_time_eq(given.as_bytes(), expected.as_bytes()) {
            return Err((StatusCode::UNAUTHORIZED, "missing or wrong bearer token"));
        }
    }
    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Serve Streamable HTTP on `addr` (use a loopback address) at path `/mcp`.
pub async fn serve_http(
    server: GtkTermMcp,
    addr: SocketAddr,
    security: HttpSecurity,
) -> std::io::Result<HttpHandle> {
    let cancel = CancellationToken::new();
    let config = StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token());
    let service: StreamableHttpService<GtkTermMcp, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        config,
    );
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(move |req: Request, next: Next| {
            let sec = security.clone();
            async move {
                match check(req.headers(), &sec) {
                    Ok(()) => next.run(req).await,
                    Err((code, msg)) => (code, msg).into_response(),
                }
            }
        }));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { token.cancelled().await })
            .await;
    });
    info!(%addr, "MCP HTTP endpoint listening");
    Ok(HttpHandle {
        addr,
        url: format!("http://{addr}/mcp"),
        cancel,
        task,
    })
}

/// Generate a random bearer token (128 bits, hex). Fails rather than falling back to a
/// guessable value when the system's random source cannot be read.
pub fn random_token() -> std::io::Result<String> {
    use std::fmt::Write as _;
    use std::io::Read as _;
    let mut buf = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().fold(String::with_capacity(32), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }))
}
