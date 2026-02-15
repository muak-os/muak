/// Aligns a value up to the nearest multiple of the given alignment.
#[inline]
pub fn align_to(value: u32, alignment: u32) -> u32 {
    if alignment == 0 {
        return value;
    }
    (value + alignment - 1) & !(alignment - 1)
}

/// Reads a little-endian u32 from a byte buffer at the given offset.
#[inline]
pub fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Writes a little-endian u32 to a byte buffer at the given offset.
#[inline]
pub fn write_u32(buf: &mut [u8], off: usize, val: u32) {
    buf[off..off + 4].copy_from_slice(&val.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_to_zero_alignment() {
        assert_eq!(align_to(100, 0), 100);
        assert_eq!(align_to(0, 0), 0);
        assert_eq!(align_to(u32::MAX, 0), u32::MAX);
    }

    #[test]
    fn test_align_to_already_aligned() {
        assert_eq!(align_to(0, 4096), 0);
        assert_eq!(align_to(4096, 4096), 4096);
        assert_eq!(align_to(8192, 4096), 8192);
    }

    #[test]
    fn test_align_to_needs_alignment() {
        assert_eq!(align_to(1, 4), 4);
        assert_eq!(align_to(100, 4), 100);
        assert_eq!(align_to(101, 4), 104);
        assert_eq!(align_to(4095, 4096), 4096);
    }

    #[test]
    fn test_align_to_various_alignments() {
        assert_eq!(align_to(10, 8), 16);
        assert_eq!(align_to(16, 8), 16);
        assert_eq!(align_to(17, 8), 24);
        assert_eq!(align_to(50, 16), 64);
        assert_eq!(align_to(64, 16), 64);
    }

    #[test]
    fn test_align_to_power_of_two() {
        for alignment in &[2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096] {
            let alignment = *alignment;
            // Test values that are already aligned
            assert_eq!(align_to(0, alignment), 0);
            assert_eq!(align_to(alignment, alignment), alignment);
            assert_eq!(align_to(alignment * 2, alignment), alignment * 2);

            // Test values that need alignment
            assert_eq!(align_to(1, alignment), alignment);
            assert_eq!(align_to(alignment - 1, alignment), alignment);
            assert_eq!(align_to(alignment + 1, alignment), alignment * 2);
        }
    }

    #[test]
    fn test_read_u32_basic() {
        let buf = [0x78, 0x56, 0x34, 0x12, 0xFF, 0xFF];
        assert_eq!(read_u32(&buf, 0), 0x12345678);
    }

    #[test]
    fn test_read_u32_different_offsets() {
        let buf = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        assert_eq!(read_u32(&buf, 0), 0x33221100);
        assert_eq!(read_u32(&buf, 1), 0x44332211);
        assert_eq!(read_u32(&buf, 4), 0x77665544);
    }

    #[test]
    fn test_read_u32_all_zeros() {
        let buf = [0x00, 0x00, 0x00, 0x00];
        assert_eq!(read_u32(&buf, 0), 0);
    }

    #[test]
    fn test_read_u32_all_ones() {
        let buf = [0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(read_u32(&buf, 0), 0xFFFFFFFF);
    }

    #[test]
    fn test_read_u32_little_endian() {
        let buf = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(read_u32(&buf, 0), 0x04030201);
    }

    #[test]
    fn test_write_u32_basic() {
        let mut buf = [0u8; 4];
        write_u32(&mut buf, 0, 0x12345678);
        assert_eq!(buf, [0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn test_write_u32_different_offsets() {
        let mut buf = [0u8; 8];
        write_u32(&mut buf, 0, 0x11223344);
        write_u32(&mut buf, 4, 0x55667788);
        assert_eq!(buf, [0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55]);
    }

    #[test]
    fn test_write_u32_overwrites_correctly() {
        let mut buf = [0xFF; 8];
        write_u32(&mut buf, 2, 0x00000000);
        assert_eq!(buf[2..6], [0x00, 0x00, 0x00, 0x00]);
        assert_eq!(buf[0], 0xFF);
        assert_eq!(buf[1], 0xFF);
        assert_eq!(buf[6], 0xFF);
        assert_eq!(buf[7], 0xFF);
    }

    #[test]
    fn test_write_u32_zero_value() {
        let mut buf = [0xFF; 4];
        write_u32(&mut buf, 0, 0);
        assert_eq!(buf, [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_write_u32_max_value() {
        let mut buf = [0x00; 4];
        write_u32(&mut buf, 0, 0xFFFFFFFF);
        assert_eq!(buf, [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn test_read_write_u32_roundtrip() {
        let mut buf = [0u8; 4];
        let test_values = [0x00000000, 0x12345678, 0xDEADBEEF, 0xFFFFFFFF, 0x00000001];
        for &value in &test_values {
            write_u32(&mut buf, 0, value);
            assert_eq!(read_u32(&buf, 0), value);
        }
    }

    #[test]
    fn test_align_to_boundary_conditions() {
        // Test boundary at alignment size
        assert_eq!(align_to(511, 512), 512);
        assert_eq!(align_to(512, 512), 512);
        assert_eq!(align_to(513, 512), 1024);

        // Test with alignment = 1 (edge case, should return value)
        assert_eq!(align_to(100, 1), 100);
        assert_eq!(align_to(0, 1), 0);
    }
}
