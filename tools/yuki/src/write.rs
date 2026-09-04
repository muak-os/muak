//! Single forward pass that emits a prepared UKI.

use std::io::{Read, Write};

use sha2::{Digest as _, Sha256};
use uki::align;
use uki::section::{CMDLINE, INITRD, KERNEL};

use crate::error::{Result, YukiError};
use crate::io;
use crate::pe::section::Section;
use crate::prepare::Manifest;

/// Forward-only reader plus its declared size.
pub struct Input<'a> {
    /// Reader for the component bytes.
    pub reader: &'a mut dyn Read,
    /// Declared size of the component in bytes.
    pub size: u64,
}

/// Emits a prepared UKI to `output` in a single forward pass.
///
/// The `stub` reader must be positioned exactly [`crate::probe::Probe::consumed`]
/// bytes into the stub file; the write pass continues on the same stream.
///
/// # Errors
///
/// Returns an error when an input size does not match the plan, a stream ends
/// early, or writing fails.
pub fn write<W: Write>(
    manifest: &Manifest,
    stub: &mut dyn Read,
    cmdline: Input<'_>,
    kernel: Input<'_>,
    initramfs: Input<'_>,
    output: &mut W,
) -> Result<Vec<Section>> {
    validate_inputs(manifest, &cmdline, &kernel, &initramfs)?;

    let layout = manifest.layout();
    let assembly = manifest.assembly();

    output.write_all(&assembly.patched_prefix)?;
    io::copy_exact(stub, output, assembly.stub_remainder, "stub", &mut |_| {})?;

    let cmdline_reader = cmdline.reader;
    let kernel_reader = kernel.reader;
    let initramfs_reader = initramfs.reader;

    let mut pos = layout.stub_size;
    let mut sections = Vec::with_capacity(assembly.sections.len());

    for planned in &assembly.sections {
        let file_offset = u64::try_from(planned.file_offset).map_err(|_source| {
            YukiError::InvalidPeStructure(format!("section '{}' offset overflow", planned.name))
        })?;
        if file_offset < pos {
            return Err(YukiError::InvalidPeStructure(format!(
                "section '{}' precedes current write position",
                planned.name
            )));
        }
        io::write_gap(output, file_offset.saturating_sub(pos))?;

        let size = u64::try_from(planned.size).map_err(|_source| {
            YukiError::InvalidPeStructure(format!("section '{}' size overflow", planned.name))
        })?;
        let reader: &mut dyn Read = match planned.name {
            CMDLINE => &mut *cmdline_reader,
            KERNEL => &mut *kernel_reader,
            INITRD => &mut *initramfs_reader,
            _ => {
                return Err(YukiError::InvalidPeStructure(format!(
                    "unknown section '{}'",
                    planned.name
                )));
            }
        };

        let mut ctx = Sha256::new();
        io::copy_exact(reader, output, size, planned.name, &mut |chunk| {
            ctx.update(chunk);
        })?;

        let size_u32 = u32::try_from(size).map_err(|_source| {
            YukiError::InvalidPeStructure(format!("section '{}' size exceeds u32", planned.name))
        })?;
        let aligned = align::to(size_u32, assembly.file_alignment);
        io::write_gap(output, u64::from(aligned.saturating_sub(size_u32)))?;
        pos = file_offset.saturating_add(u64::from(aligned));

        let mut section = planned.clone();
        section.checksum.copy_from_slice(ctx.finalize().as_ref());
        sections.push(section);
    }

    if pos != layout.total_size {
        return Err(YukiError::InvalidPeStructure(format!(
            "output length mismatch: wrote {pos} bytes, expected {}",
            layout.total_size
        )));
    }

    Ok(sections)
}

fn validate_inputs(
    manifest: &Manifest,
    cmdline: &Input<'_>,
    kernel: &Input<'_>,
    initramfs: &Input<'_>,
) -> Result<()> {
    for planned in &manifest.assembly().sections {
        let input_size = match planned.name {
            CMDLINE => cmdline.size,
            KERNEL => kernel.size,
            INITRD => initramfs.size,
            _ => 0,
        };
        let planned_size = u64::try_from(planned.size).map_err(|_source| {
            YukiError::InvalidPeStructure(format!("section '{}' size overflow", planned.name))
        })?;
        if planned_size != input_size {
            return Err(YukiError::InvalidPeStructure(format!(
                "section '{}' input size {input_size} does not match planned {planned_size}",
                planned.name
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use object::LittleEndian as LE;
    use object::pe as object_pe;
    use object::read::pe::PeFile64;

    use super::*;
    use crate::prepare;
    use crate::probe;

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        let end = offset.checked_add(4).unwrap();
        buf.get_mut(offset..end)
            .unwrap()
            .copy_from_slice(&value.to_le_bytes());
    }

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        let end = offset.checked_add(2).unwrap();
        buf.get_mut(offset..end)
            .unwrap()
            .copy_from_slice(&value.to_le_bytes());
    }

    fn minimal_stub() -> Vec<u8> {
        let opt_start = 88_usize;
        let section_start = opt_start.saturating_add(240);
        let headers_raw = section_start.saturating_add(5_usize.saturating_mul(40));
        let headers_aligned = headers_raw.next_multiple_of(512);
        let total_size = headers_aligned.saturating_add(512);

        let mut stub = vec![0_u8; total_size];
        stub.get_mut(0..2).unwrap().copy_from_slice(b"MZ");
        write_u32(&mut stub, 0x3C, 64);
        stub.get_mut(64..68).unwrap().copy_from_slice(b"PE\0\0");
        write_u16(&mut stub, 68, 0x8664);
        write_u16(&mut stub, 70, 1);
        write_u16(&mut stub, 84, 240);
        write_u16(&mut stub, 86, 0x0222);
        write_u16(&mut stub, opt_start, 0x020B);
        write_u32(&mut stub, opt_start.saturating_add(32), 4096);
        write_u32(&mut stub, opt_start.saturating_add(36), 512);
        write_u32(
            &mut stub,
            opt_start.saturating_add(60),
            u32::try_from(headers_aligned).unwrap(),
        );
        write_u16(&mut stub, opt_start.saturating_add(68), 10);
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

    fn build(
        stub_bytes: &[u8],
        cmdline: &[u8],
        kernel: &[u8],
        initrd: &[u8],
    ) -> Result<(Vec<u8>, Vec<Section>)> {
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let cmdline_size = u64::try_from(cmdline.len()).unwrap();
        let kernel_size = u64::try_from(kernel.len()).unwrap();
        let initrd_size = u64::try_from(initrd.len()).unwrap();

        let mut stub = Cursor::new(stub_bytes);
        let probe = probe::probe(&mut stub)?;
        let manifest = prepare::prepare(probe, stub_size, cmdline_size, kernel_size, initrd_size)?;

        let mut output = Vec::new();
        let mut cmdline_r = Cursor::new(cmdline);
        let mut kernel_r = Cursor::new(kernel);
        let mut initrd_r = Cursor::new(initrd);

        let sections = crate::write::write(
            &manifest,
            &mut stub,
            Input {
                reader: &mut cmdline_r,
                size: cmdline_size,
            },
            Input {
                reader: &mut kernel_r,
                size: kernel_size,
            },
            Input {
                reader: &mut initrd_r,
                size: initrd_size,
            },
            &mut output,
        )?;

        Ok((output, sections))
    }

    struct ErrorWriter;

    impl Write for ErrorWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn prepare_uki(
        stub: &[u8],
        cmdline: u64,
        kernel: u64,
        initrd: u64,
    ) -> (crate::prepare::Manifest, Cursor<&[u8]>) {
        let stub_size = u64::try_from(stub.len()).unwrap();
        let mut stub_reader = Cursor::new(stub);
        let probe = probe::probe(&mut stub_reader).unwrap();
        let manifest = prepare::prepare(probe, stub_size, cmdline, kernel, initrd).unwrap();
        (manifest, stub_reader)
    }

    #[test]
    fn write_emits_byte_exact_sections() {
        // ARRANGE
        let stub = minimal_stub();
        let cmdline = b"quiet boot".to_vec();
        let kernel = vec![0x11_u8; 1024];
        let initrd = vec![0x22_u8; 2048];

        // ACT
        let (uki_out, _sections) = build(&stub, &cmdline, &kernel, &initrd).unwrap();

        // ASSERT
        let pe = PeFile64::parse(&*uki_out).unwrap();
        let expected = [
            (b".cmdline".as_slice(), cmdline.as_slice()),
            (b".kernel".as_slice(), kernel.as_slice()),
            (b".initrd".as_slice(), initrd.as_slice()),
        ];
        for (name, data) in expected {
            let section = pe
                .section_table()
                .iter()
                .find(|sec| sec.name.starts_with(name))
                .expect("section should exist");
            let offset = usize::try_from(section.pointer_to_raw_data.get(LE)).unwrap();
            let size = usize::try_from(section.virtual_size.get(LE)).unwrap();
            let section_bytes = uki_out.get(offset..offset + size).unwrap();
            assert_eq!(section_bytes, data);
        }
    }

    #[test]
    fn write_emits_zeroed_gaps_and_padding() {
        // ARRANGE
        let stub = minimal_stub();
        let cmdline = b"quiet".to_vec();
        let kernel = vec![0xBB_u8; 100];
        let initrd = vec![0xCC_u8; 300];
        let (manifest, mut stub_r) = prepare_uki(&stub, 5, 100, 300);

        // ACT
        let mut output = Vec::new();
        let mut cmdline_r = Cursor::new(&cmdline);
        let mut kernel_r = Cursor::new(&kernel);
        let mut initrd_r = Cursor::new(&initrd);
        crate::write::write(
            &manifest,
            &mut stub_r,
            Input {
                reader: &mut cmdline_r,
                size: 5,
            },
            Input {
                reader: &mut kernel_r,
                size: 100,
            },
            Input {
                reader: &mut initrd_r,
                size: 300,
            },
            &mut output,
        )
        .unwrap();

        // ASSERT
        let pe = PeFile64::parse(&*output).unwrap();
        for section in pe.section_table().iter() {
            let offset = usize::try_from(section.pointer_to_raw_data.get(LE)).unwrap();
            let size = usize::try_from(section.virtual_size.get(LE)).unwrap();
            let raw_size = usize::try_from(section.size_of_raw_data.get(LE)).unwrap();
            let padding = output.get(offset + size..offset + raw_size).unwrap();
            assert!(
                padding.iter().all(|&byte| byte == 0),
                "padding after section should be zeroed"
            );
        }
    }

    #[test]
    fn write_output_length_matches_total_size() {
        // ARRANGE
        let stub = minimal_stub();
        let (manifest, _stub_r) = prepare_uki(&stub, 10, 1024, 2048);

        // ACT
        let (uki_out, _sections) = build(&stub, &[0xAA; 10], &[0xBB; 1024], &[0xCC; 2048]).unwrap();

        // ASSERT
        assert_eq!(
            u64::try_from(uki_out.len()).unwrap(),
            manifest.layout().total_size,
            "output length should equal the planned total size"
        );
    }

    #[test]
    fn write_sections_have_nonzero_checksums() {
        // ARRANGE
        let stub = minimal_stub();
        let cmdline = b"quiet".to_vec();
        let kernel = vec![0xBB_u8; 1024];
        let initrd = vec![0xCC_u8; 2048];

        // ACT
        let (_uki_out, sections) = build(&stub, &cmdline, &kernel, &initrd).unwrap();

        // ASSERT
        for section in &sections {
            assert_ne!(
                section.checksum, [0_u8; 32],
                "section '{}' should have a non-zero checksum",
                section.name
            );
        }
    }

    #[test]
    fn write_rejects_short_stream() {
        // ARRANGE
        let stub = minimal_stub();
        let (manifest, mut stub_r) = prepare_uki(&stub, 10, 1024, 2048);

        let mut output = Vec::new();
        let mut cmdline_r = Cursor::new(b"short");
        let mut kernel_r = Cursor::new(vec![0xBB; 1024]);
        let mut initrd_r = Cursor::new(vec![0xCC; 2048]);

        // ACT
        let result = crate::write::write(
            &manifest,
            &mut stub_r,
            Input {
                reader: &mut cmdline_r,
                size: 10,
            },
            Input {
                reader: &mut kernel_r,
                size: 1024,
            },
            Input {
                reader: &mut initrd_r,
                size: 2048,
            },
            &mut output,
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("ended early")
        ));
    }

    #[test]
    fn write_propagates_writer_error() {
        // ARRANGE
        let stub = minimal_stub();
        let (manifest, mut stub_r) = prepare_uki(&stub, 10, 1024, 2048);

        let mut error_writer = ErrorWriter;
        let mut cmdline_r = Cursor::new(vec![0xAA; 10]);
        let mut kernel_r = Cursor::new(vec![0xBB; 1024]);
        let mut initrd_r = Cursor::new(vec![0xCC; 2048]);

        // ACT
        let result = crate::write::write(
            &manifest,
            &mut stub_r,
            Input {
                reader: &mut cmdline_r,
                size: 10,
            },
            Input {
                reader: &mut kernel_r,
                size: 1024,
            },
            Input {
                reader: &mut initrd_r,
                size: 2048,
            },
            &mut error_writer,
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn write_preserves_original_stub_sections() {
        // ARRANGE
        let stub = minimal_stub();
        let original_pe = PeFile64::parse(&*stub).unwrap();
        let text = original_pe
            .section_table()
            .iter()
            .find(|sec| sec.name.starts_with(b".text"))
            .expect("stub should have a .text section");
        let text_offset = usize::try_from(text.pointer_to_raw_data.get(LE)).unwrap();
        let text_size = usize::try_from(text.size_of_raw_data.get(LE)).unwrap();

        // ACT
        let (uki_out, _sections) =
            build(&stub, b"quiet", &vec![0xBB_u8; 1024], &vec![0xCC_u8; 2048]).unwrap();

        // ASSERT
        let out_pe = PeFile64::parse(&*uki_out).unwrap();
        let out_text = out_pe
            .section_table()
            .iter()
            .find(|sec| sec.name.starts_with(b".text"))
            .expect("output should keep the stub .text section");
        let out_offset = usize::try_from(out_text.pointer_to_raw_data.get(LE)).unwrap();
        let out_bytes = uki_out.get(out_offset..out_offset + text_size).unwrap();
        let stub_bytes = stub.get(text_offset..text_offset + text_size).unwrap();
        assert_eq!(
            out_bytes, stub_bytes,
            "original stub section bytes should be preserved"
        );
    }

    #[test]
    fn write_preserves_stub_remainder_bytes() {
        // ARRANGE
        let stub = minimal_stub();
        let stub_size = u64::try_from(stub.len()).unwrap();
        let mut stub_reader = Cursor::new(&stub);
        let probe = probe::probe(&mut stub_reader).unwrap();
        let consumed = probe.consumed();
        let manifest = prepare::prepare(probe, stub_size, 10, 1024, 2048).unwrap();

        let mut output = Vec::new();
        let mut cmdline_r = Cursor::new(vec![0xAA; 10]);
        let mut kernel_r = Cursor::new(vec![0xBB; 1024]);
        let mut initrd_r = Cursor::new(vec![0xCC; 2048]);

        // ACT
        crate::write::write(
            &manifest,
            &mut stub_reader,
            Input {
                reader: &mut cmdline_r,
                size: 10,
            },
            Input {
                reader: &mut kernel_r,
                size: 1024,
            },
            Input {
                reader: &mut initrd_r,
                size: 2048,
            },
            &mut output,
        )
        .unwrap();

        // ASSERT
        let consumed_usize = usize::try_from(consumed).unwrap();
        let remainder = output.get(consumed_usize..stub.len()).unwrap();
        let expected = stub.get(consumed_usize..).unwrap();
        assert_eq!(
            remainder, expected,
            "stub bytes after the probed header must be preserved verbatim"
        );
        assert_eq!(
            u64::try_from(output.len()).unwrap(),
            manifest.layout().total_size
        );
    }

    #[test]
    fn write_kernel_section_is_executable() {
        // ARRANGE
        let stub = minimal_stub();
        let kernel = vec![0xBB_u8; 1024];
        let initrd = vec![0xCC_u8; 2048];

        // ACT
        let (uki_out, _sections) = build(&stub, b"quiet", &kernel, &initrd).unwrap();

        // ASSERT
        let pe = PeFile64::parse(&*uki_out).unwrap();
        let kernel_section = pe
            .section_table()
            .iter()
            .find(|sec| sec.name.starts_with(b".kernel"))
            .expect("output should have a .kernel section");
        let chars = kernel_section.characteristics.get(LE);
        assert!(chars & object_pe::IMAGE_SCN_MEM_EXECUTE != 0);
        assert!(chars & object_pe::IMAGE_SCN_MEM_READ != 0);
    }
}
