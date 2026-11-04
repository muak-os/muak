//! Shared canned-response servers for integration tests.

use std::io::{self, Read as _, Write as _};
use std::net::TcpListener;
use std::thread;

const TRUNCATED_BODY: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Length: 32\r\nConnection: close\r\n\r\nshort";

/// Serve one blob body shorter than its declared length and return the URL
/// that requests it.
///
/// # Errors
///
/// Returns an error when the server thread cannot bind its socket.
pub fn spawn_truncated_body() -> io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    thread::spawn(move || {
        let _served = serve_truncated_body(&listener);
    });

    Ok(format!("http://{address}/v2/repo/blobs/sha256:abc"))
}

fn serve_truncated_body(listener: &TcpListener) -> io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    let mut request = [0_u8; 2048];
    let _: usize = stream.read(&mut request)?;
    stream.write_all(TRUNCATED_BODY)?;
    stream.flush()
}
