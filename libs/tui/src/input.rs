//! Incremental decoder for terminal key input.

const MAX_PENDING_BYTES: usize = 16;

/// Key decoded from terminal input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page up key.
    PageUp,
    /// Page down key.
    PageDown,
    /// Enter or carriage return.
    Enter,
    /// Tab key.
    Tab,
    /// Backspace key.
    Backspace,
    /// Escape key.
    Esc,
    /// Delete key.
    Delete,
    /// Insert key.
    Insert,
    /// Printable character.
    Char(char),
}

/// Incremental decoder for VT escape sequences and plain keys.
#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
}

impl Decoder {
    /// Feeds raw bytes and returns every complete key found.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.pending.extend_from_slice(bytes);
        let mut keys = Vec::new();
        while self.next_sequence(&mut keys) {}
        self.trim_pending();

        keys
    }

    /// Emits a held-back lone `ESC` as `Esc`, dropping stale partials.
    pub fn flush(&mut self) -> Vec<Key> {
        let key = (self.pending.as_slice() == b"\x1b").then_some(Key::Esc);
        self.pending.clear();

        key.into_iter().collect()
    }

    /// Returns whether a partial sequence is buffered.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn next_sequence(&mut self, keys: &mut Vec<Key>) -> bool {
        self.take_plain(keys);
        let Some(start) = self.pending.iter().position(|&byte| byte == b'\x1b') else {
            return false;
        };
        self.pending.drain(..start);

        let Some(len) = self.sequence_len(keys) else {
            return false;
        };
        self.pending.drain(..len);

        true
    }

    fn sequence_len(&self, keys: &mut Vec<Key>) -> Option<usize> {
        let second = *self.pending.get(1)?;
        match second {
            b'[' => {
                let len = csi_end(self.pending.get(2..)?)?;
                let end = 2_usize.saturating_add(len);
                keys.extend(csi_key(self.pending.get(2..end)?));

                Some(end)
            }
            b'O' => {
                let seq = self.pending.get(2..3)?;
                keys.extend(ss3_key(seq));

                Some(3)
            }
            _ => {
                keys.push(Key::Esc);

                Some(1)
            }
        }
    }

    fn trim_pending(&mut self) {
        let keep = self.pending.first() == Some(&b'\x1b') && self.pending.len() < MAX_PENDING_BYTES;
        if !keep {
            self.pending.clear();
        }
    }

    fn take_plain(&mut self, keys: &mut Vec<Key>) {
        let plain_end = self
            .pending
            .iter()
            .position(|&byte| byte == b'\x1b')
            .unwrap_or(self.pending.len());
        if plain_end > 0 {
            keys.extend(self.pending.drain(..plain_end).filter_map(plain_key));
        }
    }
}

fn plain_key(byte: u8) -> Option<Key> {
    match byte {
        b'\r' | b'\n' => Some(Key::Enter),
        b'\t' => Some(Key::Tab),
        0x7f | 0x08 => Some(Key::Backspace),
        0x20..=0x7e => Some(Key::Char(char::from(byte))),
        _ => None,
    }
}

fn csi_end(seq: &[u8]) -> Option<usize> {
    seq.iter()
        .position(|&byte| byte.is_ascii_alphabetic() || byte == b'~')
        .map(|pos| pos.saturating_add(1))
}

fn csi_key(seq: &[u8]) -> Option<Key> {
    match seq {
        b"A" => Some(Key::Up),
        b"B" => Some(Key::Down),
        b"C" => Some(Key::Right),
        b"D" => Some(Key::Left),
        b"H" | b"1~" | b"7~" => Some(Key::Home),
        b"F" | b"4~" | b"8~" => Some(Key::End),
        b"2~" => Some(Key::Insert),
        b"3~" => Some(Key::Delete),
        b"5~" => Some(Key::PageUp),
        b"6~" => Some(Key::PageDown),
        _ => None,
    }
}

fn ss3_key(seq: &[u8]) -> Option<Key> {
    match seq {
        b"A" => Some(Key::Up),
        b"B" => Some(Key::Down),
        b"C" => Some(Key::Right),
        b"D" => Some(Key::Left),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(chunks: &[&[u8]]) -> Vec<Key> {
        let mut decoder = Decoder::default();
        let mut keys = Vec::new();
        for chunk in chunks {
            keys.extend(decoder.push(chunk));
        }
        keys.extend(decoder.flush());

        keys
    }

    #[test]
    fn decodes_arrow_sequences() {
        // ARRANGE
        let chunks = [b"\x1b[A\x1b[B\x1b[C\x1b[D".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::Up, Key::Down, Key::Right, Key::Left]);
    }

    #[test]
    fn decodes_home_and_end_variants() {
        // ARRANGE
        let chunks = [b"\x1b[H\x1b[F\x1b[1~\x1b[4~\x1b[7~\x1b[8~".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(
            keys,
            vec![
                Key::Home,
                Key::End,
                Key::Home,
                Key::End,
                Key::Home,
                Key::End
            ]
        );
    }

    #[test]
    fn decodes_navigation_sequences() {
        // ARRANGE
        let chunks = [b"\x1b[2~\x1b[3~\x1b[5~\x1b[6~".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(
            keys,
            vec![Key::Insert, Key::Delete, Key::PageUp, Key::PageDown]
        );
    }

    #[test]
    fn decodes_ss3_arrows() {
        // ARRANGE
        let chunks = [b"\x1bOA\x1bOB\x1bOC\x1bOD".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::Up, Key::Down, Key::Right, Key::Left]);
    }

    #[test]
    fn decodes_plain_ascii_keys() {
        // ARRANGE
        let chunks = [b"jk \r\t\x7f".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(
            keys,
            vec![
                Key::Char('j'),
                Key::Char('k'),
                Key::Char(' '),
                Key::Enter,
                Key::Tab,
                Key::Backspace
            ]
        );
    }

    #[test]
    fn ignores_unknown_sequences_and_controls() {
        // ARRANGE
        let chunks = [b"\x1b[Z\x01\x02".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, Vec::<Key>::new());
    }

    #[test]
    fn decodes_escape_split_across_pushes() {
        // ARRANGE
        let chunks = [b"\x1b".as_slice(), b"[A".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::Up]);
    }

    #[test]
    fn decodes_csi_split_mid_parameter() {
        // ARRANGE
        let chunks = [b"\x1b[5".as_slice(), b"~".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::PageUp]);
    }

    #[test]
    fn decodes_ss3_split_across_pushes() {
        // ARRANGE
        let chunks = [b"\x1bO".as_slice(), b"A".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::Up]);
    }

    #[test]
    fn escape_not_followed_by_sequence_is_escape() {
        // ARRANGE
        let chunks = [b"\x1bx".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::Esc, Key::Char('x')]);
    }

    #[test]
    fn lone_escape_flushes_as_escape() {
        // ARRANGE
        let chunks = [b"\x1b".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::Esc]);
    }

    #[test]
    fn lone_escape_followed_by_sequence_decodes_both() {
        // ARRANGE
        let chunks = [b"\x1b\x1b[A".as_slice()];

        // ACT
        let keys = decode_all(&chunks);

        // ASSERT
        assert_eq!(keys, vec![Key::Esc, Key::Up]);
    }

    #[test]
    fn flush_discards_incomplete_csi() {
        // ARRANGE
        let mut decoder = Decoder::default();

        // ACT
        let first = decoder.push(b"\x1b[5");
        let second = decoder.push(b"A");

        // ASSERT
        assert_eq!(first, Vec::<Key>::new());
        assert_eq!(second, Vec::<Key>::new());
    }

    #[test]
    fn unterminated_garbage_is_dropped() {
        // ARRANGE
        let mut decoder = Decoder::default();

        // ACT
        let first = decoder.push(b"\x1b[12345678901234");
        let second = decoder.push(b"A");

        // ASSERT
        assert_eq!(first, Vec::<Key>::new());
        assert_eq!(second, vec![Key::Char('A')]);
    }
}
