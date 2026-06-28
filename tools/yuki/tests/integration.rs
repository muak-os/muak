//! Integration tests for yuki UKI building.

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use object::LittleEndian as LE;
    use object::pe as object_pe;
    use object::read::pe::PeFile64;
    use yuki::compute_size;
    use yuki::error::YukiError;
    use yuki::{BuildInput, SizedPart, build};

    use super::fixtures::components::{fake_dtb, fake_initrd, fake_kernel, sample_cmdline};
    use super::fixtures::pe::{generate_minimal_stub, generate_stub_with_section_count, write_u32};

    fn part(reader: &mut Cursor<Vec<u8>>) -> SizedPart<'_> {
        SizedPart {
            len: u64::try_from(reader.get_ref().len()).unwrap_or(0),
            reader,
        }
    }

    fn bytes_part<'a>(reader: &'a mut Cursor<&'static [u8]>) -> SizedPart<'a> {
        SizedPart {
            len: u64::try_from(reader.get_ref().len()).unwrap_or(0),
            reader,
        }
    }

    fn build_to_vec(input: BuildInput<'_>) -> Result<Vec<u8>, YukiError> {
        let mut output = Cursor::new(Vec::new());
        build(input, &mut output)?;
        Ok(output.into_inner())
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

    fn build_result_with_stub(stub: Vec<u8>) -> Result<Vec<u8>, YukiError> {
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        let mut stub_reader = Cursor::new(stub);
        let mut kernel_reader = Cursor::new(kernel);
        let mut initrd_reader = Cursor::new(initrd);
        let mut cmdline_reader = Cursor::new(cmdline);

        build_to_vec(BuildInput {
            stub: part(&mut stub_reader),
            kernel: part(&mut kernel_reader),
            initramfs: part(&mut initrd_reader),
            cmdline: part(&mut cmdline_reader),
            dtb: None,
        })
    }

    #[test]
    fn build_creates_valid_uki() {
        // ARRANGE
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(4096));
        let mut initrd = Cursor::new(fake_initrd(8192));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
        .expect("build should succeed");

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
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(4096));
        let mut initrd = Cursor::new(fake_initrd(8192));
        let mut cmdline = Cursor::new(sample_cmdline());
        let mut dtb = Cursor::new(fake_dtb(1024));

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: Some(part(&mut dtb)),
        })
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
        let result = build_result_with_stub(stub);

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
        let result = build_result_with_stub(stub);

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
        let result = build_result_with_stub(stub);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("invalid size of headers 0")
        ));
    }

    #[test]
    fn build_rejects_section_raw_data_overflow_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 328 + 16, u32::MAX);
        write_u32(&mut stub, 328 + 20, 1);

        // ACT
        let result = build_result_with_stub(stub);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("section raw data end overflow")
        ));
    }

    #[test]
    fn build_rejects_section_virtual_end_overflow_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 328 + 8, 1);
        write_u32(&mut stub, 328 + 12, u32::MAX);

        // ACT
        let result = build_result_with_stub(stub);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("section virtual end overflow")
        ));
    }

    #[test]
    fn build_preserves_original_sections() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let original_pe = PeFile64::parse(&*stub).expect("generated stub should be valid PE");
        let original_section_count = original_pe.section_table().len();
        let mut stub_reader = Cursor::new(stub);
        let mut kernel = Cursor::new(fake_kernel(1024));
        let mut initrd = Cursor::new(fake_initrd(1024));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub_reader),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
        .expect("build should succeed");
        let result_pe = PeFile64::parse(&*uki).expect("output should be valid PE");

        // ASSERT
        assert_eq!(result_pe.section_table().len(), original_section_count + 3);
    }

    #[test]
    fn build_with_large_files() {
        // ARRANGE
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(1024 * 1024));
        let mut initrd = Cursor::new(fake_initrd(2 * 1024 * 1024));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
        .expect("build with large files should succeed");

        // ASSERT
        assert!(uki.len() > 3 * 1024 * 1024);
        PeFile64::parse(&*uki).expect("large UKI should be valid PE64");
    }

    #[test]
    fn build_handles_empty_cmdline() {
        // ARRANGE
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(1024));
        let mut initrd = Cursor::new(fake_initrd(1024));
        let mut cmdline = Cursor::new(Vec::new());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
        .expect("empty cmdline should be allowed");

        // ASSERT
        PeFile64::parse(&*uki).expect("should be valid PE");
    }

    #[test]
    fn build_rejects_invalid_pe_stub() {
        // ARRANGE
        let mut stub = Cursor::new(b"this is not a PE file at all".to_vec());
        let mut kernel = Cursor::new(fake_kernel(1024));
        let mut initrd = Cursor::new(fake_initrd(1024));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let result = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        });

        // ASSERT
        assert!(matches!(result, Err(YukiError::PeParseError(_))));
    }

    #[test]
    fn sections_contain_correct_data() {
        // ARRANGE
        let kernel_data = fake_kernel(1024);
        let initrd_data = fake_initrd(2048);
        let cmdline_data = sample_cmdline();
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(kernel_data.clone());
        let mut initrd = Cursor::new(initrd_data.clone());
        let mut cmdline = Cursor::new(cmdline_data.clone());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
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
                .find(|section| section.name.starts_with(section_name))
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
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(1024));
        let mut initrd = Cursor::new(fake_initrd(1024));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
        .expect("build should succeed");

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
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(1024));
        let mut initrd = Cursor::new(fake_initrd(1024));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
        .expect("build should succeed");

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
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(1024));
        let mut initrd = Cursor::new(fake_initrd(1024));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: part(&mut stub),
            kernel: part(&mut kernel),
            initramfs: part(&mut initrd),
            cmdline: part(&mut cmdline),
            dtb: None,
        })
        .expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("should be valid PE");
        let subsystem = pe.nt_headers().optional_header.subsystem.get(LE);
        assert_eq!(subsystem, object_pe::IMAGE_SUBSYSTEM_EFI_APPLICATION);
    }

    #[test]
    fn generated_stub_is_valid() {
        // ARRANGE
        let stub = generate_minimal_stub();

        // ACT
        let pe = PeFile64::parse(&*stub).expect("generated stub should be valid PE64");

        // ASSERT
        assert!(stub.starts_with(b"MZ"));
        assert_eq!(pe.section_table().len(), 1);
        assert!(
            pe.section_table()
                .iter()
                .next()
                .expect("should have section")
                .name
                .starts_with(b".text")
        );
    }

    #[test]
    fn build_rejects_too_many_sections() {
        // ARRANGE
        let mut stub = Cursor::new(generate_stub_with_section_count(u16::MAX - 2));
        let mut kernel = Cursor::new(b"kernel".to_vec());
        let mut initrd = Cursor::new(b"initrd".to_vec());
        let mut cmdline = Cursor::new(b"quiet".to_vec());
        let mut output = Cursor::new(Vec::new());

        // ACT
        let result = build(
            BuildInput {
                stub: part(&mut stub),
                kernel: part(&mut kernel),
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut output,
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::TooManySections)));
    }

    #[test]
    fn build_rejects_insufficient_header_capacity() {
        // ARRANGE
        let mut stub_bytes = generate_minimal_stub();
        write_u32(&mut stub_bytes, 148, 368);
        let mut stub = Cursor::new(stub_bytes);
        let mut kernel = Cursor::new(b"kernel".to_vec());
        let mut initrd = Cursor::new(b"initrd".to_vec());
        let mut cmdline = Cursor::new(b"quiet".to_vec());
        let mut output = Cursor::new(Vec::new());

        // ACT
        let result = build(
            BuildInput {
                stub: part(&mut stub),
                kernel: part(&mut kernel),
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut output,
        );

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
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(b"kernel".to_vec());
        let mut initrd = Cursor::new(b"initrd".to_vec());
        let mut cmdline = Cursor::new(b"quiet".to_vec());

        // ACT
        let result = build(
            BuildInput {
                stub: part(&mut stub),
                kernel: part(&mut kernel),
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut FailWriter,
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn build_fails_during_section_streaming() {
        // ARRANGE
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(vec![0xAA; 2048]);
        let mut initrd = Cursor::new(vec![0xBB; 2048]);
        let mut cmdline = Cursor::new(b"quiet".to_vec());
        let mut writer = LimitedWriter {
            written: 0,
            fail_after: 1600,
        };

        // ACT
        let result = build(
            BuildInput {
                stub: part(&mut stub),
                kernel: part(&mut kernel),
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut writer,
        );

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn build_rejects_stub_shorter_than_prefix() {
        // ARRANGE
        let stub_bytes = generate_minimal_stub();
        let mut stub = Cursor::new(stub_bytes.clone());
        let mut kernel = Cursor::new(b"kernel".to_vec());
        let mut initrd = Cursor::new(b"initrd".to_vec());
        let mut cmdline = Cursor::new(b"quiet".to_vec());
        let prefix_len = 512_u64;

        // ACT
        let result = build(
            BuildInput {
                stub: SizedPart {
                    len: prefix_len.saturating_sub(1),
                    reader: &mut stub,
                },
                kernel: part(&mut kernel),
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut Cursor::new(Vec::new()),
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("stub length smaller than copied prefix")
        ));
    }

    #[test]
    fn build_handles_stub_longer_than_section_table() {
        // ARRANGE
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(b"kernel".to_vec());
        let mut initrd = Cursor::new(b"initrd".to_vec());
        let mut cmdline = Cursor::new(b"quiet".to_vec());

        // ACT
        let result = build(
            BuildInput {
                stub: SizedPart {
                    len: 1024,
                    reader: &mut stub,
                },
                kernel: part(&mut kernel),
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut Cursor::new(Vec::new()),
        );

        // ASSERT
        assert!(result.is_ok(), "stub longer than sections should succeed");
    }

    #[test]
    fn build_rejects_section_length_larger_than_stream() {
        // ARRANGE
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(b"kernel".to_vec());
        let mut initrd = Cursor::new(b"initrd".to_vec());
        let mut cmdline = Cursor::new(b"quiet".to_vec());

        // ACT
        let result = build(
            BuildInput {
                stub: part(&mut stub),
                kernel: SizedPart {
                    len: 100,
                    reader: &mut kernel,
                },
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut Cursor::new(Vec::new()),
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message)) if message.contains("ended early")
        ));
    }

    #[test]
    fn build_accepts_static_byte_slices() {
        // ARRANGE
        static KERNEL: &[u8] = b"kernel";
        static INITRD: &[u8] = b"initrd";
        static CMDLINE: &[u8] = b"quiet";

        let stub_bytes: &'static [u8] = Box::leak(generate_minimal_stub().into_boxed_slice());
        let mut stub = Cursor::new(stub_bytes);
        let mut kernel = Cursor::new(KERNEL);
        let mut initrd = Cursor::new(INITRD);
        let mut cmdline = Cursor::new(CMDLINE);

        // ACT
        let uki = build_to_vec(BuildInput {
            stub: bytes_part(&mut stub),
            kernel: bytes_part(&mut kernel),
            initramfs: bytes_part(&mut initrd),
            cmdline: bytes_part(&mut cmdline),
            dtb: None,
        })
        .expect("static slices should work");

        // ASSERT
        PeFile64::parse(&*uki).expect("static slices should yield a valid PE");
    }

    #[test]
    fn sections_have_nonzero_checksums() {
        // ARRANGE
        let mut stub = Cursor::new(generate_minimal_stub());
        let mut kernel = Cursor::new(fake_kernel(1024));
        let mut initrd = Cursor::new(fake_initrd(2048));
        let mut cmdline = Cursor::new(sample_cmdline());

        // ACT
        let sections = build(
            BuildInput {
                stub: part(&mut stub),
                kernel: part(&mut kernel),
                initramfs: part(&mut initrd),
                cmdline: part(&mut cmdline),
                dtb: None,
            },
            &mut Cursor::new(Vec::new()),
        )
        .expect("build should succeed");

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

    #[test]
    fn compute_size_matches_build_output() {
        // ARRANGE
        let stub_bytes = generate_minimal_stub();
        let cmdline = sample_cmdline();
        let kernel = fake_kernel(8192);
        let initrd = fake_initrd(4096);

        // ACT
        let stub_len = u64::try_from(stub_bytes.len()).unwrap_or(u64::MAX);
        let computed = compute_size(
            &mut Cursor::new(&stub_bytes),
            stub_len,
            u64::try_from(cmdline.len()).unwrap_or(0),
            u64::try_from(kernel.len()).unwrap_or(0),
            u64::try_from(initrd.len()).unwrap_or(0),
            None,
        )
        .expect("compute_size must succeed");

        let mut stub_reader = Cursor::new(stub_bytes);
        let mut cmdline_reader = Cursor::new(cmdline);
        let mut kernel_reader = Cursor::new(kernel);
        let mut initrd_reader = Cursor::new(initrd);
        let mut output = Vec::new();

        build(
            BuildInput {
                stub: part(&mut stub_reader),
                cmdline: part(&mut cmdline_reader),
                kernel: part(&mut kernel_reader),
                initramfs: part(&mut initrd_reader),
                dtb: None,
            },
            &mut output,
        )
        .expect("build must succeed");

        // ASSERT
        assert_eq!(u64::try_from(output.len()).unwrap_or(0), computed);
    }

    #[test]
    fn compute_size_matches_build_output_with_dtb() {
        // ARRANGE
        let stub_bytes = generate_minimal_stub();
        let cmdline = sample_cmdline();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(2048);
        let dtb = fake_dtb(512);

        // ACT
        let stub_len = u64::try_from(stub_bytes.len()).unwrap_or(u64::MAX);
        let computed = compute_size(
            &mut Cursor::new(&stub_bytes),
            stub_len,
            u64::try_from(cmdline.len()).unwrap_or(0),
            u64::try_from(kernel.len()).unwrap_or(0),
            u64::try_from(initrd.len()).unwrap_or(0),
            Some(u64::try_from(dtb.len()).unwrap_or(0)),
        )
        .expect("compute_size with dtb must succeed");

        let mut stub_reader = Cursor::new(stub_bytes);
        let mut cmdline_reader = Cursor::new(cmdline);
        let mut kernel_reader = Cursor::new(kernel);
        let mut initrd_reader = Cursor::new(initrd);
        let mut dtb_reader = Cursor::new(dtb);
        let mut output = Vec::new();

        build(
            BuildInput {
                stub: part(&mut stub_reader),
                cmdline: part(&mut cmdline_reader),
                kernel: part(&mut kernel_reader),
                initramfs: part(&mut initrd_reader),
                dtb: Some(part(&mut dtb_reader)),
            },
            &mut output,
        )
        .expect("build with dtb must succeed");

        // ASSERT
        assert_eq!(u64::try_from(output.len()).unwrap_or(0), computed);
    }

    #[test]
    fn compute_size_matches_with_stub_longer_than_headers() {
        // ARRANGE
        let stub_bytes = generate_minimal_stub();
        let cmdline = vec![0x01];

        // ACT
        let stub_len = u64::try_from(stub_bytes.len()).unwrap_or(u64::MAX);
        let computed = compute_size(
            &mut Cursor::new(&stub_bytes),
            stub_len,
            u64::try_from(cmdline.len()).unwrap_or(0),
            2,
            3,
            None,
        )
        .expect("compute_size must succeed");

        let mut stub_reader = Cursor::new(stub_bytes);
        let mut cmdline_reader = Cursor::new(cmdline);
        let mut kernel_reader = Cursor::new(vec![0x02; 2]);
        let mut initrd_reader = Cursor::new(vec![0x03; 3]);
        let mut output = Vec::new();

        build(
            BuildInput {
                stub: SizedPart {
                    len: u64::try_from(stub_reader.get_ref().len()).unwrap_or(0),
                    reader: &mut stub_reader,
                },
                cmdline: part(&mut cmdline_reader),
                kernel: part(&mut kernel_reader),
                initramfs: part(&mut initrd_reader),
                dtb: None,
            },
            &mut output,
        )
        .expect("build must succeed");

        // ASSERT
        assert_eq!(u64::try_from(output.len()).unwrap_or(0), computed);
    }

    #[test]
    fn compute_size_rejects_invalid_stub() {
        // ARRANGE & ACT
        let result = compute_size(&mut Cursor::new(b"not a PE file"), 13, 10, 10, 10, None);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn compute_size_rejects_oversized_component() {
        // ARRANGE
        let stub_bytes = generate_minimal_stub();

        // ACT
        let stub_len = u64::try_from(stub_bytes.len()).unwrap_or(u64::MAX);
        let result = compute_size(
            &mut Cursor::new(&stub_bytes),
            stub_len,
            u64::from(u32::MAX).saturating_add(1),
            10,
            10,
            None,
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg)) if msg.contains("too large")
        ));
    }
}
