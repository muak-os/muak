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

type RouteKey = (String, String);

#[derive(Clone, Debug)]
pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct MockRegistry {
    address: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    shutdown: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockRegistry {
    pub(crate) fn start(routes: HashMap<RouteKey, HttpResponse>) -> Result<Self, IoError> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;

        let address = listener.local_addr()?.to_string();
        let routes = Arc::new(routes);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));

        let thread_routes = Arc::clone(&routes);
        let thread_requests = Arc::clone(&requests);
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_connections = Arc::clone(&connections);

        let handle = thread::spawn(move || {
            run_registry_server(
                &listener,
                &thread_routes,
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

    #[must_use]
    pub(crate) fn reference(&self, repository: &str, tag: &str) -> String {
        format!("{}/{repository}:{tag}", self.address)
    }

    #[must_use]
    pub(crate) fn digest_reference(&self, repository: &str, digest: &str) -> String {
        format!("{}/{repository}@{digest}", self.address)
    }

    pub(crate) fn request(
        &self,
        method: &str,
        path: &str,
    ) -> Result<Option<RecordedRequest>, IoError> {
        let method = method.to_ascii_uppercase();
        let requests = self
            .requests
            .lock()
            .map_err(|_error| IoError::other("request log mutex poisoned"))?;
        Ok(requests
            .iter()
            .find(|request| request.method == method && request.path == path)
            .cloned())
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
    delay: Duration,
}

impl HttpResponse {
    #[must_use]
    pub(crate) fn json(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/vnd.oci.image.manifest.v1+json",
            body,
            delay: Duration::ZERO,
        }
    }

    #[must_use]
    pub(crate) fn index(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/vnd.oci.image.index.v1+json",
            body,
            delay: Duration::ZERO,
        }
    }

    #[must_use]
    pub(crate) fn octet_stream(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body,
            delay: Duration::ZERO,
        }
    }

    #[must_use]
    pub(crate) fn ok() -> Self {
        Self {
            status: 200,
            content_type: "text/plain",
            body: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    #[must_use]
    pub(crate) fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

pub(crate) fn get<T: Into<String>>(path: T, response: HttpResponse) -> (RouteKey, HttpResponse) {
    (("GET".to_owned(), path.into()), response)
}

pub(crate) fn put<T: Into<String>>(path: T, response: HttpResponse) -> (RouteKey, HttpResponse) {
    (("PUT".to_owned(), path.into()), response)
}

fn run_registry_server(
    listener: &TcpListener,
    routes: &Arc<HashMap<RouteKey, HttpResponse>>,
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
    routes: &HashMap<RouteKey, HttpResponse>,
    requests: &Mutex<Vec<RecordedRequest>>,
) -> Result<(), IoError> {
    let request = read_request(&mut stream)?;
    let response = routes
        .get(&(request.method.clone(), request.path.clone()))
        .cloned()
        .unwrap_or_else(not_found_response);

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

    Ok(RecordedRequest {
        method,
        path,
        headers,
        body,
    })
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

    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.body.len(),
        response.content_type,
    );

    stream.write_all(headers.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn not_found_response() -> HttpResponse {
    HttpResponse {
        status: 404,
        content_type: "text/plain",
        body: b"not found".to_vec(),
        delay: Duration::ZERO,
    }
}
