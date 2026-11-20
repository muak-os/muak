//! Internet checksum helpers, as defined by RFC 1071.

/// Computes the 16 bit one's complement checksum of `data`.
#[must_use = "callers need the checksum"]
pub fn sum(data: &[u8]) -> u16 {
    finish(accumulate(0, data))
}

/// Computes the 16 bit one's complement checksum of `chunks` summed as one stream.
#[must_use = "callers need the checksum"]
pub fn sum_chunks(chunks: &[&[u8]]) -> u16 {
    debug_assert!(
        chunks.iter().all(|chunk| chunk.len() & 1 == 0),
        "every chunk must have an even length"
    );

    finish(chunks.iter().fold(0, |sum, chunk| accumulate(sum, chunk)))
}

/// Adds the big-endian 16 bit words of `data` to `sum`.
#[must_use = "callers need the accumulated sum"]
pub(crate) fn accumulate(sum: u32, data: &[u8]) -> u32 {
    let mut total = sum;
    let mut bytes = data.iter().copied();
    while let Some(high) = bytes.next() {
        let low = bytes.next().unwrap_or(0);
        total = total.saturating_add(u32::from(u16::from_be_bytes([high, low])));
    }

    total
}

/// Folds the carry bits of `sum` and returns the one's complement.
#[must_use = "callers need the folded checksum"]
pub(crate) fn finish(mut sum: u32) -> u16 {
    while sum >> 16_u32 != 0 {
        sum = (sum & 0xFFFF).saturating_add(sum >> 16);
    }

    !u16::try_from(sum).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_of_set_words_complements_to_zero() {
        // ARRANGE
        let data = [0xFF_u8, 0xFF];

        // ACT
        let total = sum(&data);

        // ASSERT
        assert_eq!(total, 0);
    }

    #[test]
    fn sum_chunks_of_no_chunks_matches_sum_of_nothing() {
        // ARRANGE
        let chunks: [&[u8]; 0] = [];

        // ACT
        let total = sum_chunks(&chunks);

        // ASSERT
        assert_eq!(total, sum(&[]));
    }

    #[test]
    fn sum_chunks_matches_sum_of_concatenation() {
        // ARRANGE
        let head = [0x12_u8, 0x34];
        let tail = [0x56_u8, 0x78, 0x9a, 0xbc];
        let joined = [0x12_u8, 0x34, 0x56, 0x78, 0x9a, 0xbc];

        // ACT
        let total = sum_chunks(&[&head, &tail]);

        // ASSERT
        assert_eq!(total, sum(&joined));
    }

    #[test]
    fn sum_of_zero_words_complements_to_all_ones() {
        // ARRANGE
        let data = [0x00_u8, 0x00];

        // ACT
        let total = sum(&data);

        // ASSERT
        assert_eq!(total, 0xFFFF);
    }
}
