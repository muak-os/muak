//! Integration tests for yuki UKI building.

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use object::LittleEndian as LE;
    use object::pe as object_pe;
    use object::read::pe::PeFile64;
    use yuki::error::YukiError;
    use yuki::{BuildInput, build};

    use super::fixtures::components::{fake_dtb, fake_initrd, fake_kernel, sample_cmdline};
    use super::fixtures::pe::{generate_minimal_stub, generate_stub_with_section_count};

    fn write_bytes(bytes: &mut [u8], offset: usize, data: &[u8]) {
        let end = offset.saturating_add(data.len());
        if let Some(dst) = bytes.get_mut(offset..end) {
            dst.copy_from_slice(data);
        }
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        write_bytes(bytes, offset, &value.to_le_bytes());
    }

    #[test]
    fn build_creates_valid_uki() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(4096);
        let initrd = fake_initrd(8192);
        let cmdline = sample_cmdline();

        // ACT
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
        .expect("build should succeed");

        // ASSERT
        assert!(uki.starts_with(b"MZ"), "output should start with MZ");

        let pe = PeFile64::parse(&*uki).expect("output should be valid PE64");

        let section_names: Vec<&[u8]> = pe
            .section_table()
            .iter()
            .map(|section| section.name.as_slice())
            .collect();

        assert!(
            section_names
                .iter()
                .any(|name| name.starts_with(b".cmdline")),
            "should have .cmdline section, got: {section_names:?}"
        );
        assert!(
            section_names.iter().any(|name| name.starts_with(b".linux")),
            "should have .linux section"
        );
        assert!(
            section_names
                .iter()
                .any(|name| name.starts_with(b".initrd")),
            "should have .initrd section"
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
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: Some(&dtb),
            luks_key: None,
        })
        .expect("build with DTB should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("output should be valid PE64");
        let section_names: Vec<&[u8]> = pe
            .section_table()
            .iter()
            .map(|section| section.name.as_slice())
            .collect();

        assert!(
            section_names.iter().any(|name| name.starts_with(b".dtb")),
            "should have .dtb section, got: {section_names:?}"
        );
    }

    fn build_result_with_stub(stub: &[u8]) -> Result<Vec<u8>, YukiError> {
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        build(&BuildInput {
            stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
    }

    #[test]
    fn build_rejects_invalid_file_alignment_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 88 + 36, 3);

        // ACT
        let result = build_result_with_stub(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("invalid file alignment"))
        );
    }

    #[test]
    fn build_rejects_zero_section_alignment_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 88 + 32, 0);

        // ACT
        let result = build_result_with_stub(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("invalid section alignment"))
        );
    }

    #[test]
    fn build_rejects_zero_size_of_headers_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 148, 0);

        // ACT
        let result = build_result_with_stub(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("invalid size of headers 0"))
        );
    }

    #[test]
    fn build_rejects_section_raw_data_overflow_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 328 + 16, u32::MAX);
        write_u32(&mut stub, 328 + 20, 1);

        // ACT
        let result = build_result_with_stub(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section raw data end overflow"))
        );
    }

    #[test]
    fn build_rejects_section_virtual_end_overflow_in_stub() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        write_u32(&mut stub, 328 + 8, 1);
        write_u32(&mut stub, 328 + 12, u32::MAX);

        // ACT
        let result = build_result_with_stub(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section virtual end overflow"))
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
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
        .expect("build should succeed");
        let result_pe = PeFile64::parse(&*uki).expect("output should be valid PE");

        // ASSERT
        assert_eq!(
            result_pe.section_table().len(),
            original_section_count + 3,
            "should add exactly 3 sections"
        );
    }

    #[test]
    fn build_with_large_files() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024 * 1024);
        let initrd = fake_initrd(2 * 1024 * 1024);
        let cmdline = sample_cmdline();

        // ACT
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
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
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: b"",
            dtb: None,
            luks_key: None,
        })
        .expect("empty cmdline should be allowed");

        // ASSERT
        PeFile64::parse(&*uki).expect("should be valid PE");
    }

    #[test]
    fn build_rejects_invalid_pe_stub() {
        // ARRANGE
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        // ACT
        let result = build(&BuildInput {
            stub: b"this is not a PE file at all",
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        });

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::PeParseError(_))),
            "should fail with PE parse error for invalid stub, got: {result:?}"
        );
    }

    #[test]
    fn sections_contain_correct_data() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel_data = fake_kernel(1024);
        let initrd_data = fake_initrd(2048);
        let cmdline_data = sample_cmdline();

        // ACT
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel_data,
            initramfs: &initrd_data,
            cmdline: &cmdline_data,
            dtb: None,
            luks_key: None,
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
            let offset = usize::try_from(section.pointer_to_raw_data.get(LE))
                .expect("section offset should fit in usize");
            let virtual_size = usize::try_from(section.virtual_size.get(LE))
                .expect("virtual size should fit in usize");
            let section_data = uki
                .get(offset..offset + virtual_size)
                .expect("section data should be in bounds");

            assert!(
                section_data.starts_with(expected_data),
                "section should contain expected data"
            );
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
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
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

        assert!(
            chars & object_pe::IMAGE_SCN_MEM_EXECUTE != 0,
            ".linux section should be executable"
        );
        assert!(
            chars & object_pe::IMAGE_SCN_MEM_READ != 0,
            ".linux section should be readable"
        );
    }

    #[test]
    fn data_sections_are_not_executable() {
        // ARRANGE
        let stub = generate_minimal_stub();
        let kernel = fake_kernel(1024);
        let initrd = fake_initrd(1024);
        let cmdline = sample_cmdline();

        // ACT
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
        .expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("should be valid PE");

        let data_sections = pe.section_table().iter().filter(|section| {
            section.name.starts_with(b".cmdline") || section.name.starts_with(b".initrd")
        });

        for section in data_sections {
            let chars = section.characteristics.get(LE);
            assert!(
                chars & object_pe::IMAGE_SCN_MEM_EXECUTE == 0,
                "{:?} section should not be executable",
                core::str::from_utf8(&section.name).unwrap_or("?")
            );
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
        let uki = build(&BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
        .expect("build should succeed");

        // ASSERT
        let pe = PeFile64::parse(&*uki).expect("should be valid PE");
        let subsystem = pe.nt_headers().optional_header.subsystem.get(LE);

        assert_eq!(
            subsystem,
            object_pe::IMAGE_SUBSYSTEM_EFI_APPLICATION,
            "output should be EFI application"
        );
    }

    #[test]
    fn generated_stub_is_valid() {
        // ARRANGE
        let stub = generate_minimal_stub();

        // ACT
        assert!(stub.starts_with(b"MZ"), "should have DOS signature");

        // ASSERT
        let pe = PeFile64::parse(&*stub).expect("generated stub should be valid PE64");

        assert_eq!(pe.section_table().len(), 1, "should have 1 section");

        let section = pe
            .section_table()
            .iter()
            .next()
            .expect("should have section");
        assert!(
            section.name.starts_with(b".text"),
            "section should be .text"
        );
    }

    #[test]
    fn build_rejects_too_many_sections() {
        // ARRANGE
        let stub = generate_stub_with_section_count(u16::MAX - 2);

        // ACT
        let result = build(&BuildInput {
            stub: &stub,
            kernel: b"kernel",
            initramfs: b"initrd",
            cmdline: b"quiet",
            dtb: None,
            luks_key: None,
        });

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::TooManySections)),
            "should return TooManySections, got: {result:?}"
        );
    }
}
