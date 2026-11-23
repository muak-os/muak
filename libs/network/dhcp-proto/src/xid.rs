//! DHCP transaction ID extraction.

use super::field;

/// Extracts the transaction ID from a raw DHCP message, reading `0` for missing bytes.
#[must_use = "the transaction ID is the result of parsing"]
pub fn extract(buf: &[u8]) -> u32 {
    let end = field::XID.saturating_add(4);

    buf.get(field::XID..end)
        .and_then(|slot| slot.first_chunk::<4>())
        .map_or(0, |octets| u32::from_be_bytes(*octets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header;

    // Buffer large enough for any message the builders produce.
    const MAX_MESSAGE_LEN: usize = 1500;

    #[test]
    fn extract_reads_xid() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let message = header::build(&mut buf, 0xCAFE_BABE, [0; 6]).expect("header fits");

        // ACT
        let result = extract(message);

        // ASSERT
        assert_eq!(result, 0xCAFE_BABE);
    }

    #[test]
    fn extract_reads_zero_for_short_buffer() {
        // ARRANGE
        let buf = [0xFF_u8; field::XID];

        // ACT
        let result = extract(&buf);

        // ASSERT
        assert_eq!(result, 0);
    }
}
