mod config;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use notify::NotifyClient;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::{TcpListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    kmsg::init("apid")?;
    kmsg::info!("API daemon starting");

    let args: Vec<String> = std::env::args().collect();
    let listen_addr = args
        .iter()
        .position(|a| a == "--listen")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or(config::DEFAULT_LISTEN_ADDR);

    let notifier = NotifyClient::new("apid")?;

    let addr: SocketAddr = listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;

    kmsg::info!("API daemon ready, listening on {}", addr);
    notifier.ready(&format!("tcp://{}", listen_addr))?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).ok();
        let mut sigint = signal(SignalKind::interrupt()).ok();

        tokio::select! {
            _ = async { sigterm.as_mut()?.recv().await }, if sigterm.is_some() => {
                kmsg::info!("Received SIGTERM, shutting down");
            }
            _ = async { sigint.as_mut()?.recv().await }, if sigint.is_some() => {
                kmsg::info!("Received SIGINT, shutting down");
            }
        }
        shutdown_clone.store(true, Ordering::SeqCst);
    });

    while !shutdown.load(Ordering::SeqCst) {
        let accept_future = listener.accept();
        let timeout_result =
            tokio::time::timeout(std::time::Duration::from_millis(100), accept_future).await;

        let (stream, peer_addr) = match timeout_result {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                kmsg::warn!("Accept error: {}", e);
                continue;
            }
            Err(_) => continue,
        };

        let io = TokioIo::new(stream);
        tokio::spawn(serve_connection(io, peer_addr));
    }

    notifier.stopping("Graceful shutdown")?;
    kmsg::info!("API daemon stopped");

    Ok(())
}

async fn handle_request(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let path = req.uri().path();

    let socket_path = if path.starts_with(config::VM_SERVICE_PREFIX) {
        config::VMD_SOCKET
    } else if path.starts_with(config::PROCESS_SERVICE_PREFIX)
        || path.starts_with(config::PROVISION_SERVICE_PREFIX)
    {
        config::GRANOLA_SOCKET
    } else {
        kmsg::warn!("Unknown service path: {}", path);
        return Ok(Response::builder()
            .status(404)
            .header("content-type", "application/grpc")
            .header("grpc-status", "12") // UNIMPLEMENTED
            .header("grpc-message", "Unknown service")
            .body(Full::new(Bytes::new()))
            .expect("building response should not fail"));
    };

    match proxy_to_backend(req, socket_path).await {
        Ok(response) => Ok(response),
        Err(e) => {
            kmsg::error!("Proxy error to {}: {}", socket_path, e);
            Ok(Response::builder()
                .status(503)
                .header("content-type", "application/grpc")
                .header("grpc-status", "14") // UNAVAILABLE
                .header("grpc-message", format!("Backend unavailable: {}", e))
                .body(Full::new(Bytes::new()))
                .expect("building response should not fail"))
        }
    }
}

async fn proxy_to_backend(
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

fn is_benign_error(e: &hyper::Error) -> bool {
    if e.is_incomplete_message() || e.is_canceled() {
        return true;
    }

    let msg = e.to_string().to_lowercase();
    msg.contains("connection reset")
        || msg.contains("broken pipe")
        || msg.contains("connection refused")
}

async fn serve_connection(io: TokioIo<tokio::net::TcpStream>, peer_addr: SocketAddr) {
    let service = service_fn(handle_request);
    let conn = http2::Builder::new(TokioExecutor::new()).serve_connection(io, service);

    if let Err(e) = conn.await
        && !is_benign_error(&e)
    {
        kmsg::warn!("Connection error from {}: {}", peer_addr, e);
    }
}
