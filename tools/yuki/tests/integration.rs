//! Integration tests for yuki UKI building.

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use object::LittleEndian as LE;
    use object::pe as object_pe;
    use object::read::pe::PeFile64;
    use yuki::builder::{Builder, Finished, NeedsCmdline, NeedsInitramfs, NeedsKernel, NeedsStub};
    use yuki::error::YukiError;
    use yuki::layout;
    use yuki::pe::section::Section;

    use super::fixtures::components::{fake_dtb, fake_initrd, fake_kernel, sample_cmdline};
    use super::fixtures::pe::{generate_minimal_stub, generate_stub_with_section_count, write_u32};

    fn build_to_vec(
        stub_bytes: &[u8],
        cmdline: &[u8],
        kernel: &[u8],
        initrd: &[u8],
        dtb: Option<&[u8]>,
    ) -> Result<(Vec<u8>, Vec<Section>), YukiError> {
        let stub_size = u64::try_from(stub_bytes.len()).unwrap_or(0);
        let cmdline_size = u64::try_from(cmdline.len()).unwrap_or(0);
        let kernel_size = u64::try_from(kernel.len()).unwrap_or(0);
        let initrd_size = u64::try_from(initrd.len()).unwrap_or(0);
        let dtb_size = dtb.map(|dtb_data| u64::try_from(dtb_data.len()).unwrap_or(0));

        let mut stub_reader = Cursor::new(stub_bytes);
        let (_layout, state) = layout::compute(
            &mut stub_reader,
            stub_size,
            cmdline_size,
            kernel_size,
            initrd_size,
            dtb_size,
        )?;

        let mut output = Cursor::new(Vec::new());
        let mut stub_r = Cursor::new(stub_bytes);
        let mut cmdline_r = Cursor::new(cmdline);
        let mut kernel_r = Cursor::new(kernel);
        let mut initrd_r = Cursor::new(initrd);

        let builder: Builder<'_, _, NeedsStub> = Builder::new(state, &mut output);
        let builder: Builder<'_, _, NeedsCmdline> = builder.add_stub(&mut stub_r)?;
        let builder: Builder<'_, _, NeedsKernel> = builder.add_cmdline(&mut cmdline_r)?;

        let builder = if let Some(dtb_data) = dtb {
            let mut dtb_r = Cursor::new(dtb_data);
            builder.add_dtb(&mut dtb_r)?
        } else {
            builder
        };

        let builder: Builder<'_, _, NeedsInitramfs> = builder.add_kernel(&mut kernel_r)?;
        let builder: Builder<'_, _, Finished> = builder.add_initramfs(&mut initrd_r)?;
        let sections = builder.finish()?;

        Ok((output.into_inner(), sections))
    }

    fn non_zero_allowed(allowed: usize) -> std::io::Result<core::num::NonZeroUsize> {
        core::num::NonZeroUsize::new(allowed)
            .ok_or_else(|| std::io::Error::other("writer failed after limit"))
    }

    struct FailWriter;

    impl Write for FailWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct LimitedWriter {
        written: u64,
        fail_after: u64,
    }

    impl Write for LimitedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let allowed = usize::try_from(self.fail_after.saturating_sub(self.written))
                .unwrap_or(usize::MAX)
                .min(buf.len());
            let allowed = non_zero_allowed(allowed)?;

            self.written = self
                .written
                .saturating_add(u64::try_from(allowed.get()).unwrap_or(u64::MAX));
            Ok(allowed.get())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn build_creates_valid_uki() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(4096);
        let initrd = fake_initrd(8192);
        let cmdline = sample_cmdline();

        // ACT
        let (uki, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        assert!(uki.starts_with(b"MZ"));

        let pe = PeFile64::parse(&*uki).expect("output should be valid PE64");
        let section_names: Vec<&[u8]> = pe
            .section_table()
            .iter()
            .map(|section| section.name.as_slice())
            .collect();

        assert!(
            section_names
                .iter()
                .any(|name| name.starts_with(b".cmdline"))
        );
        assert!(section_names.iter().any(|name| name.starts_with(b".linux")));
        assert!(
            section_names
                .iter()
                .any(|name| name.starts_with(b".initrd"))
        );
    }

    #[test]
    fn build_with_dtb() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(4096);
        let initrd = fake_initrd(8192);
        let cmdline = sample_cmdline();
        let dtb = fake_dtb(1024);

        // ACT
        let (uki, _sections) = build_to_vec(&stub, &cmdline, &kernel, &initrd, Some(&dtb))
            .expect("build with DTB should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("output should be valid PE64");
        let section_names: Vec<&[u8]> = pe
            .section_table()
            .iter()
            .map(|section| section.name.as_slice())
            .collect();

        assert!(section_names.iter().any(|name| name.starts_with(b".dtb")));
    }

    #[test]
    fn build_rejects_invalid_file_alignment_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 88 + 36, 3);

        // ACT
        let result = build_to_vec(
            &stub,
            b"quiet",
            &fake_kernel(1024),
            &fake_initrd(1024),
            None,
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("invalid file alignment")
        ));
    }

    #[test]
    fn build_rejects_zero_section_alignment_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 88 + 32, 0);

        // ACT
        let result = build_to_vec(
            &stub,
            b"quiet",
            &fake_kernel(1024),
            &fake_initrd(1024),
            None,
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("invalid section alignment")
        ));
    }

    #[test]
    fn build_rejects_zero_size_of_headers_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 148, 0);

        // ACT
        let result = build_to_vec(
            &stub,
            b"quiet",
            &fake_kernel(1024),
            &fake_initrd(1024),
            None,
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("invalid size of headers 0")
        ));
    }

    #[test]
    fn build_preserves_original_sections() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let original_pe = PeFile64::parse(&*stub).expect("generated stub should be valid PE");
        let original_section_count = original_pe.section_table().len();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        // ACT
        let (uki, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");
        let result_pe = PeFile64::parse(&*uki).expect("output should be valid PE");

        // ASSERT
        assert_eq!(result_pe.section_table().len(), original_section_count + 3);
    }

    #[test]
    fn build_with_large_files() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024 * 1024);
        let initrd = fake_initrd(2 * 1024 * 1024);
        let cmdline = sample_cmdline();

        // ACT
        let (uki, _sections) = build_to_vec(&stub, &cmdline, &kernel, &initrd, None)
            .expect("build with large files should succeed");

        // ASSERT
        assert!(uki.len() > 3 * 1024 * 1024);
        PeFile64::parse(&*uki).expect("large UKI should be valid PE64");
    }

    #[test]
    fn build_handles_empty_cmdline() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);

        // ACT
        let (uki, _sections) = build_to_vec(&stub, &[], &kernel, &initrd, None)
            .expect("empty cmdline should be allowed");

        // ASSERT
        PeFile64::parse(&*uki).expect("should be valid PE");
    }

    #[test]
    fn build_rejects_invalid_pe_stub() {
        // ARRANGE
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);

        // ACT
        let result = build_to_vec(
            b"this is not a PE file at all",
            b"quiet",
            &kernel,
            &initrd,
            None,
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::PeParseError(_))));
    }

    #[test]
    fn sections_contain_correct_data() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel_data = fake_kernel(1024);
        let initrd_data = fake_initrd(2048);
        let cmdline_data = sample_cmdline();

        // ACT
        let (uki, _sections) = build_to_vec(&stub, &cmdline_data, &kernel_data, &initrd_data, None)
            .expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("should be valid PE");
        let sections = pe.section_table();
        let expected_sections = [
            (b".linux".as_slice(), kernel_data.as_slice()),
            (b".initrd".as_slice(), initrd_data.as_slice()),
            (b".cmdline".as_slice(), cmdline_data.as_slice()),
        ];

        for (section_name, expected_data) in expected_sections {
            let section = sections
                .iter()
                .find(|sec| sec.name.starts_with(section_name))
                .expect("expected section should exist");
            let offset = usize::try_from(section.pointer_to_raw_data.get(LE)).unwrap_or(0);
            let virtual_size = usize::try_from(section.virtual_size.get(LE)).unwrap_or(0);
            let section_data = uki
                .get(offset..offset + virtual_size)
                .expect("section data should be in bounds");

            assert!(section_data.starts_with(expected_data));
        }
    }

    #[test]
    fn linux_section_is_executable() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        // ACT
        let (uki, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("should be valid PE");
        let linux_section = pe
            .section_table()
            .iter()
            .find(|section| section.name.starts_with(b".linux"))
            .expect("should have .linux section");
        let chars = linux_section.characteristics.get(LE);

        assert!(chars & object_pe::IMAGE_SCN_MEM_EXECUTE != 0);
        assert!(chars & object_pe::IMAGE_SCN_MEM_READ != 0);
    }

    #[test]
    fn data_sections_are_not_executable() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        // ACT
        let (uki, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("should be valid PE");

        for section in pe.section_table().iter().filter(|section| {
            section.name.starts_with(b".cmdline") || section.name.starts_with(b".initrd")
        }) {
            let chars = section.characteristics.get(LE);
            assert!(chars & object_pe::IMAGE_SCN_MEM_EXECUTE == 0);
        }
    }

    #[test]
    fn output_is_efi_application() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        // ACT
        let (uki, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("should be valid PE");
        let subsystem = pe.nt_headers().optional_header.subsystem.get(LE);
        assert_eq!(subsystem, object_pe::IMAGE_SUBSYSTEM_EFI_APPLICATION);
    }

    #[test]
    fn build_rejects_too_many_sections() {
        // ARRANGE
        let stub = generate_stub_with_section_count(u16::MAX - 2);

        // ACT
        let result = build_to_vec(&stub, b"quiet", b"kernel", b"initrd", None);

        // ASSERT
        assert!(matches!(result, Err(YukiError::TooManySections)));
    }

    #[test]
    fn build_rejects_insufficient_header_capacity() {
        // ARRANGE
        let mut stub_bytes = generate_minimal_stub();
        write_u32(&mut stub_bytes, 148, 368);

        // ACT
        let result = build_to_vec(&stub_bytes, b"quiet", b"kernel", b"initrd", None);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("section table exceeds size of headers")
        ));
    }

    #[test]
    fn build_propagates_writer_error() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let stub_size = u64::try_from(stub.len()).unwrap();

        let mut stub_reader = Cursor::new(&stub);
        let (_layout, state) =
            layout::compute(&mut stub_reader, stub_size, 10, 1024, 2048, None).unwrap();

        let mut fail_writer = FailWriter;
        let mut stub_r = Cursor::new(&stub);

        // ACT
        let builder: Builder<'_, _, NeedsStub> = Builder::new(state, &mut fail_writer);
        let result = builder.add_stub(&mut stub_r);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn build_fails_during_section_streaming() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let stub_size = u64::try_from(stub.len()).unwrap();
        let kernel = vec![0xAA; 2048];
        let initrd = vec![0xBB; 2048];
        let cmdline = b"quiet".to_vec();

        let mut stub_reader = Cursor::new(&stub);
        let (layout, state) = layout::compute(
            &mut stub_reader,
            stub_size,
            u64::try_from(cmdline.len()).unwrap(),
            u64::try_from(kernel.len()).unwrap(),
            u64::try_from(initrd.len()).unwrap(),
            None,
        )
        .unwrap();

        let fail_after = layout.kernel_offset.saturating_add(100);
        let mut limited_writer = LimitedWriter {
            written: 0,
            fail_after,
        };
        let mut stub_r = Cursor::new(&stub);
        let mut cmdline_r = Cursor::new(&cmdline);
        let mut kernel_r = Cursor::new(&kernel);

        // ACT
        let builder: Builder<'_, _, NeedsStub> = Builder::new(state, &mut limited_writer);
        let builder = builder.add_stub(&mut stub_r).unwrap();
        let builder = builder.add_cmdline(&mut cmdline_r).unwrap();
        let result = builder.add_kernel(&mut kernel_r);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn build_rejects_section_length_larger_than_stream() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let stub_size = u64::try_from(stub.len()).unwrap();

        let mut stub_reader = Cursor::new(&stub);
        let (_layout, state) =
            layout::compute(&mut stub_reader, stub_size, 10, 100, 2048, None).unwrap();

        let mut output = Vec::new();
        let mut stub_r = Cursor::new(&stub);
        let mut cmdline_r = Cursor::new(vec![0xAA; 10]);
        let mut kernel_r = Cursor::new(b"kernel".to_vec());

        // ACT
        let builder: Builder<'_, _, NeedsStub> = Builder::new(state, &mut output);
        let builder = builder.add_stub(&mut stub_r).unwrap();
        let builder = builder.add_cmdline(&mut cmdline_r).unwrap();
        let result = builder.add_kernel(&mut kernel_r);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message)) if message.contains("ended early")
        ));
    }

    #[test]
    fn compute_layout_matches_build_output() {
        // ARRANGE
        let stub_bytes = generate_minimal_stub();
        let cmdline = sample_cmdline();
        let kernel = fake_kernel(8192);
        let initrd = fake_initrd(4096);

        // ACT
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let mut stub_reader = Cursor::new(&stub_bytes);
        let (layout, _state) = layout::compute(
            &mut stub_reader,
            stub_size,
            u64::try_from(cmdline.len()).unwrap(),
            u64::try_from(kernel.len()).unwrap(),
            u64::try_from(initrd.len()).unwrap(),
            None,
        )
        .expect("compute must succeed");

        let (uki, _sections) = build_to_vec(&stub_bytes, &cmdline, &kernel, &initrd, None)
            .expect("build must succeed");

        // ASSERT
        assert_eq!(u64::try_from(uki.len()).unwrap(), layout.total_size);
    }

    #[test]
    fn compute_layout_matches_build_output_with_dtb() {
        // ARRANGE
        let stub_bytes = generate_minimal_stub();
        let cmdline = sample_cmdline();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(2048);
        let dtb = fake_dtb(512);

        // ACT
        let stub_size = u64::try_from(stub_bytes.len()).unwrap();
        let mut stub_reader = Cursor::new(&stub_bytes);
        let (layout, _state) = layout::compute(
            &mut stub_reader,
            stub_size,
            u64::try_from(cmdline.len()).unwrap(),
            u64::try_from(kernel.len()).unwrap(),
            u64::try_from(initrd.len()).unwrap(),
            Some(u64::try_from(dtb.len()).unwrap()),
        )
        .expect("compute with dtb must succeed");

        let (uki, _sections) = build_to_vec(&stub_bytes, &cmdline, &kernel, &initrd, Some(&dtb))
            .expect("build with dtb must succeed");

        // ASSERT
        assert_eq!(u64::try_from(uki.len()).unwrap(), layout.total_size);
    }

    #[test]
    fn compute_layout_rejects_invalid_stub() {
        // ARRANGE & ACT
        let result = layout::compute(&mut Cursor::new(b"not a PE file"), 13, 10, 10, 10, None);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn sections_have_nonzero_checksums() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(2048);
        let cmdline = sample_cmdline();

        // ACT
        let (_uki, sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        assert_eq!(sections.len(), 3);
        for section in &sections {
            assert_ne!(
                section.checksum, [0_u8; 32],
                "section {} should have a non-zero checksum",
                section.name
            );
        }
    }
}
