//! Compute the layout of a UKI image from its section table.

use uki::section::{CMDLINE, INITRD, KERNEL};

use crate::error::{Result, YukiError};
use crate::pe::section::Table;

/// Computed byte offsets for each UKI component within the output PE image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Size of the EFI stub in bytes.
    pub stub_size: u64,
    /// File offset of the `.cmdline` section.
    pub cmdline_offset: u64,
    /// File offset of the `.kernel` section.
    pub kernel_offset: u64,
    /// File offset of the `.initrd` section.
    pub initramfs_offset: u64,
    /// Total size of the output UKI image in bytes.
    pub total_size: u64,
}

/// Extracts the geometry view from a section table.
pub(crate) fn from_table(stub_size: u64, table: &Table) -> Result<Layout> {
    let mut cmdline_offset = 0_u64;
    let mut kernel_offset = 0_u64;
    let mut initramfs_offset = 0_u64;

    for sec in &table.sections {
        let offset = u64::try_from(sec.file_offset).map_err(|_source| {
            YukiError::InvalidPeStructure(format!("section '{}' offset overflow", sec.name))
        })?;
        match sec.name {
            CMDLINE => cmdline_offset = offset,
            KERNEL => kernel_offset = offset,
            INITRD => initramfs_offset = offset,
            _ => {}
        }
    }

    Ok(Layout {
        stub_size,
        cmdline_offset,
        kernel_offset,
        initramfs_offset,
        total_size: u64::from(table.current_file_offset),
    })
}

#[cfg(test)]
mod tests {
    use uki::metadata::Metadata;
    use uki::section::{CMDLINE, INITRD, KERNEL};

    use super::*;

    fn test_metadata() -> Metadata {
        Metadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            size_of_headers: 1024,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            existing_section_count: 1,
            num_data_directories: 16,
        }
    }

    #[test]
    fn from_table_extracts_section_offsets() {
        // ARRANGE
        let metadata = test_metadata();
        let mut table = Table::new(&metadata);
        table.finalize_section(CMDLINE, 10).unwrap();
        table.finalize_section(KERNEL, 100).unwrap();
        table.finalize_section(INITRD, 300).unwrap();

        // ACT
        let layout = from_table(512, &table).unwrap();

        // ASSERT
        assert_eq!(layout.stub_size, 512);
        assert_eq!(
            layout.cmdline_offset,
            u64::try_from(
                table
                    .sections
                    .iter()
                    .find(|sec| sec.name == CMDLINE)
                    .unwrap()
                    .file_offset
            )
            .unwrap()
        );
        assert_eq!(
            layout.kernel_offset,
            u64::try_from(
                table
                    .sections
                    .iter()
                    .find(|sec| sec.name == KERNEL)
                    .unwrap()
                    .file_offset
            )
            .unwrap()
        );
        assert_eq!(
            layout.initramfs_offset,
            u64::try_from(
                table
                    .sections
                    .iter()
                    .find(|sec| sec.name == INITRD)
                    .unwrap()
                    .file_offset
            )
            .unwrap()
        );
    }

    #[test]
    fn from_table_total_size_matches_last_aligned_section_end() {
        // ARRANGE
        let metadata = test_metadata();
        let mut table = Table::new(&metadata);
        table.finalize_section(CMDLINE, 10).unwrap();
        table.finalize_section(KERNEL, 100).unwrap();
        table.finalize_section(INITRD, 300).unwrap();

        // ACT
        let layout = from_table(512, &table).unwrap();

        // ASSERT
        assert_eq!(layout.total_size, u64::from(table.current_file_offset));
        let last = table.sections.last().unwrap();
        let aligned_size =
            uki::align::to(u32::try_from(last.size).unwrap(), metadata.file_alignment);
        let last_end = last
            .file_offset
            .saturating_add(usize::try_from(aligned_size).unwrap());
        assert_eq!(
            layout.total_size,
            u64::try_from(last_end).unwrap(),
            "total size should match the last aligned section end"
        );
    }
}
