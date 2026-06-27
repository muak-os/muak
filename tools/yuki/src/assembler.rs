//! UKI image assembly orchestration.

use std::io::Write;

use ring::digest;

use crate::align;
use crate::error::{Result, YukiError};
use crate::pe::{self, PeMetadata};
use crate::prefix;
use crate::section::{self, Layout};
use crate::stream;
use crate::{BuildInput, SizedPart};

const SECTION_ORDER: &[&str] = &[".cmdline", ".dtb", ".linux", ".initrd"];

pub(crate) fn assemble<W: Write>(
    mut input: BuildInput<'_>,
    writer: &mut W,
) -> Result<Vec<section::Section>> {
    let (metadata, mut stub_prefix) = pe::extract_metadata(input.stub.reader)?;
    let new_section_count = validate_build_params(&metadata, input.dtb.is_some())?;

    let mut layout = Layout::new(&metadata);
    let Ok(stub_file_off) = u32::try_from(input.stub.len) else {
        return Err(YukiError::InvalidPeStructure(
            "stub file offset overflow".to_owned(),
        ));
    };
    layout.current_file_offset = layout.current_file_offset.max(stub_file_off);
    let gap_start = finalize_sections(&mut layout, &input)?;

    prefix::patch(&mut stub_prefix, &metadata, &layout, new_section_count)?;
    assemble_image(
        writer,
        &mut input,
        &stub_prefix,
        &metadata,
        gap_start,
        &mut layout.sections,
    )?;

    Ok(layout.sections)
}

fn validate_build_params(metadata: &PeMetadata, has_dtb: bool) -> Result<u16> {
    let new_count = 3_u16.saturating_add(u16::from(has_dtb));
    if usize::from(metadata.existing_section_count).saturating_add(usize::from(new_count))
        > usize::from(u16::MAX)
    {
        return Err(YukiError::TooManySections);
    }
    pe::validate_section_header_capacity(metadata, usize::from(new_count))?;

    Ok(new_count)
}

fn section_size(input: &BuildInput<'_>, name: &str) -> Option<u64> {
    match name {
        ".cmdline" => Some(input.cmdline.len),
        ".dtb" => input.dtb.as_ref().map(|dtb| dtb.len),
        ".linux" => Some(input.kernel.len),
        ".initrd" => Some(input.initramfs.len),
        _ => None,
    }
}

fn finalize_sections(layout: &mut Layout, input: &BuildInput<'_>) -> Result<u64> {
    for &name in SECTION_ORDER {
        let Some(len) = section_size(input, name) else {
            continue;
        };
        layout.finalize_section(name, section::validate_size(len, name)?)?;
    }

    let Some(first) = layout.sections.first() else {
        return Err(YukiError::InvalidPeStructure(
            "missing generated sections".to_owned(),
        ));
    };
    let Ok(gap_start) = u64::try_from(first.file_offset) else {
        return Err(YukiError::InvalidPeStructure(
            "first section offset overflow".to_owned(),
        ));
    };

    Ok(gap_start)
}

fn assemble_image<W: Write>(
    writer: &mut W,
    input: &mut BuildInput<'_>,
    stub_prefix: &[u8],
    metadata: &PeMetadata,
    gap_start: u64,
    sections: &mut [section::Section],
) -> Result<()> {
    let mut iter = sections.iter_mut();

    copy_stub(&mut input.stub, writer, stub_prefix)?;
    stream::write_gap(writer, gap_start.saturating_sub(input.stub.len))?;

    for &name in SECTION_ORDER {
        let part: Option<&mut SizedPart<'_>> = match name {
            ".cmdline" => Some(&mut input.cmdline),
            ".dtb" => input.dtb.as_mut(),
            ".linux" => Some(&mut input.kernel),
            ".initrd" => Some(&mut input.initramfs),
            _ => None,
        };
        let Some(part) = part else {
            continue;
        };
        write_section(part, writer, metadata.file_alignment, name, &mut iter)?;
    }

    Ok(())
}

fn copy_stub<W: Write>(stub: &mut SizedPart<'_>, writer: &mut W, prefix: &[u8]) -> Result<()> {
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

    stream::copy_exact(stub.reader, writer, remaining, "stub", &mut |_| {})
}

fn write_section<'a, W: Write>(
    part: &mut SizedPart<'_>,
    writer: &mut W,
    file_alignment: u32,
    name: &'static str,
    sections: &mut impl Iterator<Item = &'a mut section::Section>,
) -> Result<()> {
    let mut ctx = digest::Context::new(&digest::SHA256);
    stream::copy_exact(part.reader, writer, part.len, name, &mut |chunk| {
        ctx.update(chunk);
    })?;

    write_zero_padding(writer, file_alignment, align::u64_to_usize(part.len)?)?;

    let Some(section) = sections.next() else {
        return Err(YukiError::InvalidPeStructure(
            "section count mismatch during assembly".to_owned(),
        ));
    };
    let digest = ctx.finish();
    section.checksum.copy_from_slice(digest.as_ref());

    Ok(())
}

fn write_zero_padding<W: Write>(
    writer: &mut W,
    file_alignment: u32,
    actual_size: usize,
) -> Result<()> {
    let actual_u32 = align::usize_to_u32(actual_size)?;
    let aligned = align::align_to(actual_u32, file_alignment);

    stream::write_gap(writer, u64::from(aligned.saturating_sub(actual_u32)))
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;
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
    fn write_section_propagates_reader_error() {
        // ARRANGE
        let mut writer = ErrorWriter;

        // ACT
        let result = write_section(
            &mut SizedPart {
                len: 4,
                reader: &mut &b"data"[..],
            },
            &mut writer,
            test_metadata().file_alignment,
            ".test",
            &mut [].iter_mut(),
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_section_writes_data_and_padding() {
        // ARRANGE
        let data = b"hello section data";
        let mut reader: &[u8] = data;
        let mut output = Vec::new();
        let section = section::Section {
            name: ".test",
            file_offset: 0,
            size: data.len(),
            checksum: [0; 32],
        };
        let mut sections = [section.clone()];

        // ACT
        write_section(
            &mut SizedPart {
                len: u64::try_from(data.len()).unwrap_or(0),
                reader: &mut reader,
            },
            &mut output,
            test_metadata().file_alignment,
            ".test",
            &mut sections.iter_mut(),
        )
        .unwrap();

        // ASSERT
        assert_eq!(output.get(..data.len()).unwrap_or_default(), data);
        assert_eq!(output.len(), 512);
        assert_ne!(sections[0].checksum, [0_u8; 32]);
    }

    #[test]
    fn write_section_rejects_short_stream() {
        // ARRANGE
        let data = b"short data";
        let mut reader: &[u8] = data;
        let mut output = Vec::new();

        // ACT
        let result = write_section(
            &mut SizedPart {
                len: 32,
                reader: &mut reader,
            },
            &mut output,
            test_metadata().file_alignment,
            ".test",
            &mut [].iter_mut(),
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("ended early")
        ));
    }

    #[test]
    fn write_zero_padding_propagates_writer_error() {
        // ARRANGE & ACT
        let result = write_zero_padding(&mut ErrorWriter, test_metadata().file_alignment, 10);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }
}
