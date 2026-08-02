//! Mock backend server fixture.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::path::{Path, PathBuf};

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tempfile::TempDir;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

/// Mock backend server that listens on a UNIX socket.
pub struct MockBackend {
    socket_path: PathBuf,
    _temp_dir: TempDir,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockBackend {
    /// Creates a new mock backend with the given response handler.
    pub async fn start<F>(handler: F) -> anyhow::Result<Self>
    where
        F: Fn(Request<Incoming>) -> Response<Full<Bytes>> + Send + Sync + 'static,
    {
        let temp_dir = TempDir::new()?;
        let socket_path = temp_dir.path().join("backend.sock");

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);
        let socket_path_clone = socket_path.clone();

        let listener = UnixListener::bind(&socket_path)?;
        let handler = Arc::new(handler);

        let handle = tokio::spawn(accept_loop(listener, shutdown_clone, handler));

        // Give the server time to start.
        sleep(Duration::from_millis(10)).await;

        Ok(Self {
            socket_path: socket_path_clone,
            _temp_dir: temp_dir,
            shutdown,
            handle: Some(handle),
        })
    }

    /// Returns the UNIX socket path the backend listens on.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Creates a mock backend that always returns a gRPC success response.
    pub async fn success() -> anyhow::Result<Self> {
        Self::start(|_req| grpc_ok_response()).await
    }

    /// Shuts down the mock backend.
    pub async fn shutdown(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _join_result = handle.await;
        }
    }
}

impl Drop for MockBackend {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

/// Accepts connections until shutdown is requested, serving each in a task.
async fn accept_loop<F>(listener: UnixListener, shutdown: Arc<AtomicBool>, handler: Arc<F>)
where
    F: Fn(Request<Incoming>) -> Response<Full<Bytes>> + Send + Sync + 'static,
{
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let accept_result = timeout(Duration::from_millis(100), listener.accept()).await;
        let Ok(Ok((stream, _))) = accept_result else {
            continue;
        };

        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            let _serve = serve_connection(stream, handler).await;
        });
    }
}

/// Serves one HTTP/2 connection using the given handler.
async fn serve_connection<F>(stream: UnixStream, handler: Arc<F>)
where
    F: Fn(Request<Incoming>) -> Response<Full<Bytes>> + Send + Sync + 'static,
{
    let io = TokioIo::new(stream);
    let service = service_fn(move |req| {
        let handler = Arc::clone(&handler);
        async move {
            let response = handler(req);
            Ok::<_, core::convert::Infallible>(response)
        }
    });

    let _serve = http2::Builder::new(TokioExecutor::new())
        .serve_connection(io, service)
        .await;
}

/// Builds a gRPC success response.
fn grpc_ok_response() -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("content-type", "application/grpc")
        .header("grpc-status", "0")
        .header("grpc-message", "OK")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap_or(Response::new(http_body_util::Full::new(Bytes::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_backend_creates_socket() {
        let backend = MockBackend::success()
            .await
            .expect("Failed to create mock backend");
        assert!(backend.socket_path().exists());
        backend.shutdown().await;
    }

    #[tokio::test]
    async fn mock_backend_accepts_connections() {
        let backend = MockBackend::success()
            .await
            .expect("Failed to create mock backend");

        let stream = UnixStream::connect(backend.socket_path()).await;
        assert!(stream.is_ok(), "Should be able to connect to mock backend");

        backend.shutdown().await;
    }
}
