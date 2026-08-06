//! Integration tests for yuki UKI building.

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use object::LittleEndian as LE;
    use object::pe as object_pe;
    use object::read::pe::PeFile64;
    use yuki::error::YukiError;
    use yuki::pe::section::Section;
    use yuki::prepare;
    use yuki::probe;
    use yuki::write::{self, Input};

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
        let probed = probe::probe(&mut stub_reader)?;
        let manifest = prepare::prepare(
            probed,
            stub_size,
            cmdline_size,
            kernel_size,
            initrd_size,
            dtb_size,
        )?;

        let mut output = Cursor::new(Vec::new());
        let mut cmdline_r = Cursor::new(cmdline);
        let mut kernel_r = Cursor::new(kernel);
        let mut initrd_r = Cursor::new(initrd);
        let mut dtb_r = dtb.map(Cursor::new);

        let sections = write::write(
            &manifest,
            &mut stub_reader,
            Input {
                reader: &mut cmdline_r,
                size: cmdline_size,
            },
            dtb_r.as_mut().map(|cursor| Input {
                reader: cursor,
                size: dtb_size.unwrap_or_default(),
            }),
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
        let (manifest, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        assert!(manifest.starts_with(b"MZ"));

        let pe = PeFile64::parse(&*manifest).expect("output should be valid PE64");
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
        assert!(
            section_names
                .iter()
                .any(|name| name.starts_with(b".kernel"))
        );
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
        let (manifest, _sections) = build_to_vec(&stub, &cmdline, &kernel, &initrd, Some(&dtb))
            .expect("build with DTB should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*manifest).expect("output should be valid PE64");
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
        assert!(result.is_err(), "invalid file alignment should be rejected");
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
        assert!(
            result.is_err(),
            "invalid section alignment should be rejected"
        );
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
        assert!(
            result.is_err(),
            "invalid size of headers should be rejected"
        );
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
        let (manifest, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");
        let result_pe = PeFile64::parse(&*manifest).expect("output should be valid PE");

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
        let (manifest, _sections) = build_to_vec(&stub, &cmdline, &kernel, &initrd, None)
            .expect("build with large files should succeed");

        // ASSERT
        assert!(manifest.len() > 3 * 1024 * 1024);
        PeFile64::parse(&*manifest).expect("large UKI should be valid PE64");
    }

    #[test]
    fn build_handles_empty_cmdline() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);

        // ACT
        let (manifest, _sections) = build_to_vec(&stub, &[], &kernel, &initrd, None)
            .expect("empty cmdline should be allowed");

        // ASSERT
        PeFile64::parse(&*manifest).expect("should be valid PE");
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
        assert!(result.is_err(), "invalid PE should be rejected");
    }

    #[test]
    fn sections_contain_correct_data() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel_data = fake_kernel(1024);
        let initrd_data = fake_initrd(2048);
        let cmdline_data = sample_cmdline();

        // ACT
        let (manifest, _sections) =
            build_to_vec(&stub, &cmdline_data, &kernel_data, &initrd_data, None)
                .expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*manifest).expect("should be valid PE");
        let sections = pe.section_table();
        let expected_sections = [
            (b".kernel".as_slice(), kernel_data.as_slice()),
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
            let section_data = manifest
                .get(offset..offset + virtual_size)
                .expect("section data should be in bounds");

            assert!(section_data.starts_with(expected_data));
        }
    }

    #[test]
    fn kernel_section_is_executable() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        // ACT
        let (manifest, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*manifest).expect("should be valid PE");
        let kernel_section = pe
            .section_table()
            .iter()
            .find(|section| section.name.starts_with(b".kernel"))
            .expect("should have .kernel section");
        let chars = kernel_section.characteristics.get(LE);

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
        let (manifest, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*manifest).expect("should be valid PE");

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
        let (manifest, _sections) =
            build_to_vec(&stub, &cmdline, &kernel, &initrd, None).expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*manifest).expect("should be valid PE");
        let subsystem = pe.nt_headers().optional_header.subsystem.get(LE);
        assert_eq!(subsystem, object_pe::IMAGE_SUBSYSTEM_EFI_APPLICATION);
    }

    #[test]
    fn build_rejects_oversized_stub_header() {
        // ARRANGE
        let stub = generate_stub_with_section_count(u16::MAX - 2);

        // ACT
        let result = build_to_vec(&stub, b"quiet", b"kernel", b"initrd", None);

        // ASSERT
        assert!(
            result.is_err(),
            "a stub whose header exceeds the probe bound should be rejected"
        );
    }

    #[test]
    fn build_rejects_insufficient_header_capacity() {
        // ARRANGE
        let mut stub_bytes = generate_stub_with_section_count(3);
        write_u32(&mut stub_bytes, 148, 512);

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
        let probed = probe::probe(&mut stub_reader).unwrap();
        let manifest = prepare::prepare(probed, stub_size, 10, 1024, 2048, None).unwrap();

        let mut fail_writer = FailWriter;
        let mut cmdline_r = Cursor::new(vec![0xAA; 10]);
        let mut kernel_r = Cursor::new(vec![0xBB; 1024]);
        let mut initrd_r = Cursor::new(vec![0xCC; 2048]);

        // ACT
        let result = write::write(
            &manifest,
            &mut stub_reader,
            Input {
                reader: &mut cmdline_r,
                size: 10,
            },
            None,
            Input {
                reader: &mut kernel_r,
                size: 1024,
            },
            Input {
                reader: &mut initrd_r,
                size: 2048,
            },
            &mut fail_writer,
        );

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
        let probed = probe::probe(&mut stub_reader).unwrap();
        let manifest = prepare::prepare(
            probed,
            stub_size,
            u64::try_from(cmdline.len()).unwrap(),
            u64::try_from(kernel.len()).unwrap(),
            u64::try_from(initrd.len()).unwrap(),
            None,
        )
        .unwrap();

        let fail_after = manifest.layout().kernel_offset.saturating_add(100);
        let mut limited_writer = LimitedWriter {
            written: 0,
            fail_after,
        };
        let mut cmdline_r = Cursor::new(&cmdline);
        let mut kernel_r = Cursor::new(&kernel);

        // ACT
        let result = write::write(
            &manifest,
            &mut stub_reader,
            Input {
                reader: &mut cmdline_r,
                size: u64::try_from(cmdline.len()).unwrap(),
            },
            None,
            Input {
                reader: &mut kernel_r,
                size: u64::try_from(kernel.len()).unwrap(),
            },
            Input {
                reader: &mut Cursor::new(&initrd),
                size: u64::try_from(initrd.len()).unwrap(),
            },
            &mut limited_writer,
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn build_rejects_section_length_larger_than_stream() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let stub_size = u64::try_from(stub.len()).unwrap();

        let mut stub_reader = Cursor::new(&stub);
        let probed = probe::probe(&mut stub_reader).unwrap();
        let manifest = prepare::prepare(probed, stub_size, 10, 100, 2048, None).unwrap();

        let mut output = Vec::new();
        let mut cmdline_r = Cursor::new(vec![0xAA; 10]);
        let mut kernel_r = Cursor::new(b"kernel".to_vec());
        let mut initrd_r = Cursor::new(vec![0xCC; 2048]);

        // ACT
        let result = write::write(
            &manifest,
            &mut stub_reader,
            Input {
                reader: &mut cmdline_r,
                size: 10,
            },
            None,
            Input {
                reader: &mut kernel_r,
                size: 100,
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
        let probed = probe::probe(&mut stub_reader).unwrap();
        let manifest = prepare::prepare(
            probed,
            stub_size,
            u64::try_from(cmdline.len()).unwrap(),
            u64::try_from(kernel.len()).unwrap(),
            u64::try_from(initrd.len()).unwrap(),
            None,
        )
        .expect("prepare must succeed");
        let layout = manifest.layout();

        let (manifest, _sections) = build_to_vec(&stub_bytes, &cmdline, &kernel, &initrd, None)
            .expect("build must succeed");

        // ASSERT
        assert_eq!(u64::try_from(manifest.len()).unwrap(), layout.total_size);
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
        let probed = probe::probe(&mut stub_reader).unwrap();
        let manifest = prepare::prepare(
            probed,
            stub_size,
            u64::try_from(cmdline.len()).unwrap(),
            u64::try_from(kernel.len()).unwrap(),
            u64::try_from(initrd.len()).unwrap(),
            Some(u64::try_from(dtb.len()).unwrap()),
        )
        .expect("prepare with dtb must succeed");
        let layout = manifest.layout();

        let (manifest, _sections) =
            build_to_vec(&stub_bytes, &cmdline, &kernel, &initrd, Some(&dtb))
                .expect("build with dtb must succeed");

        // ASSERT
        assert_eq!(u64::try_from(manifest.len()).unwrap(), layout.total_size);
    }

    #[test]
    fn compute_layout_rejects_invalid_stub() {
        // ARRANGE & ACT
        let result = probe::probe(&mut Cursor::new(b"not a PE file"));

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn build_rejects_truncated_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 344, 4096);
        let mut stub_reader = Cursor::new(&stub);
        let probed = probe::probe(&mut stub_reader).unwrap();

        // ACT
        let result = prepare::prepare(probed, 2048, 10, 1024, 2048, None);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("stub truncated")
        ));
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
