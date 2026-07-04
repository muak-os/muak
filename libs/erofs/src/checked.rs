pub(crate) fn add(lhs: usize, rhs: usize) -> Option<usize> {
    lhs.checked_add(rhs)
}

pub(crate) fn sub(lhs: usize, rhs: usize) -> Option<usize> {
    lhs.checked_sub(rhs)
}

pub(crate) fn align_up(value: usize, alignment: usize) -> Option<usize> {
    let mask = alignment.checked_sub(1)?;
    value.checked_add(mask).map(|aligned| aligned & !mask)
}

pub(crate) fn read_bytes<const N: usize>(buf: &[u8], offset: usize) -> Option<[u8; N]> {
    let end = offset.checked_add(N)?;
    let slice = buf.get(offset..end)?;
    let mut bytes = [0_u8; N];
    bytes.copy_from_slice(slice);
    Some(bytes)
}

pub(crate) fn write_byte(buf: &mut [u8], offset: usize, value: u8) -> bool {
    if let Some(slot) = buf.get_mut(offset) {
        *slot = value;
        true
    } else {
        false
    }
}

pub(crate) fn write_bytes(buf: &mut [u8], offset: usize, value: &[u8]) -> bool {
    let Some(end) = offset.checked_add(value.len()) else {
        return false;
    };
    let Some(dst) = buf.get_mut(offset..end) else {
        return false;
    };
    dst.copy_from_slice(value);
    true
}

pub(crate) fn u8_from_usize(value: usize) -> Option<u8> {
    u8::try_from(value).ok()
}

pub(crate) fn u16_from_usize(value: usize) -> Option<u16> {
    u16::try_from(value).ok()
}

pub(crate) fn u32_from_usize(value: usize) -> Option<u32> {
    u32::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_byte_returns_false_out_of_bounds() {
        // ARRANGE
        let mut buf = [0_u8; 1];

        // ACT
        let wrote = write_byte(&mut buf, 2, 1);

        // ASSERT
        assert!(!wrote);
    }

    #[test]
    fn write_bytes_returns_false_on_overflow_and_range_miss() {
        // ARRANGE
        let mut buf = [0_u8; 2];

        // ACT
        let overflow = write_bytes(&mut buf, usize::MAX, &[1]);
        let range_miss = write_bytes(&mut buf, 2, &[1]);

        // ASSERT
        assert!(!overflow);
        assert!(!range_miss);
    }

    #[test]
    fn align_up_returns_none_when_alignment_or_sum_overflows() {
        // ARRANGE & ACT
        let zero_alignment = align_up(4, 0);
        let overflowing = align_up(usize::MAX, 8);

        // ASSERT
        assert!(zero_alignment.is_none());
        assert!(overflowing.is_none());
    }
}
