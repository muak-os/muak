//! Integration tests for zeroing regions of a device.

pub mod common;

#[cfg(test)]
mod tests {
    use partdev::clear;

    use super::common::Fixture;

    const HEAD_BYTES: usize = 10 * 1024 * 1024;
    const TAIL_BYTES: usize = 1024 * 1024;

    #[test]
    fn fast_zeroes_the_head_and_tail_but_not_the_middle() {
        // ARRANGE
        let fixture = Fixture::new(24 * 1024);
        fixture.fill(0xAA);

        // ACT
        clear::fast(fixture.path_str()).expect("fast clear");

        // ASSERT
        let bytes = fixture.read_all();
        assert!(bytes[..HEAD_BYTES].iter().all(|byte| *byte == 0), "head");
        assert!(
            bytes[bytes.len() - TAIL_BYTES..]
                .iter()
                .all(|byte| *byte == 0),
            "tail"
        );
        assert_eq!(bytes[HEAD_BYTES], 0xAA, "middle must stay untouched");
    }

    #[test]
    fn zero_zeroes_the_whole_device() {
        // ARRANGE
        let fixture = Fixture::new(16);
        fixture.fill(0xAA);

        // ACT
        clear::zero(fixture.path_str()).expect("zero clear");

        // ASSERT
        assert!(fixture.read_all().iter().all(|byte| *byte == 0));
    }
}
