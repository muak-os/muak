//! Streaming PE image assembly.

use std::io::{Read, Write};

use object::pe::ImageSectionHeader;

use crate::SizedPart;
use crate::align;
use crate::error::{Result, YukiError};
use crate::pe::{self, PeMetadata};
use crate::section::{self, Layout};

const ZERO_BUF: [u8; 8192] = [0; 8192];

pub(crate) fn copy_stub<W: Write>(
    stub: &mut SizedPart<'_>,
    writer: &mut W,
    prefix: &[u8],
) -> Result<()> {
    writer.write_all(prefix)?;
    let Ok(prefix_len) = u64::try_from(prefix.len()) else {
        return Err(YukiError::InvalidPeStructure(
            "stub prefix length overflow".to_owned(),
        ));
    };
    let Some(remaining) = stub.len.checked_sub(prefix_len) else {
        return Err(YukiError::InvalidPeStructure(
            "stub length smaller than copied prefix".to_owned(),
        ));
    };
    copy_exact(stub.reader, writer, remaining, "stub")?;

    Ok(())
}

pub(crate) fn write_gap<W: Write>(writer: &mut W, size: u64) -> Result<()> {
    let mut remaining = size;
    while remaining > 0 {
        let Ok(chunk) = usize::try_from(remaining.min(8192_u64)) else {
            return Err(YukiError::InvalidPeStructure(
                "gap range overflow".to_owned(),
            ));
        };
        let Some(pad) = ZERO_BUF.get(..chunk) else {
            return Err(YukiError::InvalidPeStructure(
                "zero buffer range invalid".to_owned(),
            ));
        };
        writer.write_all(pad)?;
        let Ok(chunk_u64) = u64::try_from(chunk) else {
            return Err(YukiError::InvalidPeStructure(
                "gap count overflow".to_owned(),
            ));
        };
        remaining = remaining.saturating_sub(chunk_u64);
    }

    Ok(())
}

pub(crate) fn write_part<W: Write>(
    part: &mut SizedPart<'_>,
    writer: &mut W,
    file_alignment: u32,
    name: &'static str,
) -> Result<()> {
    copy_exact(part.reader, writer, part.len, name)?;

    write_zero_padding(writer, file_alignment, align::u64_to_usize(part.len)?)
}

fn copy_exact<W: Write>(
    reader: &mut dyn Read,
    writer: &mut W,
    len: u64,
    name: &'static str,
) -> Result<()> {
    let mut buffer = [0_u8; 16384];
    let mut limited = reader.take(len);
    let mut copied = 0_u64;
    loop {
        let n = limited.read(&mut buffer).map_err(YukiError::Io)?;
        if n == 0 {
            break;
        }
        let Some(chunk) = buffer.get(..n) else {
            return Err(YukiError::Io(std::io::Error::other("buffer range invalid")));
        };
        writer.write_all(chunk)?;
        let Ok(amount) = u64::try_from(n) else {
            return Err(YukiError::InvalidPeStructure(
                "read count overflow".to_owned(),
            ));
        };
        copied = copied.saturating_add(amount);
    }

    if copied != len {
        return Err(YukiError::InvalidPeStructure(format!(
            "section '{name}' ended early: expected {len} bytes, copied {copied}"
        )));
    }

    Ok(())
}

pub(crate) fn write_zero_padding<W: Write>(
    writer: &mut W,
    file_alignment: u32,
    actual_size: usize,
) -> Result<()> {
    let actual_u32 = align::usize_to_u32(actual_size)?;
    let aligned = align::align_to(actual_u32, file_alignment);

    write_gap(writer, u64::from(aligned.saturating_sub(actual_u32)))
}

pub(crate) fn patch_prefix(
    prefix: &mut [u8],
    metadata: &PeMetadata,
    layout: &Layout,
    new_section_count: u16,
) -> Result<()> {
    let total_sections = metadata
        .existing_section_count
        .saturating_add(new_section_count);
    patch_header_fields(prefix, metadata, layout, total_sections)?;

    append_section_headers(prefix, metadata, layout)
}

fn patch_header_fields(
    prefix: &mut [u8],
    metadata: &PeMetadata,
    layout: &Layout,
    total_sections: u16,
) -> Result<()> {
    write_prefix_range(
        prefix,
        pe::section_count_offset(metadata),
        &total_sections.to_le_bytes(),
        "section count",
    )?;

    let size_of_image = align::align_to(layout.max_virtual_end(), metadata.section_alignment);

    write_prefix_range(
        prefix,
        pe::size_of_image_offset(metadata),
        &size_of_image.to_le_bytes(),
        "size of image",
    )
}

fn append_section_headers(prefix: &mut [u8], metadata: &PeMetadata, layout: &Layout) -> Result<()> {
    for (i, header) in layout.headers.iter().enumerate() {
        let section_index = usize::from(metadata.existing_section_count).saturating_add(i);
        let offset = metadata.section_table_offset.saturating_add(
            section_index.saturating_mul(core::mem::size_of::<ImageSectionHeader>()),
        );
        let header_bytes = section::header_to_bytes(header);
        write_prefix_range(prefix, offset, &header_bytes, "section header")?;
    }

    Ok(())
}

fn write_prefix_range(
    prefix: &mut [u8],
    offset: usize,
    data: &[u8],
    field: &'static str,
) -> Result<()> {
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
    use crate::SizedPart;
    use crate::pe::PeMetadata;

    struct ErrorWriter;

    impl Write for ErrorWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

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
    fn copy_stub_propagates_reader_error() {
        // ARRANGE
        let mut writer = ErrorWriter;

        // ACT
        let result = copy_stub(
            &mut SizedPart {
                len: 0,
                reader: &mut &b""[..],
            },
            &mut writer,
            b"prefix",
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn copy_stub_writes_prefix_and_data() {
        // ARRANGE
        let prefix = b"PREFIX";
        let data = b"hello world stub content";
        let mut full_data = prefix.to_vec();
        full_data.extend_from_slice(data);
        let mut full_slice: &[u8] = &full_data;
        // simulate extract_metadata which consumes prefix bytes
        let mut consumed = [0_u8; 6];
        full_slice.read_exact(&mut consumed).unwrap();
        let mut output = Vec::new();

        // ACT
        copy_stub(
            &mut SizedPart {
                len: u64::try_from(prefix.len() + data.len()).unwrap_or(0),
                reader: &mut full_slice,
            },
            &mut output,
            prefix,
        )
        .unwrap();

        // ASSERT
        assert_eq!(output, b"PREFIXhello world stub content");
    }

    #[test]
    fn copy_stub_empty_prefix() {
        // ARRANGE
        let data = b"stub data";
        let mut reader: &[u8] = data;
        let mut output = Vec::new();

        // ACT
        copy_stub(
            &mut SizedPart {
                len: u64::try_from(data.len()).unwrap_or(0),
                reader: &mut reader,
            },
            &mut output,
            b"",
        )
        .unwrap();

        // ASSERT
        assert_eq!(output, b"stub data");
    }

    #[test]
    fn copy_stub_empty_reader() {
        // ARRANGE
        let mut output = Vec::new();

        // ACT
        copy_stub(
            &mut SizedPart {
                len: 6,
                reader: &mut &b""[..],
            },
            &mut output,
            b"prefix",
        )
        .unwrap();

        // ASSERT
        assert_eq!(output, b"prefix");
    }

    #[test]
    fn copy_stub_rejects_short_stub() {
        // ARRANGE
        let mut output = Vec::new();

        // ACT
        let result = copy_stub(
            &mut SizedPart {
                len: 2,
                reader: &mut &b"ab"[..],
            },
            &mut output,
            b"prefix",
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("stub length smaller than copied prefix")
        ));
    }

    #[test]
    fn write_part_propagates_reader_error() {
        // ARRANGE
        let mut writer = ErrorWriter;

        // ACT
        let result = write_part(
            &mut SizedPart {
                len: 4,
                reader: &mut &b"data"[..],
            },
            &mut writer,
            test_metadata().file_alignment,
            ".test",
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_part_writes_data_and_padding() {
        // ARRANGE
        let data = b"hello section data";
        let mut reader: &[u8] = data;
        let mut output = Vec::new();

        // ACT
        write_part(
            &mut SizedPart {
                len: u64::try_from(data.len()).unwrap_or(0),
                reader: &mut reader,
            },
            &mut output,
            test_metadata().file_alignment,
            ".test",
        )
        .unwrap();

        // ASSERT
        assert_eq!(output.get(..data.len()).unwrap_or_default(), data);
        assert_eq!(output.len(), 512);
    }

    #[test]
    fn write_part_rejects_short_stream() {
        // ARRANGE
        let data = b"short data";
        let mut reader: &[u8] = data;
        let mut output = Vec::new();

        // ACT
        let result = write_part(
            &mut SizedPart {
                len: 32,
                reader: &mut reader,
            },
            &mut output,
            test_metadata().file_alignment,
            ".test",
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("ended early")
        ));
    }

    #[test]
    fn patch_prefix_writes_section_count() {
        // ARRANGE
        let metadata = test_metadata();
        let size = usize::try_from(metadata.size_of_headers).unwrap_or(0);
        let mut prefix = vec![0_u8; size.max(1024)];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".test", 100).unwrap();

        // ACT
        patch_prefix(&mut prefix, &metadata, &layout, 1).unwrap();

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
    fn patch_prefix_writes_size_of_image() {
        // ARRANGE
        let metadata = test_metadata();
        let size = usize::try_from(metadata.size_of_headers).unwrap_or(0);
        let mut prefix = vec![0_u8; size.max(1024)];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".test", 100).unwrap();

        // ACT
        patch_prefix(&mut prefix, &metadata, &layout, 1).unwrap();

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
    fn patch_prefix_writes_section_headers() {
        // ARRANGE
        let metadata = test_metadata();
        let size = usize::try_from(metadata.size_of_headers).unwrap_or(0);
        let mut prefix = vec![0_u8; size.max(2048)];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".cmdline", 10).unwrap();
        layout.finalize_section(".linux", 200).unwrap();

        // ACT
        patch_prefix(&mut prefix, &metadata, &layout, 2).unwrap();

        // ASSERT
        let hdr_size = core::mem::size_of::<ImageSectionHeader>();
        let first_new =
            metadata.section_table_offset + usize::from(metadata.existing_section_count) * hdr_size;
        assert_eq!(prefix.get(first_new..first_new + 8).unwrap(), b".cmdline");
        let second_new = first_new + hdr_size;
        assert_eq!(prefix.get(second_new..second_new + 6).unwrap(), b".linux");
    }

    #[test]
    fn write_zero_padding_propagates_writer_error() {
        // ARRANGE & ACT
        let result = write_zero_padding(&mut ErrorWriter, test_metadata().file_alignment, 10);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_gap_writes_zeros() {
        // ARRANGE
        let mut output = Vec::new();

        // ACT
        write_gap(&mut output, 32).unwrap();

        // ASSERT
        assert_eq!(output, vec![0_u8; 32]);
    }

    #[test]
    fn error_writer_flush_succeeds() {
        // ARRANGE
        let mut writer = ErrorWriter;

        // ACT & ASSERT
        writer.flush().unwrap();
    }

    #[test]
    fn patch_prefix_rejects_out_of_bounds_write() {
        // ARRANGE
        let metadata = test_metadata();
        let mut prefix = vec![0_u8; 8];
        let mut layout = Layout::new(&metadata);
        layout.finalize_section(".test", 100).unwrap();

        // ACT
        let result = patch_prefix(&mut prefix, &metadata, &layout, 1);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("outside the extracted PE prefix")
        ));
    }
}
