#![cfg(test)]

//! A temporary device image shared by the partdev integration tests.

use std::fs::File;
use std::os::unix::fs::FileExt as _;

use parttable::gpt::partition::{LINUX_FS_GUID, Partition};
use parttable::gpt::table::Table;
use parttable::mbr::PartitionEntry;
use tempfile::NamedTempFile;

/// Size in bytes of one GPT sector used by the fixtures.
pub const SECTOR_BYTES: u64 = 512;

/// A zeroed temporary device image of a fixed number of sectors.
pub struct Fixture {
    file: NamedTempFile,
    sectors: u64,
}

impl Fixture {
    /// Creates a zeroed image of `sectors` sectors.
    ///
    /// # Panics
    ///
    /// Panics when the temporary image cannot be created or resized.
    #[must_use]
    pub fn new(sectors: u64) -> Self {
        let file = NamedTempFile::new().expect("fixture file");
        file.as_file()
            .set_len(sectors.saturating_mul(SECTOR_BYTES))
            .expect("resize fixture");

        Self { file, sectors }
    }

    /// Returns the file behind the image.
    #[must_use]
    pub fn file(&self) -> &File {
        self.file.as_file()
    }

    /// Returns the image path as a UTF-8 string.
    ///
    /// # Panics
    ///
    /// Panics when the temporary path is not valid UTF-8.
    #[must_use]
    pub fn path_str(&self) -> &str {
        self.file.path().to_str().expect("utf-8 fixture path")
    }

    /// Returns the number of sectors in the image.
    #[must_use]
    pub fn sectors(&self) -> u64 {
        self.sectors
    }

    /// Overwrites the whole image with `byte`.
    ///
    /// # Panics
    ///
    /// Panics when the image cannot be written.
    pub fn fill(&self, byte: u8) {
        let length = usize::try_from(self.sectors.saturating_mul(SECTOR_BYTES)).unwrap_or(0);
        let data = vec![byte; length];
        self.file
            .as_file()
            .write_all_at(&data, 0)
            .expect("fill fixture");
    }

    /// Writes a GPT carrying `names` as partitions.
    ///
    /// # Panics
    ///
    /// Panics when the table cannot be built or committed.
    pub fn write_gpt(&self, names: &[&str]) {
        let table = gpt_table(self.sectors, names);
        partdev::table::commit(self.file.as_file(), &table, self.sectors).expect("commit GPT");
    }

    /// Writes a single-entry MBR.
    ///
    /// # Panics
    ///
    /// Panics when the MBR cannot be written.
    pub fn write_mbr(&self) {
        let entry = PartitionEntry {
            bootable: false,
            partition_type: 0x83,
            starting_lba: 1,
            size_lba: 1,
        };
        let mut file = self.file.as_file();
        parttable::mbr::write(&mut file, &[Some(entry), None, None, None]).expect("write MBR");
    }

    /// Reads the whole image.
    ///
    /// # Panics
    ///
    /// Panics when the image cannot be read.
    #[must_use]
    pub fn read_all(&self) -> Vec<u8> {
        let length = usize::try_from(self.sectors.saturating_mul(SECTOR_BYTES)).unwrap_or(0);
        let mut data = vec![0_u8; length];
        self.file
            .as_file()
            .read_exact_at(&mut data, 0)
            .expect("read fixture");

        data
    }
}

/// Builds a GPT with one partition per name in `names`.
///
/// # Panics
///
/// Panics when the table cannot be created for `sectors`.
#[must_use]
pub fn gpt_table(sectors: u64, names: &[&str]) -> Table {
    let mut table = Table::create(sectors, SECTOR_BYTES, [0xCD; 16]).expect("create table");

    for (index, name) in names.iter().enumerate() {
        let offset = u64::try_from(index).unwrap_or(0);
        let starting_lba = 2048_u64.saturating_add(offset.saturating_mul(4096));
        table.set_partition(
            u32::try_from(offset.saturating_add(1)).unwrap_or(0),
            Partition {
                type_guid: LINUX_FS_GUID,
                unique_guid: [u8::try_from(offset).unwrap_or(0); 16],
                starting_lba,
                ending_lba: starting_lba.saturating_add(4095),
                attributes: 0,
                name: (*name).to_owned(),
            },
        );
    }

    table
}

/// Reads `length` bytes at `offset` without moving the file cursor.
///
/// # Panics
///
/// Panics when the range cannot be read.
#[must_use]
pub fn read_at(file: &File, offset: u64, length: usize) -> Vec<u8> {
    let mut data = vec![0_u8; length];
    file.read_exact_at(&mut data, offset).expect("read range");

    data
}
