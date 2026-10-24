#![cfg(test)]

//! Shared fixtures for tui integration tests.

/// Removes ANSI escape sequences from `bytes`.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while let Some(byte) = bytes.get(index).copied() {
        if byte == 0x1b {
            index = skip_escape(bytes, index.saturating_add(1));
            continue;
        }

        output.push(byte);
        index = index.saturating_add(1);
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn skip_escape(bytes: &[u8], start: usize) -> usize {
    if bytes.get(start).copied() != Some(b'[') {
        return start;
    }

    let mut index = start.saturating_add(1);
    while let Some(next) = bytes.get(index).copied() {
        index = index.saturating_add(1);
        if (0x40..=0x7e).contains(&next) {
            break;
        }
    }

    index
}
