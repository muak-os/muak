//! HTTP/2 reverse proxy to backend UNIX sockets

use anyhow::{Context, Result};
use hyper::body::Incoming;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::UnixStream;

/// Proxies a request to a backend service via UNIX socket with streaming pass through
pub async fn proxy_to_backend<T>(req: Request<T>, socket_path: &str) -> Result<Response<Incoming>>
where
    T: hyper::body::Body + Send + Unpin + 'static,
    T::Data: Send,
    T::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let stream = UnixStream::connect(socket_path).await.context(format!(
        "Failed to connect to backend socket at {}",
        socket_path
    ))?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io)
        .await
        .context("Failed to perform HTTP/2 handshake with backend service")?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            kmsg::warn!("Backend connection error: {}", e);
        }
    });

    let response = sender
        .send_request(req)
        .await
        .context("Failed to send request to backend service")?;
    Ok(response)
}
