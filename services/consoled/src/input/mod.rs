//! Raw TTY input reader for decoding VT escape sequences and plain keys.

use core::time::Duration;
use std::fs::File;
use std::io::{self, Read as _};

use anyhow::Result;
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tui::input::{Decoder, Key};

const EVENT_CHANNEL_CAPACITY: usize = 64;
const READ_BUF_SIZE: usize = 16;
const SPLIT_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(50);

/// Spawns the input reader on `file`, emitting decoded keys on the receiver.
///
/// # Errors
///
/// Returns an error when the file descriptor cannot be registered for asynchronous notifications.
pub fn spawn(file: File) -> Result<mpsc::Receiver<Key>> {
    let (tx, rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    // SAFETY: `file` uniquely owns the descriptor and stays open for the lifetime of
    // the `AsyncFd`, so the registered descriptor remains valid and unchanged.
    let async_fd = unsafe { AsyncFd::register(file) }.map_err(io::Error::from)?;

    tokio::spawn(async move {
        let _run_result = run(async_fd, tx).await;
    });

    Ok(rx)
}

async fn run(async_fd: AsyncFd<File>, tx: mpsc::Sender<Key>) -> Result<()> {
    let mut buf = [0_u8; READ_BUF_SIZE];
    let mut decoder = Decoder::default();

    loop {
        let (keys, eof) = poll_events(&async_fd, &mut decoder, &mut buf).await?;
        send_keys(keys, &tx).await?;
        if eof {
            break;
        }
    }

    Ok(())
}

async fn poll_events(
    fd: &AsyncFd<File>,
    decoder: &mut Decoder,
    buf: &mut [u8],
) -> Result<(Vec<Key>, bool)> {
    let n = read_once(fd, buf).await?;
    if n == 0 {
        return Ok((Vec::new(), true));
    }

    let mut keys = decoder.push(buf.get(..n).unwrap_or_default());

    while decoder.has_pending() {
        let Ok(Ok(n)) = timeout(SPLIT_SEQUENCE_TIMEOUT, read_once(fd, buf)).await else {
            keys.extend(decoder.flush());
            return Ok((keys, false));
        };
        if n == 0 {
            return Ok((keys, true));
        }
        keys.extend(decoder.push(buf.get(..n).unwrap_or_default()));
    }

    Ok((keys, false))
}

async fn read_once(fd: &AsyncFd<File>, buf: &mut [u8]) -> io::Result<usize> {
    let mut guard = fd.readable().await?;
    loop {
        match guard.get_inner().read(buf) {
            Ok(n) => {
                guard.clear_ready();
                return Ok(n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                guard.clear_ready();
                guard = fd.readable().await?;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn send_keys(keys: Vec<Key>, tx: &mpsc::Sender<Key>) -> Result<()> {
    for key in keys {
        tx.send(key).await?;
    }

    Ok(())
}
