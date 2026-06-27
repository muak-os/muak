//! In-place PE prefix patching.

use object::pe::ImageSectionHeader;

use crate::align;
use crate::error::{Result, YukiError};
use crate::pe::{self, PeMetadata};
use crate::section::{self, Layout};

pub(crate) fn patch(
    prefix: &mut [u8],
    metadata: &PeMetadata,
    layout: &Layout,
    new_section_count: u16,
) -> Result<()> {
    let total_sections = metadata
        .existing_section_count
        .saturating_add(new_section_count);
    header_fields(prefix, metadata, layout, total_sections)?;

    section_headers(prefix, metadata, layout)
}

fn header_fields(
    prefix: &mut [u8],
    metadata: &PeMetadata,
    layout: &Layout,
    total_sections: u16,
) -> Result<()> {
    range(
        prefix,
        pe::section_count_offset(metadata),
        &total_sections.to_le_bytes(),
        "section count",
    )?;

    let size_of_image = align::align_to(layout.max_virtual_end(), metadata.section_alignment);

    range(
        prefix,
        pe::size_of_image_offset(metadata),
        &size_of_image.to_le_bytes(),
        "size of image",
    )
}

fn section_headers(prefix: &mut [u8], metadata: &PeMetadata, layout: &Layout) -> Result<()> {
    for (i, header) in layout.headers.iter().enumerate() {
        let section_index = usize::from(metadata.existing_section_count).saturating_add(i);
        let offset = metadata.section_table_offset.saturating_add(
            section_index.saturating_mul(core::mem::size_of::<ImageSectionHeader>()),
        );
        let header_bytes = section::header_to_bytes(header);
        range(prefix, offset, &header_bytes, "section header")?;
    }

    Ok(())
}

fn range(prefix: &mut [u8], offset: usize, data: &[u8], field: &'static str) -> Result<()> {
    let Some(end) = offset.checked_add(data.len()) else {
        return Err(YukiError::InvalidPeStructure(format!(
            "{field} range overflow while patching PE prefix"
        )));
    };
    let Some(target) = prefix.get_mut(offset..end) else {
        return Err(YukiError::InvalidPeStructure(format!(
            "{field} lies outside the extracted PE prefix"
        )));
    };
    target.copy_from_slice(data);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_metadata() -> PeMetadata {
        PeMetadata {
            file_header_offset: 64,
            optional_header_offset: 84,
            section_table_offset: 324,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            existing_section_count: 1,
        }
    }

    #[test]
    fn patch_writes_section_count() {
        // ARRANGE
        let metadata = test_metadata();
        let size = usize::try_from(metadata.size_of_headers).unwrap_or(0);
        let mut prefix = vec![0_u8; size.max(1024)];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".test", 100).unwrap();

        // ACT
        patch(&mut prefix, &metadata, &layout, 1).unwrap();

        // ASSERT
        let section_count_offset = pe::section_count_offset(&metadata);
        let count = u16::from_le_bytes(
            prefix
                .get(section_count_offset..section_count_offset + 2)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        assert_eq!(count, metadata.existing_section_count + 1);
    }

    #[test]
    fn patch_writes_size_of_image() {
        // ARRANGE
        let metadata = test_metadata();
        let size = usize::try_from(metadata.size_of_headers).unwrap_or(0);
        let mut prefix = vec![0_u8; size.max(1024)];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".test", 100).unwrap();

        // ACT
        patch(&mut prefix, &metadata, &layout, 1).unwrap();

        // ASSERT
        let soi_offset = pe::size_of_image_offset(&metadata);
        let size_of_image = u32::from_le_bytes(
            prefix
                .get(soi_offset..soi_offset + 4)
                .unwrap()
                .try_into()
                .unwrap(),
        );
        assert!(size_of_image > 0);
    }

    #[test]
    fn patch_writes_section_headers() {
        // ARRANGE
        let metadata = test_metadata();
        let size = usize::try_from(metadata.size_of_headers).unwrap_or(0);
        let mut prefix = vec![0_u8; size.max(2048)];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".cmdline", 10).unwrap();
        layout.finalize_section(".linux", 200).unwrap();

        // ACT
        patch(&mut prefix, &metadata, &layout, 2).unwrap();

        // ASSERT
        let hdr_size = core::mem::size_of::<ImageSectionHeader>();
        let first_new =
            metadata.section_table_offset + usize::from(metadata.existing_section_count) * hdr_size;
        assert_eq!(prefix.get(first_new..first_new + 8).unwrap(), b".cmdline");
        let second_new = first_new + hdr_size;
        assert_eq!(prefix.get(second_new..second_new + 6).unwrap(), b".linux");
    }

    #[test]
    fn patch_rejects_out_of_bounds_write() {
        // ARRANGE
        let metadata = test_metadata();
        let mut prefix = vec![0_u8; 8];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".test", 100).unwrap();

        // ACT
        let result = patch(&mut prefix, &metadata, &layout, 1);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("outside the extracted PE prefix")
        ));
    }
}
