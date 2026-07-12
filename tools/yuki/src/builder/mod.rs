//! Typestate-based UKI builder.

mod io;

use core::marker::PhantomData;
use std::io::{Read, Write};

use ring::digest;

use crate::align;
use crate::error::{Result, YukiError};
use crate::layout::BuildState;
use crate::pe::section;

/// Typestate: needs stub.
pub struct NeedsStub;
/// Typestate: needs cmdline.
pub struct NeedsCmdline;
/// Typestate: needs kernel (DTB optional).
pub struct NeedsKernel;
/// Typestate: needs initramfs.
pub struct NeedsInitramfs;
/// Typestate: finished.
pub struct Finished;

/// Typestate-based UKI builder that enforces correct component ordering at compile time.
pub struct Builder<'a, W: Write, State> {
    writer: &'a mut W,
    state: BuildState,
    sections: Vec<section::Section>,
    _state: PhantomData<State>,
}

impl<'a, W: Write> Builder<'a, W, NeedsStub> {
    /// Creates a new builder from computed layout state.
    pub fn new(state: BuildState, writer: &'a mut W) -> Self {
        Self {
            writer,
            state,
            sections: Vec::new(),
            _state: PhantomData,
        }
    }

    /// Writes the stub EFI binary.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails or the stub data is incomplete.
    pub fn add_stub(self, stub_reader: &mut dyn Read) -> Result<Builder<'a, W, NeedsCmdline>> {
        self.writer.write_all(&self.state.stub_prefix)?;

        let prefix_len = u64::try_from(self.state.stub_prefix.len()).map_err(|_e| {
            YukiError::InvalidPeStructure("stub prefix length overflow".to_owned())
        })?;
        let remaining = self
            .state
            .stub_size
            .checked_sub(prefix_len)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("stub length smaller than copied prefix".to_owned())
            })?;
        io::copy_exact(stub_reader, self.writer, remaining, "stub", &mut |_| {})?;

        let gap = self
            .state
            .layout
            .cmdline_offset
            .checked_sub(self.state.stub_size)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("cmdline offset before stub end".to_owned())
            })?;
        io::write_gap(self.writer, gap)?;

        Ok(Builder {
            writer: self.writer,
            state: self.state,
            sections: self.sections,
            _state: PhantomData,
        })
    }
}

impl<'a, W: Write> Builder<'a, W, NeedsCmdline> {
    /// Writes the kernel command line.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails or the cmdline data is incomplete.
    pub fn add_cmdline(mut self, reader: &mut dyn Read) -> Result<Builder<'a, W, NeedsKernel>> {
        self.write_section(reader, ".cmdline")?;

        Ok(Builder {
            writer: self.writer,
            state: self.state,
            sections: self.sections,
            _state: PhantomData,
        })
    }
}

impl<'a, W: Write> Builder<'a, W, NeedsKernel> {
    /// Optionally writes a device tree blob.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails or the DTB data is incomplete.
    pub fn add_dtb(mut self, reader: &mut dyn Read) -> Result<Builder<'a, W, NeedsKernel>> {
        if self.state.has_dtb {
            self.write_section(reader, ".dtb")?;
        }

        Ok(self)
    }

    /// Writes the kernel image.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails or the kernel data is incomplete.
    pub fn add_kernel(mut self, reader: &mut dyn Read) -> Result<Builder<'a, W, NeedsInitramfs>> {
        self.write_section(reader, ".linux")?;

        Ok(Builder {
            writer: self.writer,
            state: self.state,
            sections: self.sections,
            _state: PhantomData,
        })
    }
}

impl<'a, W: Write> Builder<'a, W, NeedsInitramfs> {
    /// Writes the initramfs image.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails or the initramfs data is incomplete.
    pub fn add_initramfs(mut self, reader: &mut dyn Read) -> Result<Builder<'a, W, Finished>> {
        self.write_section(reader, ".initrd")?;

        Ok(Builder {
            writer: self.writer,
            state: self.state,
            sections: self.sections,
            _state: PhantomData,
        })
    }
}

impl<W: Write> Builder<'_, W, Finished> {
    /// Finishes building and returns section metadata with checksums.
    ///
    /// # Errors
    ///
    /// Returns an error if finalization fails.
    pub fn finish(self) -> Result<Vec<section::Section>> {
        Ok(self.sections)
    }
}

impl<W: Write, State> Builder<'_, W, State> {
    fn write_section(&mut self, reader: &mut dyn Read, name: &'static str) -> Result<()> {
        let section_meta = self
            .state
            .table
            .sections
            .iter()
            .find(|sec| sec.name == name)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure(format!("section '{name}' not in layout"))
            })?;
        let size = u64::try_from(section_meta.size).map_err(|_e| {
            YukiError::InvalidPeStructure(format!("section '{name}' size overflow"))
        })?;

        let mut ctx = digest::Context::new(&digest::SHA256);
        io::copy_exact(reader, self.writer, size, name, &mut |chunk| {
            ctx.update(chunk);
        })?;

        write_zero_padding(
            self.writer,
            self.state.file_alignment,
            align::u64_to_usize(size)?,
        )?;

        let digest = ctx.finish();
        let mut section = section_meta.clone();
        section.checksum.copy_from_slice(digest.as_ref());
        self.sections.push(section);

        Ok(())
    }
}

fn write_zero_padding<W: Write>(
    writer: &mut W,
    file_alignment: u32,
    actual_size: usize,
) -> Result<()> {
    let actual_u32 = align::usize_to_u32(actual_size)?;
    let aligned = align::to(actual_u32, file_alignment);

    io::write_gap(writer, u64::from(aligned.saturating_sub(actual_u32)))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::layout;

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
    fn builder_typestate_flow() {
        // ARRANGE
        let stub_bytes = minimal_stub();
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let (layout, state) = layout::compute(
            &mut Cursor::new(&stub_bytes),
            stub_size,
            10,
            1024,
            2048,
            None,
        )
        .unwrap();

        let mut output = Vec::new();
        let mut stub_reader = Cursor::new(&stub_bytes);
        let mut cmdline_data = Cursor::new(vec![0xAA; 10]);
        let mut kernel_data = Cursor::new(vec![0xBB; 1024]);
        let mut initrd_data = Cursor::new(vec![0xCC; 2048]);

        // ACT
        let builder = Builder::new(state, &mut output);
        let builder = builder.add_stub(&mut stub_reader).unwrap();
        let builder = builder.add_cmdline(&mut cmdline_data).unwrap();
        let builder = builder.add_kernel(&mut kernel_data).unwrap();
        let builder = builder.add_initramfs(&mut initrd_data).unwrap();
        let sections = builder.finish().unwrap();

        // ASSERT
        assert_eq!(sections.len(), 3);
        assert_eq!(sections.first().unwrap().name, ".cmdline");
        assert_eq!(sections.get(1).unwrap().name, ".linux");
        assert_eq!(sections.get(2).unwrap().name, ".initrd");
        assert_eq!(output.len(), usize::try_from(layout.total_size).unwrap());
    }

    #[test]
    fn builder_with_dtb() {
        // ARRANGE
        let stub_bytes = minimal_stub();
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let (_layout, state) = layout::compute(
            &mut Cursor::new(&stub_bytes),
            stub_size,
            10,
            1024,
            2048,
            Some(512),
        )
        .unwrap();

        let mut output = Vec::new();
        let mut stub_reader = Cursor::new(&stub_bytes);
        let mut cmdline_data = Cursor::new(vec![0xAA; 10]);
        let mut dtb_data = Cursor::new(vec![0xDD; 512]);
        let mut kernel_data = Cursor::new(vec![0xBB; 1024]);
        let mut initrd_data = Cursor::new(vec![0xCC; 2048]);

        // ACT
        let builder = Builder::new(state, &mut output);
        let builder = builder.add_stub(&mut stub_reader).unwrap();
        let builder = builder.add_cmdline(&mut cmdline_data).unwrap();
        let builder = builder.add_dtb(&mut dtb_data).unwrap();
        let builder = builder.add_kernel(&mut kernel_data).unwrap();
        let builder = builder.add_initramfs(&mut initrd_data).unwrap();
        let sections = builder.finish().unwrap();

        // ASSERT
        assert_eq!(sections.len(), 4);
        assert_eq!(sections.first().unwrap().name, ".cmdline");
        assert_eq!(sections.get(1).unwrap().name, ".dtb");
        assert_eq!(sections.get(2).unwrap().name, ".linux");
        assert_eq!(sections.get(3).unwrap().name, ".initrd");
    }

    #[test]
    fn builder_propagates_writer_error() {
        // ARRANGE
        let stub_bytes = minimal_stub();
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let (_layout, state) = layout::compute(
            &mut Cursor::new(&stub_bytes),
            stub_size,
            10,
            1024,
            2048,
            None,
        )
        .unwrap();

        let mut error_writer = ErrorWriter;
        let mut stub_reader = Cursor::new(&stub_bytes);

        // ACT
        let builder = Builder::new(state, &mut error_writer);
        let result = builder.add_stub(&mut stub_reader);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn builder_rejects_short_stream() {
        // ARRANGE
        let stub_bytes = minimal_stub();
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let (_layout, state) = layout::compute(
            &mut Cursor::new(&stub_bytes),
            stub_size,
            100,
            1024,
            2048,
            None,
        )
        .unwrap();

        let mut output = Vec::new();
        let mut stub_reader = Cursor::new(&stub_bytes);
        let mut cmdline_data = Cursor::new(vec![0xAA; 10]);

        // ACT
        let builder = Builder::new(state, &mut output);
        let builder = builder.add_stub(&mut stub_reader).unwrap();
        let result = builder.add_cmdline(&mut cmdline_data);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg)) if msg.contains("ended early")
        ));
    }

    #[test]
    fn sections_have_nonzero_checksums() {
        // ARRANGE
        let stub_bytes = minimal_stub();
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let (_layout, state) = layout::compute(
            &mut Cursor::new(&stub_bytes),
            stub_size,
            10,
            1024,
            2048,
            None,
        )
        .unwrap();

        let mut output = Vec::new();
        let mut stub_reader = Cursor::new(&stub_bytes);
        let mut cmdline_data = Cursor::new(vec![0xAA; 10]);
        let mut kernel_data = Cursor::new(vec![0xBB; 1024]);
        let mut initrd_data = Cursor::new(vec![0xCC; 2048]);

        // ACT
        let builder = Builder::new(state, &mut output);
        let builder = builder.add_stub(&mut stub_reader).unwrap();
        let builder = builder.add_cmdline(&mut cmdline_data).unwrap();
        let builder = builder.add_kernel(&mut kernel_data).unwrap();
        let builder = builder.add_initramfs(&mut initrd_data).unwrap();
        let sections = builder.finish().unwrap();

        // ASSERT
        for sec in &sections {
            assert_ne!(
                sec.checksum, [0_u8; 32],
                "section {} should have non-zero checksum",
                sec.name
            );
        }
    }
}
