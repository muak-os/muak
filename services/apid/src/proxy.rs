//! HTTP/2 reverse proxy to backend UNIX sockets with connection pooling.

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use http_body_util::BodyExt as _;
use http_body_util::combinators::BoxBody;
use hyper::body::{Body, Bytes, Incoming};
use hyper::client::conn::http2;
use hyper::client::conn::http2::SendRequest;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::constants;

/// Boxed body type used by pooled connections.
type BoxedBody = BoxBody<Bytes, Box<dyn core::error::Error + Send + Sync>>;

/// Per-socket cached HTTP/2 connection handle.
type CachedSender = Mutex<Option<SendRequest<BoxedBody>>>;

/// Maintains persistent HTTP/2 connections to backend UNIX sockets.
pub struct BackendPool {
    senders: HashMap<String, CachedSender>,
}

impl Default for BackendPool {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendPool {
    /// Creates a new pool with an entry for each known backend socket.
    #[must_use]
    pub fn new() -> Self {
        let mut senders = HashMap::new();
        senders.insert(constants::WORKLOADD_SOCKET.to_owned(), Mutex::new(None));
        senders.insert(constants::GRANOLA_SOCKET.to_owned(), Mutex::new(None));
        senders.insert(constants::PROVISIOND_SOCKET.to_owned(), Mutex::new(None));
        Self { senders }
    }

    /// Creates a pool with a single entry for the given socket path.
    #[must_use]
    pub fn from_socket(path: &str) -> Self {
        let mut senders = HashMap::new();
        senders.insert(path.to_owned(), Mutex::new(None));
        Self { senders }
    }

    /// Acquires a ready `SendRequest` handle, reusing a cached connection or
    /// establishing a new one.
    async fn acquire(&self, socket_path: &str) -> Result<SendRequest<BoxedBody>> {
        let entry = self
            .senders
            .get(socket_path)
            .context("Unknown backend socket")?;

        let mut guard = entry.lock().await;

        if let Some(sender) = guard.as_mut()
            && sender.is_ready()
        {
            return Ok(sender.clone());
        }

        let sender = connect(socket_path).await?;
        *guard = Some(sender.clone());
        Ok(sender)
    }

    /// Clears the cached connection for a socket so the next request reconnects.
    async fn invalidate(&self, socket_path: &str) {
        if let Some(entry) = self.senders.get(socket_path) {
            let mut guard = entry.lock().await;
            *guard = None;
        }
    }
}

/// Opens a UNIX stream and performs the HTTP/2 client handshake.
async fn connect(socket_path: &str) -> Result<SendRequest<BoxedBody>> {
    let stream = UnixStream::connect(socket_path).await.context(format!(
        "Failed to connect to backend socket at {socket_path}"
    ))?;
    let io = TokioIo::new(stream);

    let (sender, conn) = http2::handshake(TokioExecutor::new(), io)
        .await
        .context("Failed to perform HTTP/2 handshake with backend service")?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("Backend connection error: {e}");
        }
    });

    Ok(sender)
}

/// Proxies a request to a backend service, reusing pooled connections.
///
/// # Errors
///
/// Returns an error if no cached connection is available and a new one cannot
/// be established, or if the request cannot be sent.
pub async fn forward<B>(
    pool: &BackendPool,
    req: Request<B>,
    socket_path: &str,
) -> Result<Response<Incoming>>
where
    B: Body<Data = Bytes> + Send + Sync + 'static,
    B::Error: Into<Box<dyn core::error::Error + Send + Sync>>,
{
    let mut sender = pool.acquire(socket_path).await?;

    let req = req.map(|body| {
        body.map_err(|e| -> Box<dyn core::error::Error + Send + Sync> { e.into() })
            .boxed()
    });

    match sender.send_request(req).await {
        Ok(response) => Ok(response),
        Err(e) => {
            pool.invalidate(socket_path).await;
            Err(e).context("Failed to send request to backend service")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_has_all_backends() {
        // ACT
        let pool = BackendPool::new();

        // ASSERT
        assert!(pool.senders.contains_key(constants::WORKLOADD_SOCKET));
        assert!(pool.senders.contains_key(constants::GRANOLA_SOCKET));
        assert!(pool.senders.contains_key(constants::PROVISIOND_SOCKET));
    }

    #[test]
    fn pool_from_socket() {
        // ACT
        let pool = BackendPool::from_socket("/tmp/test.sock");

        // ASSERT
        assert!(pool.senders.contains_key("/tmp/test.sock"));
        assert_eq!(pool.senders.len(), 1);
    }

    #[tokio::test]
    async fn acquire_unknown_socket_errors() {
        let pool = BackendPool::new();
        let result = pool.acquire("/nonexistent.sock").await;
        result.unwrap_err();
    }
}
