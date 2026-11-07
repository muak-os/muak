//! Integration tests for positioned device I/O.

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};

    use blockdev::io::{self, Positioned};

    #[test]
    fn write_at_then_read_at_round_trips_at_a_non_zero_offset() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().set_len(4096).expect("resize");
        file.as_file().write_all(&[0_u8; 4096]).expect("zero fill");
        let payload = [0xAB_u8, 0xCD, 0xEF, 0x01];

        // ACT
        io::write_at(file.as_file(), 1024, &payload).expect("positioned write");
        let mut readback = [0_u8; 4];
        io::read_at(file.as_file(), 1024, &mut readback).expect("positioned read");

        // ASSERT
        assert_eq!(readback, payload, "payload survives the round trip");
    }

    #[test]
    fn read_at_past_the_end_reports_unexpected_eof() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().set_len(16).expect("resize");
        let mut buffer = [0_u8; 4];

        // ACT
        let error = io::read_at(file.as_file(), 14, &mut buffer).expect_err("short read");

        // ASSERT
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn positioned_reads_advance_and_writes_land_at_the_offset() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().set_len(4096).expect("resize");
        file.as_file().write_all(&[0_u8; 4096]).expect("zero fill");
        let first = [1_u8, 2, 3, 4];
        let second = [5_u8, 6, 7, 8];
        io::write_at(file.as_file(), 256, &first).expect("seed first half");
        io::write_at(file.as_file(), 260, &second).expect("seed second half");
        let mut positioned = Positioned::at(file.as_file(), 256);
        let mut first_read = [0_u8; 4];
        let mut second_read = [0_u8; 4];

        // ACT
        positioned.read_exact(&mut first_read).expect("first read");
        positioned
            .read_exact(&mut second_read)
            .expect("second read");
        let written = [9_u8, 10, 11, 12];
        let mut writer = Positioned::at(file.as_file(), 512);
        writer.write_all(&written).expect("positioned write");
        let mut landed = [0_u8; 4];
        io::read_at(file.as_file(), 512, &mut landed).expect("read back write");

        // ASSERT
        assert_eq!(first_read, first, "first read sees the seeded bytes");
        assert_eq!(second_read, second, "second read advances past the first");
        assert_eq!(landed, written, "write lands at the positioned offset");
    }

    #[test]
    fn sync_on_a_regular_file_succeeds() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().write_all(b"durable").expect("write");

        // ACT
        let result = io::sync(file.as_file());

        // ASSERT
        assert!(result.is_ok(), "flush reaches the device");
    }
}
