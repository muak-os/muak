//! DHCP transaction ID generation and extraction.

use super::field;
use crate::{Failure, Result};

/// Generates a cryptographically random 32-bit DHCP transaction ID.
///
/// # Errors
///
/// Returns [`Failure::Xid`] when the system random number generator fails.
pub(crate) fn generate() -> Result<u32> {
    let mut buf = [0_u8; 4];
    getrandom::fill(&mut buf).map_err(|_error| Failure::Xid)?;

    Ok(u32::from_be_bytes(buf))
}

/// Extracts the transaction ID from a raw DHCP message, reading `0` for missing bytes.
pub(crate) fn extract(buf: &[u8]) -> u32 {
    let end = field::XID.saturating_add(4);

    buf.get(field::XID..end)
        .and_then(|slot| slot.first_chunk::<4>())
        .map_or(0, |octets| u32::from_be_bytes(*octets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_MESSAGE_LEN;
    use crate::wire::header;

    #[test]
    fn generate_produces_value() {
        // ACT / ASSERT
        let _xid = generate().expect("should generate xid");
    }

    #[test]
    fn generate_produces_different_values() {
        // ACT
        let first = generate().expect("xid a");
        let second = generate().expect("xid b");

        // ASSERT
        assert_ne!(first, second);
    }

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
}
