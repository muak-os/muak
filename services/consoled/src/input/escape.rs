//! Incremental decoder for VT escape sequences arriving in arbitrary chunks.

use super::InputEvent;

const MAX_PENDING_BYTES: usize = 16;

#[derive(Default)]
pub(super) struct Decoder {
    pending: Vec<u8>,
}

impl Decoder {
    /// Feeds raw bytes and returns every complete event found.
    pub(super) fn push(&mut self, bytes: &[u8]) -> Vec<InputEvent> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while self.next_sequence(&mut events) {}
        self.trim_pending();

        events
    }

    /// Emits a held-back lone `ESC` as `Escape`, dropping stale partials.
    pub(super) fn flush(&mut self) -> Vec<InputEvent> {
        let event = (self.pending.as_slice() == b"\x1b").then_some(InputEvent::Escape);
        self.pending.clear();

        event.into_iter().collect()
    }

    pub(super) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn next_sequence(&mut self, events: &mut Vec<InputEvent>) -> bool {
        let Some(start) = self.pending.iter().position(|&byte| byte == b'\x1b') else {
            return false;
        };
        self.pending.drain(..start);

        let Some(len) = self.sequence_len(events) else {
            return false;
        };
        self.pending.drain(..len);

        true
    }

    fn sequence_len(&self, events: &mut Vec<InputEvent>) -> Option<usize> {
        let second = self.pending.get(1)?;
        if second == &b'[' {
            let len = csi_end(self.pending.get(2..)?)?;
            let end = 2_usize.saturating_add(len);
            events.extend(csi_event(self.pending.get(2..end)?));

            Some(end)
        } else {
            events.push(InputEvent::Escape);

            Some(1)
        }
    }

    fn trim_pending(&mut self) {
        let keep = self.pending.first() == Some(&b'\x1b') && self.pending.len() < MAX_PENDING_BYTES;
        if !keep {
            self.pending.clear();
        }
    }
}

fn csi_end(seq: &[u8]) -> Option<usize> {
    seq.iter()
        .position(|&byte| byte.is_ascii_alphabetic() || byte == b'~')
        .map(|pos| pos.saturating_add(1))
}

fn csi_event(seq: &[u8]) -> Option<InputEvent> {
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

    // Feeds `chunks` to a decoder in order, then flushes it.
    fn decode_all(chunks: &[&[u8]]) -> Vec<InputEvent> {
        let mut decoder = Decoder::default();
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(decoder.push(chunk));
        }
        events.extend(decoder.flush());
        events
    }

    #[test]
    fn decode_up_arrow() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[A"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::Up]);
    }

    #[test]
    fn decode_down_arrow() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[B"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::Down]);
    }

    #[test]
    fn decode_page_up() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[5~"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::PageUp]);
    }

    #[test]
    fn decode_page_down() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[6~"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::PageDown]);
    }

    #[test]
    fn decode_end_f_suffix() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[F"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::End]);
    }

    #[test]
    fn decode_end_tilde_suffix() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[4~"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::End]);
    }

    #[test]
    fn decode_unknown_sequence_returns_empty() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[Z"]);

        // ASSERT
        assert!(events.is_empty());
    }

    #[test]
    fn decode_multiple_events_in_one_buffer() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[A\x1b[B\x1b[5~"]);

        // ASSERT
        assert_eq!(
            events,
            vec![InputEvent::Up, InputEvent::Down, InputEvent::PageUp]
        );
    }

    #[test]
    fn decode_non_escape_bytes_ignored() {
        // ARRANGE / ACT
        let events = decode_all(&[b"hello"]);

        // ASSERT
        assert!(events.is_empty());
    }

    #[test]
    fn decode_escape_split_across_pushes() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b", b"[A"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::Up]);
    }

    #[test]
    fn decode_csi_split_mid_parameter() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b[5", b"~"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::PageUp]);
    }

    #[test]
    fn decode_escape_not_followed_by_bracket() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1bx"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::Escape]);
    }

    #[test]
    fn flush_lone_escape_as_escape_event() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::Escape]);
    }

    #[test]
    fn lone_escape_followed_by_sequence_decodes_both() {
        // ARRANGE / ACT
        let events = decode_all(&[b"\x1b", b"\x1b[A"]);

        // ASSERT
        assert_eq!(events, vec![InputEvent::Escape, InputEvent::Up]);
    }

    #[test]
    fn flush_discards_incomplete_csi() {
        // ARRANGE
        let mut decoder = Decoder::default();

        // ACT
        let first = decoder.push(b"\x1b[5");
        let second = decoder.push(b"A");

        // ASSERT
        assert!(first.is_empty());
        assert!(second.is_empty());
    }

    #[test]
    fn unterminated_garbage_is_dropped() {
        // ARRANGE
        let mut decoder = Decoder::default();

        // ACT
        let first = decoder.push(b"\x1b[12345678901234");
        let second = decoder.push(b"A");

        // ASSERT
        assert!(first.is_empty());
        assert!(second.is_empty());
    }
}
