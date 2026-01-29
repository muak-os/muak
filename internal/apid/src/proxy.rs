//! HTTP/2 reverse proxy to backend Unix sockets

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::UnixStream;

/// Proxies a request to a backend service via Unix socket
pub async fn proxy_to_backend(
    req: Request<Incoming>,
    socket_path: &str,
) -> Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
    let stream = UnixStream::connect(socket_path).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), io).await?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            kmsg::warn!("Backend connection error: {}", e);
        }
    });

    let (parts, body) = req.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    let backend_req = Request::from_parts(parts, Full::new(body_bytes));

    let response = sender.send_request(backend_req).await?;

    let (parts, body) = response.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    Ok(Response::from_parts(parts, Full::new(body_bytes)))
}
