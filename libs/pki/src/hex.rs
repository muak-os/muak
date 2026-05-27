//! Hex encoding helpers.

/// Converts a byte slice to a lowercase hex string.
#[must_use]
pub fn encode_lower(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        hex.push(hex_char(byte >> 4));
        hex.push(hex_char(byte & 0x0f));
    }
    hex
}

const fn hex_char(nibble: u8) -> char {
    match nibble {
        1 => '1',
        2 => '2',
        3 => '3',
        4 => '4',
        5 => '5',
        6 => '6',
        7 => '7',
        8 => '8',
        9 => '9',
        10 => 'a',
        11 => 'b',
        12 => 'c',
        13 => 'd',
        14 => 'e',
        15 => 'f',
        _ => '0',
    }
}
