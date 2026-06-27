//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.

#![warn(missing_docs)]

mod align;
#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
mod pe;
pub mod section;
mod stream;

use std::io::{Read, Write};

use error::{Result, YukiError};
use ring::digest;

/// A readable UKI component with an exact byte length.
pub struct SizedPart<'a> {
    /// Exact component length in bytes.
    pub len: u64,
    /// Readable stream for the component bytes.
    pub reader: &'a mut dyn Read,
}

/// Component data required to build a Unified Kernel Image.
pub struct BuildInput<'a> {
    /// EFI stub PE binary to embed components into.
    pub stub: SizedPart<'a>,
    /// Kernel image.
    pub kernel: SizedPart<'a>,
    /// Initial RAM filesystem image.
    pub initramfs: SizedPart<'a>,
    /// Kernel command-line string.
    pub cmdline: SizedPart<'a>,
    /// Optional device-tree blob.
    pub dtb: Option<SizedPart<'a>>,
}

/// Builds a Unified Kernel Image by embedding components into an EFI stub.
///
/// Returns section metadata with file offsets.
///
/// # Errors
///
/// Returns an error if the stub is not a valid PE image, the section count
/// exceeds the PE limit, or writing the output fails.
pub fn build<W: Write>(mut input: BuildInput<'_>, writer: &mut W) -> Result<Vec<section::Section>> {
    let (metadata, mut stub_prefix) = pe::extract_metadata(input.stub.reader)?;
    let new_section_count = validate_build_params(&metadata, input.dtb.is_some())?;

    let mut layout = section::Layout::new(&metadata);
    let Ok(stub_file_off) = u32::try_from(input.stub.len) else {
        return Err(YukiError::InvalidPeStructure(
            "stub file offset overflow".to_owned(),
        ));
    };
    layout.current_file_offset = layout.current_file_offset.max(stub_file_off);
    let gap_start = finalize_sections(&mut layout, &input)?;

    stream::patch_prefix(&mut stub_prefix, &metadata, &layout, new_section_count)?;
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

fn validate_build_params(metadata: &pe::PeMetadata, has_dtb: bool) -> Result<u16> {
    let new_count = 3_u16.saturating_add(u16::from(has_dtb));
    if usize::from(metadata.existing_section_count).saturating_add(usize::from(new_count))
        > usize::from(u16::MAX)
    {
        return Err(YukiError::TooManySections);
    }
    pe::validate_section_header_capacity(metadata, usize::from(new_count))?;

    Ok(new_count)
}

fn finalize_sections(layout: &mut section::Layout, input: &BuildInput<'_>) -> Result<u64> {
    layout.finalize_section(
        ".cmdline",
        section::validate_size(input.cmdline.len, ".cmdline")?,
    )?;
    if let Some(dtb) = input.dtb.as_ref() {
        layout.finalize_section(".dtb", section::validate_size(dtb.len, ".dtb")?)?;
    }
    layout.finalize_section(
        ".linux",
        section::validate_size(input.kernel.len, ".linux")?,
    )?;
    layout.finalize_section(
        ".initrd",
        section::validate_size(input.initramfs.len, ".initrd")?,
    )?;

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
    metadata: &pe::PeMetadata,
    gap_start: u64,
    sections: &mut [section::Section],
) -> Result<()> {
    let mut iter = sections.iter_mut();

    stream::copy_stub(&mut input.stub, writer, stub_prefix)?;
    stream::write_gap(writer, gap_start.saturating_sub(input.stub.len))?;

    write_section(
        &mut input.cmdline,
        writer,
        metadata.file_alignment,
        ".cmdline",
        &mut iter,
    )?;

    if let Some(ref mut dtb) = input.dtb {
        write_section(dtb, writer, metadata.file_alignment, ".dtb", &mut iter)?;
    }

    write_section(
        &mut input.kernel,
        writer,
        metadata.file_alignment,
        ".linux",
        &mut iter,
    )?;

    write_section(
        &mut input.initramfs,
        writer,
        metadata.file_alignment,
        ".initrd",
        &mut iter,
    )?;

    Ok(())
}

fn write_section<'a, W: Write>(
    part: &mut SizedPart<'_>,
    writer: &mut W,
    file_alignment: u32,
    name: &'static str,
    sections: &mut impl Iterator<Item = &'a mut section::Section>,
) -> Result<()> {
    let mut ctx = digest::Context::new(&digest::SHA256);
    stream::write_part(part, writer, file_alignment, name, Some(&mut ctx))?;
    let Some(section) = sections.next() else {
        return Err(YukiError::InvalidPeStructure(
            "section count mismatch during assembly".to_owned(),
        ));
    };
    section.checksum = finish_ctx(ctx);

    Ok(())
}

fn finish_ctx(ctx: digest::Context) -> [u8; 32] {
    let digest = ctx.finish();
    let mut checksum = [0; 32];
    checksum.copy_from_slice(digest.as_ref());

    checksum
}
