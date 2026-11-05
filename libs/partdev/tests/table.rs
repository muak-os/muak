//! Integration tests for reading and writing the on-disk GPT.

pub mod common;

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use partdev::error::PartdevError;
    use partdev::table;

    use super::common::{Fixture, SECTOR_BYTES, gpt_table, read_at};

    const ENTRIES_BYTES: u64 = 0x4000;

    #[test]
    fn commit_then_read_round_trips() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        fixture.write_gpt(&["EFI"]);

        // ACT
        let gpt = table::read(fixture.file()).expect("read committed GPT");

        // ASSERT
        let partition = gpt.partition(1).expect("partition 1");
        assert_eq!(partition.name, "EFI");
        assert_eq!(partition.starting_lba, 2048);
    }

    #[test]
    fn commit_places_primary_and_backup_headers() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        let gpt = gpt_table(fixture.sectors(), &["EFI"]);
        let backup_data_offset = gpt.backup_data_offset(fixture.sectors());
        table::commit(fixture.file(), &gpt, fixture.sectors()).expect("commit");

        // ACT
        let primary = read_at(fixture.file(), 512, 8);
        let backup = read_at(
            fixture.file(),
            backup_data_offset.saturating_add(ENTRIES_BYTES),
            8,
        );

        // ASSERT
        assert_eq!(primary.as_slice(), b"EFI PART", "primary header at byte 0");
        assert_eq!(
            backup.as_slice(),
            b"EFI PART",
            "backup header before the end"
        );
        assert_eq!(
            backup_data_offset.saturating_add(ENTRIES_BYTES),
            fixture.sectors().saturating_sub(1).saturating_mul(512),
            "the backup header lands in the last sector"
        );
    }

    #[test]
    fn commit_rejects_zero_sector_count() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        let gpt = gpt_table(fixture.sectors(), &["EFI"]);

        // ACT
        let result = table::commit(fixture.file(), &gpt, 0);

        // ASSERT
        assert!(
            matches!(result, Err(PartdevError::UnknownSectorCount)),
            "a zero sector count would place the backup GPT at byte 0"
        );
    }

    #[test]
    fn commit_leaves_the_file_cursor_unchanged() {
        // ARRANGE
        // The crate exposes no cursor position, so the cursor is observed through
        // a sequential read instead.
        let fixture = Fixture::new(2048);
        let gpt = gpt_table(fixture.sectors(), &["EFI"]);
        let mut file = fixture.file();
        let mut probe = [0_u8; 1];
        file.read_exact(&mut probe)
            .expect("advance cursor to offset 1");

        // ACT
        table::commit(file, &gpt, fixture.sectors()).expect("commit");

        // ASSERT
        let expected = usize::try_from(fixture.sectors().saturating_mul(SECTOR_BYTES))
            .unwrap_or(0)
            .saturating_sub(1);
        let mut rest = vec![0_u8; expected];
        file.read_exact(&mut rest)
            .expect("the cursor must stay at offset 1 across commit");
        assert_eq!(
            file.read(&mut [0_u8; 1]).expect("probe end of file"),
            0,
            "commit must not move the file cursor from offset 1"
        );
    }

    #[test]
    fn read_on_blank_file_errors() {
        // ARRANGE
        let fixture = Fixture::new(2048);

        // ACT
        let result = table::read(fixture.file());

        // ASSERT
        assert!(result.is_err(), "a blank device carries no GPT");
    }
}
