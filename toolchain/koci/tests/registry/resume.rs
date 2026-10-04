//! A test registry that truncates transfers to exercise resumable fetches.

extern crate alloc;

use alloc::sync::Arc;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

/// A registry that truncates the first blob transfer and resumes later
/// requests with an HTTP Range response.
pub(crate) struct ResumeServer {
    reference_base: String,
    layer_len: Arc<AtomicUsize>,
    resumed_offset: Arc<AtomicUsize>,
    resumed_bytes: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ResumeServer {
    pub(crate) fn start(layer: Vec<u8>, manifest: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind resume server");
        listener.set_nonblocking(true).expect("set nonblocking");
        let reference_base = listener.local_addr().expect("local addr").to_string();
        let shutdown = Arc::new(AtomicBool::new(false));
        let layer_len = Arc::new(AtomicUsize::new(layer.len()));
        let resumed_offset = Arc::new(AtomicUsize::new(0));
        let resumed_bytes = Arc::new(AtomicUsize::new(0));
        let thread_state = ServerState {
            listener,
            layer: Arc::from(layer.into_boxed_slice()),
            manifest: Arc::from(manifest.into_boxed_slice()),
            resumed_offset: Arc::clone(&resumed_offset),
            resumed_bytes: Arc::clone(&resumed_bytes),
            shutdown: Arc::clone(&shutdown),
        };

        let handle = thread::spawn(move || thread_state.serve());

        Self {
            reference_base,
            layer_len,
            resumed_offset,
            resumed_bytes,
            shutdown,
            handle: Some(handle),
        }
    }

    pub(crate) fn reference(&self, repository: &str, tag: &str) -> String {
        format!("{}/{repository}:{tag}", self.reference_base)
    }

    /// The staged offset the resumed fetch asked for, if one was requested.
    pub(crate) fn resumed_offset(&self) -> Option<usize> {
        match self.resumed_offset.load(Ordering::SeqCst) {
            0 => None,
            offset => Some(offset),
        }
    }

    /// The bytes sent by resumed fetches so far.
    pub(crate) fn resumed_bytes(&self) -> usize {
        self.resumed_bytes.load(Ordering::SeqCst)
    }

    /// The full blob size.
    pub(crate) fn layer_len(&self) -> usize {
        self.layer_len.load(Ordering::SeqCst)
    }
}

impl Drop for ResumeServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            drop(handle.join());
        }
    }
}

#[derive(Debug)]
struct ServerState {
    listener: TcpListener,
    layer: Arc<[u8]>,
    manifest: Arc<[u8]>,
    resumed_offset: Arc<AtomicUsize>,
    resumed_bytes: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
}

/// Wait for a connection, returning `None` when the server must shut down.
fn wait_for_connection(listener: &TcpListener, shutdown: &AtomicBool) -> Option<TcpStream> {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return None;
        }
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

impl ServerState {
    fn serve(self) {
        let Self {
            listener,
            layer,
            manifest,
            resumed_offset,
            resumed_bytes,
            shutdown,
        } = self;
        while !shutdown.load(Ordering::SeqCst)
            && let Some(stream) = wait_for_connection(&listener, &shutdown)
        {
            answer_one(stream, &layer, &manifest, &resumed_offset, &resumed_bytes);
        }
    }
}

/// Answer one HTTP request: manifest, truncated blob, or 206 resume.
fn answer_one(
    mut stream: TcpStream,
    layer: &[u8],
    manifest: &[u8],
    resumed_offset: &AtomicUsize,
    resumed_bytes: &AtomicUsize,
) {
    let Some(request) = read_request_head(&mut stream) else {
        return;
    };
    if request.path.contains("/manifests/") {
        respond(
            &mut stream,
            "HTTP/1.1 200 OK",
            "application/vnd.oci.image.manifest.v1+json",
            manifest,
        );
    } else if let Some(offset) = request.range_offset() {
        resumed_offset.store(offset, Ordering::SeqCst);
        let rest = layer.get(offset..).unwrap_or(&[]).to_vec();
        resumed_bytes.fetch_add(rest.len(), Ordering::SeqCst);
        respond_resumable(&mut stream, offset, &rest, layer.len());
    } else if request.path.contains("/blobs/") {
        // Truncate the blob body and hang up mid-transfer.
        respond(
            &mut stream,
            "HTTP/1.1 200 OK",
            "application/octet-stream",
            layer,
        );
    } else {
        // Registry ping or unplanned path: a complete empty body.
        respond(&mut stream, "HTTP/1.1 200 OK", "text/plain", &[]);
    }
}

/// Write a 206 response serving the staged remainder of the blob.
fn respond_resumable(stream: &mut TcpStream, offset: usize, rest: &[u8], total: usize) {
    let header = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
        rest.len(),
        offset,
        total.saturating_sub(1),
        total,
    );
    stream
        .write_all(header.as_bytes())
        .expect("write 206 header");
    stream.write_all(rest).expect("write rest");
    stream.flush().expect("flush resume response");
}

/// Write a response with `Connection: close`, truncating blob bodies halfway.
fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "{status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len(),
    );
    stream.write_all(header.as_bytes()).expect("write header");
    if content_type == "application/octet-stream" {
        // Truncate blob transfers mid-body to force a Range resume.
        stream
            .write_all(body.get(..body.len() >> 1).unwrap_or(body))
            .expect("write partial body");
    } else {
        stream.write_all(body).expect("write body");
    }
    stream.flush().expect("flush response");
}

struct RequestHead {
    path: String,
    range_value: Option<String>,
}

impl RequestHead {
    /// The staged offset requested in the `Range` header, if any.
    fn range_offset(&self) -> Option<usize> {
        self.range_value
            .as_deref()?
            .strip_prefix("bytes=")?
            .split('-')
            .next()?
            .parse()
            .ok()
    }
}

/// Read an HTTP request head from `stream`, extracting path and Range value.
fn read_request_head(stream: &mut TcpStream) -> Option<RequestHead> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(chunk.get(..read)?);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let head_text = String::from_utf8(buffer).ok()?;
    let mut lines = head_text.lines();
    let request_line = lines.next()?;
    let path = request_line.split_whitespace().nth(1)?.to_owned();
    let range_value = lines
        .find(|line| line.to_ascii_lowercase().starts_with("range:"))
        .and_then(|line| line.split_once(':'))
        .map(|(_name, value)| value.trim().to_owned());

    Some(RequestHead { path, range_value })
}
