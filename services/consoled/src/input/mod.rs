//! Raw TTY input reader for decoding VT escape sequences and plain keys.

extern crate alloc;

mod escape;

use alloc::sync::Arc;
use core::time::Duration;
use std::fs::File;
use std::io::{self, Read as _};

use anyhow::Result;
use escape::Decoder;
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Events emitted by the input reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Up,
    Down,
    PageUp,
    PageDown,
    End,
    Escape,
}

const EVENT_CHANNEL_CAPACITY: usize = 64;
const READ_BUF_SIZE: usize = 16;
const SPLIT_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(50);

/// Spawns the input reader on `file`, emitting decoded events on the receiver.
///
/// # Errors
///
/// Returns an error when the file descriptor cannot be registered for asynchronous notifications.
pub fn spawn(file: Arc<File>) -> Result<mpsc::Receiver<InputEvent>> {
    let (tx, rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let async_fd = AsyncFd::new(file)?;

    tokio::spawn(async move {
        let _run_result = run(async_fd, tx).await;
    });

    Ok(rx)
}

async fn run(async_fd: AsyncFd<Arc<File>>, tx: mpsc::Sender<InputEvent>) -> Result<()> {
    let mut buf = [0_u8; READ_BUF_SIZE];
    let mut decoder = Decoder::default();

    loop {
        let (events, eof) = poll_events(&async_fd, &mut decoder, &mut buf).await?;
        send_events(events, &tx).await?;
        if eof {
            break;
        }
    }

    Ok(())
}

async fn poll_events(
    fd: &AsyncFd<Arc<File>>,
    decoder: &mut Decoder,
    buf: &mut [u8],
) -> Result<(Vec<InputEvent>, bool)> {
    let n = read_once(fd, buf).await?;
    if n == 0 {
        return Ok((Vec::new(), true));
    }

    let mut events = decoder.push(buf.get(..n).unwrap_or_default());

    while decoder.has_pending() {
        let Ok(Ok(n)) = timeout(SPLIT_SEQUENCE_TIMEOUT, read_once(fd, buf)).await else {
            events.extend(decoder.flush());
            return Ok((events, false));
        };
        if n == 0 {
            return Ok((events, true));
        }
        events.extend(decoder.push(buf.get(..n).unwrap_or_default()));
    }

    Ok((events, false))
}

async fn read_once(fd: &AsyncFd<Arc<File>>, buf: &mut [u8]) -> io::Result<usize> {
    let mut guard = fd.readable().await?;
    loop {
        match guard.get_inner().as_ref().read(buf) {
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

async fn send_events(events: Vec<InputEvent>, tx: &mpsc::Sender<InputEvent>) -> Result<()> {
    for event in events {
        tx.send(event).await?;
    }

    Ok(())
}
