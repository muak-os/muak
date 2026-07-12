//! UKI layout computation.

use std::io::Read;

use crate::error::{Result, YukiError};
use crate::pe::header;
use crate::pe::parse;
use crate::pe::section;

/// Computed byte offsets for each UKI component within the output PE image.
#[derive(Debug, Clone)]
pub struct Layout {
    /// File offset where the stub begins (always 0).
    pub stub_offset: u64,
    /// File offset of the `.cmdline` section.
    pub cmdline_offset: u64,
    /// File offset of the `.linux` section.
    pub kernel_offset: u64,
    /// File offset of the `.initrd` section.
    pub initramfs_offset: u64,
    /// Total size of the output UKI image in bytes.
    pub total_size: u64,
}

/// Opaque state produced by [`compute`] and consumed by [`crate::builder::Builder`].
#[derive(Debug)]
pub struct BuildState {
    pub(crate) layout: Layout,
    pub(crate) stub_prefix: Vec<u8>,
    pub(crate) stub_size: u64,
    pub(crate) table: section::Table,
    pub(crate) has_dtb: bool,
    pub(crate) file_alignment: u32,
}

/// Computes the UKI layout from a stub reader and component sizes.
///
/// # Errors
///
/// Returns an error when the stub is not a valid PE image, component lengths
/// overflow PE limits, or the section header table lacks capacity.
pub fn compute(
    stub: &mut dyn Read,
    stub_size: u64,
    cmdline_size: u64,
    kernel_size: u64,
    initramfs_size: u64,
    dtb_size: Option<u64>,
) -> Result<(Layout, BuildState)> {
    let (metadata, mut prefix_bytes) = parse::extract_metadata(stub)?;
    let has_dtb = dtb_size.is_some();

    let sizes = [
        (".cmdline", Some(cmdline_size)),
        (".dtb", dtb_size),
        (".linux", Some(kernel_size)),
        (".initrd", Some(initramfs_size)),
    ];

    let (table, _gap_start) = section::build_table(&metadata, stub_size, has_dtb, &sizes)?;

    header::patch(
        &mut prefix_bytes,
        &metadata,
        &table,
        section::count(has_dtb),
    )?;

    let layout = extract_layout(&table)?;
    let file_alignment = table.file_alignment;

    let state = BuildState {
        layout: layout.clone(),
        stub_prefix: prefix_bytes,
        stub_size,
        table,
        has_dtb,
        file_alignment,
    };

    Ok((layout, state))
}

fn offset_to_u64(file_offset: usize, name: &str) -> Result<u64> {
    u64::try_from(file_offset)
        .map_err(|_e| YukiError::InvalidPeStructure(format!("{name} section offset overflow")))
}

fn extract_layout(section_table: &section::Table) -> Result<Layout> {
    let mut layout = Layout {
        stub_offset: 0,
        cmdline_offset: 0,
        kernel_offset: 0,
        initramfs_offset: 0,
        total_size: u64::from(section_table.current_file_offset),
    };

    for sec in &section_table.sections {
        match sec.name {
            ".cmdline" => layout.cmdline_offset = offset_to_u64(sec.file_offset, "cmdline")?,
            ".linux" => layout.kernel_offset = offset_to_u64(sec.file_offset, "kernel")?,
            ".initrd" => layout.initramfs_offset = offset_to_u64(sec.file_offset, "initrd")?,
            _ => {}
        }
    }

    Ok(layout)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        if let Some(dst) = buf.get_mut(offset..offset.saturating_add(4)) {
            dst.copy_from_slice(&value.to_le_bytes());
        }
    }

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        if let Some(dst) = buf.get_mut(offset..offset.saturating_add(2)) {
            dst.copy_from_slice(&value.to_le_bytes());
        }
    }

    fn minimal_stub() -> Vec<u8> {
        let file_alignment = 512_usize;
        let section_header_size = 40_usize;
        let extra_slots = 4_usize;
        let num_sections = 1_u16;
        let section_table_size = section_header_size
            .saturating_mul(usize::from(num_sections).saturating_add(extra_slots));
        let headers_raw = 64_usize
            .saturating_add(4)
            .saturating_add(20)
            .saturating_add(240)
            .saturating_add(section_table_size);
        let headers_aligned = headers_raw.next_multiple_of(file_alignment);
        let total_size = headers_aligned.saturating_add(file_alignment);

        let mut stub = vec![0_u8; total_size];
        stub.get_mut(0..2).unwrap().copy_from_slice(b"MZ");
        write_u32(&mut stub, 0x3C, 64);
        stub.get_mut(64..68).unwrap().copy_from_slice(b"PE\0\0");
        write_u16(&mut stub, 68, 0x8664);
        write_u16(&mut stub, 70, num_sections);
        write_u16(&mut stub, 84, 240);
        write_u16(&mut stub, 86, 0x0222);
        let opt_start = 88;
        write_u16(&mut stub, opt_start, 0x020B);
        write_u32(&mut stub, opt_start.saturating_add(32), 4096);
        write_u32(&mut stub, opt_start.saturating_add(36), 512);
        write_u32(
            &mut stub,
            opt_start.saturating_add(60),
            u32::try_from(headers_aligned).unwrap(),
        );
        write_u16(&mut stub, opt_start.saturating_add(68), 10);
        let section_start = opt_start.saturating_add(240);
        stub.get_mut(section_start..section_start.saturating_add(5))
            .unwrap()
            .copy_from_slice(b".text");
        write_u32(&mut stub, section_start.saturating_add(8), 512);
        write_u32(&mut stub, section_start.saturating_add(12), 4096);
        write_u32(&mut stub, section_start.saturating_add(16), 512);
        write_u32(
            &mut stub,
            section_start.saturating_add(20),
            u32::try_from(headers_aligned).unwrap(),
        );
        write_u32(&mut stub, section_start.saturating_add(36), 0x6000_0020);

        stub
    }

    #[test]
    fn compute_returns_valid_layout() {
        // ARRANGE
        let stub_bytes = minimal_stub();
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();

        // ACT
        let (layout, _state) = compute(
            &mut Cursor::new(&stub_bytes),
            stub_size,
            10,
            1024,
            2048,
            None,
        )
        .unwrap();

        // ASSERT
        assert_eq!(layout.stub_offset, 0);
        assert!(layout.cmdline_offset >= stub_size);
        assert!(layout.kernel_offset > layout.cmdline_offset);
        assert!(layout.initramfs_offset > layout.kernel_offset);
        assert!(layout.total_size > layout.initramfs_offset);
    }

    #[test]
    fn compute_with_dtb() {
        // ARRANGE
        let stub_bytes = minimal_stub();
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();

        // ACT
        let (layout, state) = compute(
            &mut Cursor::new(&stub_bytes),
            stub_size,
            10,
            1024,
            2048,
            Some(512),
        )
        .unwrap();

        // ASSERT
        assert!(state.has_dtb);
        assert!(layout.total_size > layout.initramfs_offset);
    }

    #[test]
    fn compute_rejects_invalid_stub() {
        // ARRANGE
        let stub_bytes = b"not a PE file";

        // ACT
        let result = compute(&mut Cursor::new(stub_bytes), 13, 10, 10, 10, None);

        // ASSERT
        result.unwrap_err();
    }
}
