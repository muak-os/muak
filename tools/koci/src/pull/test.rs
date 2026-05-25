use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use tar::{Builder, Header};

use crate::image::{ImageReference, OciDescriptor};

pub(crate) type RouteKey = (String, String);

#[derive(Clone)]
pub(crate) struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    delay: Duration,
}

impl HttpResponse {
    pub(crate) fn manifest(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/vnd.oci.image.manifest.v1+json",
            body,
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn blob(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body,
            delay: Duration::ZERO,
        }
    }

    fn not_found() -> Self {
        Self {
            status: 404,
            content_type: "text/plain",
            body: b"missing".to_vec(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

pub(crate) struct TestRegistry {
    pub(crate) address: String,
    connections: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestRegistry {
    pub(crate) fn start(routes: HashMap<RouteKey, HttpResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test registry");
        listener
            .set_nonblocking(true)
            .expect("set test registry nonblocking");

        let address = listener
            .local_addr()
            .expect("get test registry address")
            .to_string();
        let routes = Arc::new(routes);
        let connections = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_routes = Arc::clone(&routes);
        let thread_connections = Arc::clone(&connections);
        let thread_shutdown = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            loop {
                if thread_shutdown.load(Ordering::SeqCst) {
                    break;
                }

                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };

                let thread_routes = Arc::clone(&thread_routes);
                let connection_handle = thread::spawn(move || {
                    let _ = handle_connection(stream, &thread_routes);
                });
                thread_connections
                    .lock()
                    .expect("lock test registry connection handles")
                    .push(connection_handle);
            }
        });

        Self {
            address,
            connections,
            shutdown,
            handle: Some(handle),
        }
    }

    pub(crate) fn reference(&self, repository: &str, tag: &str) -> String {
        format!("{}/{repository}:{tag}", self.address)
    }

    pub(crate) fn image_reference(&self) -> ImageReference {
        ImageReference {
            registry: self.address.clone(),
            name: "repo".to_string(),
            manifest_ref: "test".to_string(),
        }
    }
}

impl Drop for TestRegistry {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join test registry thread");
        }

        let connection_handles = std::mem::take(
            &mut *self
                .connections
                .lock()
                .expect("lock test registry connection handles"),
        );
        for handle in connection_handles {
            handle.join().expect("join test connection thread");
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    routes: &HashMap<RouteKey, HttpResponse>,
) -> std::result::Result<(), IoError> {
    let request = read_request(&mut stream)?;
    let response = routes
        .get(&request)
        .cloned()
        .unwrap_or_else(HttpResponse::not_found);
    write_response(&mut stream, &response)
}

fn read_request(stream: &mut TcpStream) -> std::result::Result<RouteKey, IoError> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(IoError::new(
                ErrorKind::UnexpectedEof,
                "connection closed before headers completed",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);

        if let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let header_text = std::str::from_utf8(&buffer[..header_end])
                .map_err(|error| IoError::new(ErrorKind::InvalidData, error))?;
            let mut parts = header_text
                .lines()
                .next()
                .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request line"))?
                .split_whitespace();
            let method = parts
                .next()
                .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request method"))?
                .to_string();
            let path = parts
                .next()
                .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request path"))?
                .to_string();
            return Ok((method, path));
        }
    }
}

fn write_response(
    stream: &mut TcpStream,
    response: &HttpResponse,
) -> std::result::Result<(), IoError> {
    if !response.delay.is_zero() {
        thread::sleep(response.delay);
    }

    let reason = match response.status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };

    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.body.len(),
        response.content_type,
    );

    stream.write_all(headers.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()?;
    Ok(())
}

pub(crate) fn descriptor(digest: &str, media_type: Option<&str>) -> OciDescriptor {
    OciDescriptor {
        media_type: media_type.map(str::to_string),
        digest: digest.to_string(),
        platform: None,
    }
}

pub(crate) fn layer_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = Builder::new(encoder);

    for (path, bytes) in entries {
        let mut header = Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, path, *bytes)
            .expect("append layer data");
    }

    archive
        .into_inner()
        .expect("finish layer archive")
        .finish()
        .expect("finish gzip archive")
}

pub(crate) fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", crate::digest::sha256_hex(bytes))
}

pub(crate) fn manifest_json(layer_digest: &str, layer_size: usize, media_type: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "size": 1,
        },
        "layers": [{
            "mediaType": media_type,
            "digest": layer_digest,
            "size": layer_size,
        }],
    }))
    .expect("serialize manifest json")
}
