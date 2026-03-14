//! Mock backend server fixture

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioExecutor;
use tempfile::TempDir;
use tokio::net::UnixListener;

/// Mock backend server that listens on a UNIX socket.
pub struct MockBackend {
    pub socket_path: PathBuf,
    _temp_dir: TempDir,
    shutdown: Arc<AtomicBool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl MockBackend {
    /// Creates a new mock backend with the given response handler.
    pub async fn start<F>(handler: F) -> anyhow::Result<Self>
    where
        F: Fn(Request<Incoming>) -> Response<http_body_util::Full<Bytes>> + Send + Sync + 'static,
    {
        let temp_dir = TempDir::new()?;
        let socket_path = temp_dir.path().join("backend.sock");

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let socket_path_clone = socket_path.clone();

        let listener = UnixListener::bind(&socket_path)?;
        let handler = Arc::new(handler);

        let handle = tokio::spawn(async move {
            loop {
                if shutdown_clone.load(Ordering::SeqCst) {
                    break;
                }

                let accept_result =
                    tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                        .await;

                let stream = match accept_result {
                    Ok(Ok((stream, _))) => stream,
                    Ok(Err(_)) | Err(_) => continue,
                };

                let handler = handler.clone();
                tokio::spawn(async move {
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let service = service_fn(move |req| {
                        let handler = handler.clone();
                        async move {
                            let response = handler(req);
                            Ok::<_, std::convert::Infallible>(response)
                        }
                    });

                    let _ = http2::Builder::new(TokioExecutor::new())
                        .serve_connection(io, service)
                        .await;
                });
            }
        });

        // Give the server time to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        Ok(Self {
            socket_path: socket_path_clone,
            _temp_dir: temp_dir,
            shutdown,
            handle: Some(handle),
        })
    }

    /// Creates a mock backend that echoes requests back as responses.
    #[allow(dead_code)]
    pub async fn echo() -> anyhow::Result<Self> {
        Self::start(|_req| {
            Response::builder()
                .status(200)
                .header("content-type", "application/grpc")
                .header("grpc-status", "0")
                .body(http_body_util::Full::new(Bytes::new()))
                .unwrap()
        })
        .await
    }

    /// Creates a mock backend that always returns a gRPC success response.
    pub async fn success() -> anyhow::Result<Self> {
        Self::start(|_req| {
            Response::builder()
                .status(200)
                .header("content-type", "application/grpc")
                .header("grpc-status", "0")
                .header("grpc-message", "OK")
                .body(http_body_util::Full::new(Bytes::new()))
                .unwrap()
        })
        .await
    }

    /// Shuts down the mock backend.
    pub async fn shutdown(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for MockBackend {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_backend_creates_socket() {
        let backend = MockBackend::echo()
            .await
            .expect("Failed to create mock backend");
        assert!(backend.socket_path.exists());
        backend.shutdown().await;
    }

    #[tokio::test]
    async fn mock_backend_accepts_connections() {
        let backend = MockBackend::success()
            .await
            .expect("Failed to create mock backend");

        let stream = tokio::net::UnixStream::connect(&backend.socket_path).await;
        assert!(stream.is_ok(), "Should be able to connect to mock backend");

        backend.shutdown().await;
    }
}
