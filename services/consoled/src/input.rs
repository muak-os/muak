//! Raw TTY input reader for decoding VT escape sequences.

extern crate alloc;

use alloc::sync::Arc;
use std::fs::File;
use std::io::{self, Read as _};

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

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

pub fn spawn(file: Arc<File>) -> anyhow::Result<mpsc::UnboundedReceiver<InputEvent>> {
    let (tx, rx) = mpsc::unbounded_channel();
    let async_fd = AsyncFd::new(file)?;

    tokio::spawn(async move {
        let _run_result = run(async_fd, tx).await;
    });

    Ok(rx)
}

async fn run(
    async_fd: AsyncFd<Arc<File>>,
    tx: mpsc::UnboundedSender<InputEvent>,
) -> io::Result<()> {
    let mut buf = [0_u8; 16];

    loop {
        let mut guard = async_fd.readable().await?;

        let n = match guard.get_inner().as_ref().read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                guard.clear_ready();
                continue;
            }
            Err(e) => return Err(e),
        };

        guard.clear_ready();
        send_events(decode_events(buf.get(..n).unwrap_or_default()), &tx)?;
    }

    Ok(())
}

fn send_events(events: Vec<InputEvent>, tx: &mpsc::UnboundedSender<InputEvent>) -> io::Result<()> {
    for event in events {
        tx.send(event)
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
    }
    Ok(())
}

/// Decodes zero or more `InputEvent`s from a raw byte slice.
fn decode_events(buf: &[u8]) -> Vec<InputEvent> {
    let mut events = Vec::new();
    let mut i = 0;

    while i < buf.len() {
        if buf.get(i).copied() != Some(b'\x1b') {
            i = i.saturating_add(1);
            continue;
        }

        if buf.get(i.saturating_add(1)).copied() != Some(b'[') {
            events.push(InputEvent::Escape);
            i = i.saturating_add(1);
            continue;
        }

        let seq_start = i.saturating_add(2);
        let seq_end = buf
            .get(seq_start..)
            .unwrap_or_default()
            .iter()
            .position(|&byte| byte.is_ascii_alphabetic() || byte == b'~')
            .map_or(buf.len(), |pos| {
                seq_start.saturating_add(pos).saturating_add(1)
            });

        if let Some(event) = decode_csi(buf.get(seq_start..seq_end).unwrap_or_default()) {
            events.push(event);
        }

        i = seq_end;
    }

    events
}

fn decode_csi(seq: &[u8]) -> Option<InputEvent> {
    match seq {
        b"A" => Some(InputEvent::Up),
        b"B" => Some(InputEvent::Down),
        b"5~" => Some(InputEvent::PageUp),
        b"6~" => Some(InputEvent::PageDown),
        b"F" | b"4~" => Some(InputEvent::End),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_up_arrow() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[A");

        // ASSERT
        assert_eq!(events, vec![InputEvent::Up]);
    }

    #[test]
    fn decode_down_arrow() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[B");

        // ASSERT
        assert_eq!(events, vec![InputEvent::Down]);
    }

    #[test]
    fn decode_page_up() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[5~");

        // ASSERT
        assert_eq!(events, vec![InputEvent::PageUp]);
    }

    #[test]
    fn decode_page_down() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[6~");

        // ASSERT
        assert_eq!(events, vec![InputEvent::PageDown]);
    }

    #[test]
    fn decode_end_f_suffix() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[F");

        // ASSERT
        assert_eq!(events, vec![InputEvent::End]);
    }

    #[test]
    fn decode_end_tilde_suffix() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[4~");

        // ASSERT
        assert_eq!(events, vec![InputEvent::End]);
    }

    #[test]
    fn decode_unknown_sequence_returns_empty() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[Z");

        // ASSERT
        assert!(events.is_empty());
    }

    #[test]
    fn decode_multiple_events_in_one_buffer() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b[A\x1b[B\x1b[5~");

        // ASSERT
        assert_eq!(
            events,
            vec![InputEvent::Up, InputEvent::Down, InputEvent::PageUp]
        );
    }

    #[test]
    fn decode_non_escape_bytes_ignored() {
        // ARRANGE / ACT
        let events = decode_events(b"hello");

        // ASSERT
        assert!(events.is_empty());
    }

    #[test]
    fn decode_bare_escape_key() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1b");

        // ASSERT
        assert_eq!(events, vec![InputEvent::Escape]);
    }

    #[test]
    fn decode_escape_not_followed_by_bracket() {
        // ARRANGE / ACT
        let events = decode_events(b"\x1bx");

        // ASSERT
        assert_eq!(events, vec![InputEvent::Escape]);
    }
}
