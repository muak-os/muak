//! Streaming PE image assembly.

use std::io::Write;

use object::pe::ImageSectionHeader;

use crate::binary;
use crate::error::{Result, YukiError};
use crate::pe::{self, PeMetadata};
use crate::section::{self, Layout, Section};

struct ImageChunk<'a> {
    offset: usize,
    data: &'a [u8],
}

pub(crate) fn write<W: Write>(
    writer: &mut W,
    stub: &[u8],
    metadata: &PeMetadata,
    layout: &Layout,
    sections: &[Section],
    sections_data: &[&[u8]],
    new_section_count: u16,
) -> Result<()> {
    let section_count_bytes = new_section_count.to_le_bytes();
    let size_of_image_bytes =
        binary::align_to(layout.max_virtual_end, metadata.section_alignment).to_le_bytes();
    let section_header_bytes: Vec<[u8; core::mem::size_of::<ImageSectionHeader>()]> = layout
        .headers
        .iter()
        .map(section::section_header_to_bytes)
        .collect();

    let mut chunks = vec![ImageChunk {
        offset: pe::section_count_offset(metadata),
        data: &section_count_bytes,
    }];
    chunks.push(ImageChunk {
        offset: pe::size_of_image_offset(metadata),
        data: &size_of_image_bytes,
    });

    for (index, header_bytes) in section_header_bytes.iter().enumerate() {
        let section_index = usize::from(metadata.current_section_count).saturating_add(index);
        let offset = metadata.section_table_offset.saturating_add(
            section_index.saturating_mul(core::mem::size_of::<ImageSectionHeader>()),
        );
        chunks.push(ImageChunk {
            offset,
            data: header_bytes,
        });
    }

    for (section, data) in sections.iter().zip(sections_data.iter()) {
        chunks.push(ImageChunk {
            offset: section.file_offset,
            data,
        });
    }

    chunks.sort_unstable_by_key(|chunk| chunk.offset);

    write_image(writer, stub, layout.total_file_size, &chunks)
}

fn write_image<W: Write>(
    writer: &mut W,
    stub: &[u8],
    total_file_size: usize,
    chunks: &[ImageChunk<'_>],
) -> Result<()> {
    let mut cursor = 0_usize;

    for chunk in chunks {
        let chunk_data = chunk.data;
        if chunk.offset < cursor {
            return Err(YukiError::InvalidPeStructure(format!(
                "overlapping image chunk: {}<{cursor}",
                chunk.offset
            )));
        }
        let chunk_end = chunk.offset.saturating_add(chunk_data.len());
        if chunk_end > total_file_size {
            return Err(YukiError::InvalidPeStructure(format!(
                "image chunk exceeds output size: {chunk_end}>{total_file_size}"
            )));
        }

        write_base_range(writer, stub, cursor, chunk.offset)?;
        writer.write_all(chunk_data)?;
        cursor = chunk_end;
    }

    write_base_range(writer, stub, cursor, total_file_size)
}

fn write_base_range<W: Write>(writer: &mut W, stub: &[u8], start: usize, end: usize) -> Result<()> {
    if start >= end {
        return Ok(());
    }

    let stub_avail = stub.len().saturating_sub(start);
    let stub_bytes = stub_avail.min(end.saturating_sub(start));
    if stub_bytes > 0 {
        let chunk_end = start.saturating_add(stub_bytes);
        writer.write_all(stub.get(start..chunk_end).unwrap_or_default())?;
    }

    let zeros = end.saturating_sub(start.saturating_add(stub_bytes));
    if zeros > 0 {
        const ZERO_BUF: [u8; 8192] = [0; 8192];
        let mut remaining = zeros;

        while remaining > 0 {
            let chunk_len = remaining.min(ZERO_BUF.len());
            writer.write_all(ZERO_BUF.get(..chunk_len).unwrap_or_default())?;
            remaining = remaining.saturating_sub(chunk_len);
        }
    }

    Ok(())
}

#[cfg(test)]
#[expect(clippy::excessive_nesting, reason = "test code")]
mod tests {
    use super::*;

    struct ErrorWriter;

    impl Write for ErrorWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_image_rejects_overlapping_chunks() {
        // ARRANGE
        let mut output = Vec::new();
        let chunks = [
            ImageChunk {
                offset: 2,
                data: b"abc",
            },
            ImageChunk {
                offset: 4,
                data: b"d",
            },
        ];

        // ACT
        let result = write_image(&mut output, b"stub", 8, &chunks);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message)) if message.contains("overlapping image chunk")
        ));
    }

    #[test]
    fn write_base_range_emits_stub_then_zero_padding() {
        // ARRANGE
        let mut output = Vec::new();

        // ACT
        let result = write_base_range(&mut output, b"abcd", 1, 6);
        assert!(result.is_ok(), "base range should write");

        // ASSERT
        assert_eq!(output, b"bcd\0\0");
    }

    #[test]
    fn write_image_rejects_chunk_past_output_size() {
        // ARRANGE
        let mut output = Vec::new();
        let chunks = [ImageChunk {
            offset: 4,
            data: b"toolong",
        }];

        // ACT
        let result = write_image(&mut output, b"stub", 8, &chunks);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("image chunk exceeds output size")
        ));
    }

    #[test]
    fn write_image_propagates_writer_error_on_stub_bytes() {
        // ARRANGE
        let chunks = [ImageChunk {
            offset: 2,
            data: b"xx",
        }];

        // ACT
        let result = write_image(&mut ErrorWriter, b"stub", 8, &chunks);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_image_propagates_writer_error_on_chunk_bytes() {
        // ARRANGE
        let chunks = [ImageChunk {
            offset: 0,
            data: &[1, 2, 3],
        }];

        // ACT
        let result = write_image(&mut ErrorWriter, b"", 3, &chunks);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_base_range_propagates_padding_error() {
        // ARRANGE
        let result = write_base_range(&mut ErrorWriter, b"ab", 0, 4);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_base_range_padding_fails_when_stub_empty() {
        // ARRANGE - stub is empty, so only zeros need to be written
        // ACT
        let result = write_base_range(&mut ErrorWriter, b"", 42, 100);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_image_padding_fails_with_limited_writer() {
        // ARRANGE
        struct PadWriter(usize);
        impl Write for PadWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if self.0 == 0 {
                    return Err(std::io::Error::other("pad writer full"));
                }
                let n = self.0.min(buf.len());
                self.0 -= n;
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let chunks = [ImageChunk {
            offset: 0,
            data: b"data",
        }];

        // Cover the flush implementation
        let _flush_result = PadWriter(4).flush();

        // ACT
        let result = write_image(&mut PadWriter(4), b"stub", 100, &chunks);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn fail_writer_flush_succeeds() {
        // ARRANGE
        let mut writer = ErrorWriter;

        // ACT
        let result = writer.flush();

        // ASSERT
        assert!(result.is_ok(), "flush should succeed");
    }

    #[test]
    fn image_chunk_can_borrow_owned_array() {
        // ARRANGE
        let data = [1, 2, 3];
        let chunk = ImageChunk {
            offset: 0,
            data: &data,
        };

        // ACT
        let bytes = chunk.data;

        // ASSERT
        assert_eq!(bytes, [1, 2, 3]);
    }

    #[test]
    fn image_chunk_borrows_slice_bytes() {
        // ARRANGE
        let chunk = ImageChunk {
            offset: 0,
            data: b"abc",
        };

        // ACT
        let bytes = chunk.data;

        // ASSERT
        assert_eq!(bytes, b"abc");
    }
}
