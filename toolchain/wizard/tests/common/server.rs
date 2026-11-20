//! Thread-based mock OCI registry for tests.

extern crate alloc;

use alloc::sync::Arc;
use core::mem;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::thread;

use crate::common::fixtures::{config_blob, config_digest, sha256_digest};

pub(crate) type RouteKey = (String, String);
pub(crate) type Routes = HashMap<RouteKey, HttpResponse>;

#[derive(Clone, Debug)]
pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    pub(crate) path: String,
}

pub(crate) struct MockRegistry {
    address: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    shutdown: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockRegistry {
    pub(crate) fn start(mut routes: Routes) -> Result<Self, IoError> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;

        let address = listener.local_addr()?.to_string();
        routes
            .entry(("GET".to_owned(), "/v2/".to_owned()))
            .or_insert_with(HttpResponse::ok);
        let routes = Arc::new(routes);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));

        let thread_requests = Arc::clone(&requests);
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_connections = Arc::clone(&connections);

        let handle = thread::spawn(move || {
            run_registry_server(
                &listener,
                &routes,
                &thread_requests,
                &thread_shutdown,
                &thread_connections,
            );
        });

        Ok(Self {
            address,
            requests,
            shutdown,
            connections,
            handle: Some(handle),
        })
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    pub(crate) fn request_count(&self, method: &str, path: &str) -> Result<usize, IoError> {
        Ok(self.requests_for(method, path)?.len())
    }

    fn requests_for(&self, method: &str, path: &str) -> Result<Vec<RecordedRequest>, IoError> {
        let method = method.to_ascii_uppercase();
        let requests = self
            .requests
            .lock()
            .map_err(|_error| IoError::other("request log mutex poisoned"))?;
        Ok(requests
            .iter()
            .filter(|request| request.method == method && request.path == path)
            .cloned()
            .collect())
    }
}

impl Drop for MockRegistry {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            drop(handle.join());
        }
        let connection_handles = mem::take(
            &mut *self
                .connections
                .lock()
                .expect("lock mock registry connection handles"),
        );
        for handle in connection_handles {
            drop(handle.join());
        }
    }
}

#[derive(Clone)]
pub(crate) struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    headers: Vec<(&'static str, String)>,
    delay: Duration,
}

impl HttpResponse {
    pub(crate) fn json(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/vnd.oci.image.manifest.v1+json",
            body,
            headers: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn octet_stream(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body,
            headers: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    #[must_use = "the test needs the response to serve"]
    pub(crate) fn ok() -> Self {
        Self {
            status: 200,
            content_type: "text/plain",
            body: Vec::new(),
            headers: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    #[must_use = "the test needs the response header to be served"]
    pub(crate) fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));

        self
    }
}

pub(crate) fn get<T: Into<String>>(path: T, response: HttpResponse) -> (RouteKey, HttpResponse) {
    (("GET".to_owned(), path.into()), response)
}

fn run_registry_server(
    listener: &TcpListener,
    routes: &Arc<Routes>,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
    shutdown: &AtomicBool,
    connections: &Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let stream = match accept_connection(listener) {
            Ok(Some(stream)) => stream,
            Ok(None) => continue,
            Err(_) => break,
        };

        let routes = Arc::clone(routes);
        let requests = Arc::clone(requests);
        let connection_handle = thread::spawn(move || {
            drop(handle_connection(stream, &routes, &requests));
        });
        connections
            .lock()
            .expect("lock mock registry connection handles")
            .push(connection_handle);
    }
}

fn accept_connection(listener: &TcpListener) -> Result<Option<TcpStream>, IoError> {
    match listener.accept() {
        Ok((stream, _)) => Ok(Some(stream)),
        Err(error) if error.kind() == ErrorKind::WouldBlock => {
            thread::sleep(Duration::from_millis(10));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn handle_connection(
    mut stream: TcpStream,
    routes: &Routes,
    requests: &Mutex<Vec<RecordedRequest>>,
) -> Result<(), IoError> {
    let request = read_request(&mut stream)?;
    let response = routes
        .get(&(request.method.clone(), request.path.clone()))
        .cloned()
        .or_else(|| head_manifest_response(routes, &request))
        .unwrap_or_else(|| default_response(&request.path));

    requests
        .lock()
        .map_err(|_error| IoError::other("request log mutex poisoned"))?
        .push(request);

    write_response(&mut stream, &response)
}

fn read_request(stream: &mut TcpStream) -> Result<RecordedRequest, IoError> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];

    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(IoError::new(
                ErrorKind::UnexpectedEof,
                "connection closed before headers completed",
            ));
        }
        let bytes = chunk
            .get(..read)
            .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid request read size"))?;
        buffer.extend_from_slice(bytes);

        if let Some(header_end) = find_header_end(&buffer) {
            break header_end;
        }

        if buffer.len() > 16 * 1024 {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                "request headers exceed 16 KiB",
            ));
        }
    };

    let header_bytes = buffer
        .get(..header_end)
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid request header size"))?;
    let header_text = core::str::from_utf8(header_bytes)
        .map_err(|error| IoError::new(ErrorKind::InvalidData, error))?;

    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request method"))?
        .to_owned();
    let path = request_parts
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request path"))?
        .to_owned();

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }

        let (name, value) = line.split_once(':').ok_or_else(|| {
            IoError::new(
                ErrorKind::InvalidData,
                format!("malformed request header: {line}"),
            )
        })?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| IoError::new(ErrorKind::InvalidData, error))?
        .unwrap_or(0);

    let body_start = header_end
        .checked_add(4)
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid request header size"))?;
    let mut body = buffer
        .get(body_start..)
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid request body offset"))?
        .to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(IoError::new(
                ErrorKind::UnexpectedEof,
                "connection closed before request body completed",
            ));
        }
        let bytes = chunk
            .get(..read)
            .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "invalid request read size"))?;
        body.extend_from_slice(bytes);
    }
    body.truncate(content_length);

    let _: (HashMap<String, String>, Vec<u8>) = (headers, body);
    Ok(RecordedRequest { method, path })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_response(stream: &mut TcpStream, response: &HttpResponse) -> Result<(), IoError> {
    if !response.delay.is_zero() {
        thread::sleep(response.delay);
    }

    let reason = match response.status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };

    let mut headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n",
        response.status,
        reason,
        response.body.len(),
        response.content_type,
    );
    for &(name, ref value) in &response.headers {
        headers.push_str(name);
        headers.push_str(": ");
        headers.push_str(value);
        headers.push_str("\r\n");
    }
    headers.push_str("\r\n");

    stream.write_all(headers.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

// Synthesize a manifest HEAD from its GET route, digest header included.
fn head_manifest_response(routes: &Routes, request: &RecordedRequest) -> Option<HttpResponse> {
    if request.method != "HEAD" || !request.path.contains("/manifests/") {
        return None;
    }
    let response = routes
        .get(&("GET".to_owned(), request.path.clone()))
        .cloned()?;

    Some(head_from(response))
}

fn head_from(response: HttpResponse) -> HttpResponse {
    if response.status != 200 {
        return HttpResponse {
            body: Vec::new(),
            ..response
        };
    }
    let digest = sha256_digest(&response.body);

    HttpResponse {
        body: Vec::new(),
        ..response.with_header("Docker-Content-Digest", digest)
    }
}

fn default_response(path: &str) -> HttpResponse {
    let digest = config_digest();
    if path.ends_with(&format!("/blobs/{digest}")) {
        return HttpResponse::octet_stream(config_blob());
    }

    not_found_response()
}

fn not_found_response() -> HttpResponse {
    HttpResponse {
        status: 404,
        content_type: "text/plain",
        body: b"not found".to_vec(),
        headers: Vec::new(),
        delay: Duration::ZERO,
    }
}
