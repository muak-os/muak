use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

type RouteKey = (String, String);

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct MockRegistry {
    address: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockRegistry {
    pub fn start(routes: HashMap<RouteKey, HttpResponse>) -> Result<Self, IoError> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;

        let address = listener.local_addr()?.to_string();
        let routes = Arc::new(routes);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let thread_routes = Arc::clone(&routes);
        let thread_requests = Arc::clone(&requests);
        let thread_shutdown = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            run_registry_server(listener, thread_routes, thread_requests, thread_shutdown)
        });

        Ok(Self {
            address,
            requests,
            shutdown,
            handle: Some(handle),
        })
    }

    pub fn reference(&self, repository: &str, tag: &str) -> String {
        format!("{}/{repository}:{tag}", self.address)
    }

    pub fn request(&self, method: &str, path: &str) -> Result<Option<RecordedRequest>, IoError> {
        let method = method.to_ascii_uppercase();
        let requests = self
            .requests
            .lock()
            .map_err(|_| IoError::other("request log mutex poisoned"))?;
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
            let _ = handle.join();
        }
    }
}

#[derive(Clone)]
pub struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/vnd.oci.image.manifest.v1+json",
            body,
        }
    }

    pub fn index(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/vnd.oci.image.index.v1+json",
            body,
        }
    }

    pub fn octet_stream(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body,
        }
    }

    pub fn ok() -> Self {
        Self {
            status: 200,
            content_type: "text/plain",
            body: Vec::new(),
        }
    }
}

pub fn get(path: impl Into<String>, response: HttpResponse) -> (RouteKey, HttpResponse) {
    (("GET".to_string(), path.into()), response)
}

pub fn put(path: impl Into<String>, response: HttpResponse) -> (RouteKey, HttpResponse) {
    (("PUT".to_string(), path.into()), response)
}

fn run_registry_server(
    listener: TcpListener,
    routes: Arc<HashMap<RouteKey, HttpResponse>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let stream = match accept_connection(&listener) {
            Ok(Some(stream)) => stream,
            Ok(None) => continue,
            Err(_) => break,
        };

        let _ = handle_connection(stream, &routes, &requests);
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
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
) -> Result<(), IoError> {
    let request = read_request(&mut stream)?;
    let response = routes
        .get(&(request.method.clone(), request.path.clone()))
        .cloned()
        .unwrap_or_else(not_found_response);

    requests
        .lock()
        .map_err(|_| IoError::other("request log mutex poisoned"))?
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
        buffer.extend_from_slice(&chunk[..read]);

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

    let header_bytes = &buffer[..header_end];
    let header_text = std::str::from_utf8(header_bytes)
        .map_err(|error| IoError::new(ErrorKind::InvalidData, error))?;

    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request method"))?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "missing request path"))?
        .to_string();

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
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| IoError::new(ErrorKind::InvalidData, error))?
        .unwrap_or(0);

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(IoError::new(
                ErrorKind::UnexpectedEof,
                "connection closed before request body completed",
            ));
        }
        body.extend_from_slice(&chunk[..read]);
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
    }
}
