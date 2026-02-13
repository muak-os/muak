//! Integration tests for yuki UKI building.

mod fixtures;

use fixtures::{fake_dtb, fake_initrd, fake_kernel, generate_minimal_stub, sample_cmdline};
use object::LittleEndian as LE;
use object::pe;
use object::read::pe::PeFile64;
use std::fs;
use tempfile::TempDir;

struct TestEnv {
    temp: TempDir,
}

impl TestEnv {
    fn new() -> Self {
        Self {
            temp: TempDir::new().expect("failed to create temp dir"),
        }
    }

    fn write_file(&self, name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = self.temp.path().join(name);
        fs::write(&path, data).unwrap_or_else(|e| panic!("failed to write {}: {}", name, e));
        path
    }
}

#[test]
fn test_build_creates_valid_uki() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(4096));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(8192));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("build should succeed");

    assert_eq!(&uki[0..2], b"MZ", "output should start with MZ");

    let pe = PeFile64::parse(&*uki).expect("output should be valid PE64");

    let section_names: Vec<&[u8]> = pe.section_table().iter().map(|s| &s.name[..]).collect();

    assert!(
        section_names.iter().any(|n| n.starts_with(b".cmdline")),
        "should have .cmdline section, got: {:?}",
        section_names
    );
    assert!(
        section_names.iter().any(|n| n.starts_with(b".linux")),
        "should have .linux section"
    );
    assert!(
        section_names.iter().any(|n| n.starts_with(b".initrd")),
        "should have .initrd section"
    );
}

#[test]
fn test_build_with_dtb() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(4096));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(8192));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());
    let dtb_path = env.write_file("device.dtb", &fake_dtb(1024));

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        Some(&dtb_path),
        None,
    )
    .expect("build with DTB should succeed");

    let pe = PeFile64::parse(&*uki).expect("output should be valid PE64");
    let section_names: Vec<&[u8]> = pe.section_table().iter().map(|s| &s.name[..]).collect();

    assert!(
        section_names.iter().any(|n| n.starts_with(b".dtb")),
        "should have .dtb section, got: {:?}",
        section_names
    );
}

#[test]
fn test_build_preserves_original_sections() {
    let env = TestEnv::new();

    let stub = generate_minimal_stub();
    let original_pe = PeFile64::parse(&*stub).expect("generated stub should be valid PE");
    let original_section_count = original_pe.section_table().len();

    let stub_path = env.write_file("stub.efi", &stub);
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("build should succeed");
    let result_pe = PeFile64::parse(&*uki).expect("output should be valid PE");

    assert_eq!(
        result_pe.section_table().len(),
        original_section_count + 3,
        "should add exactly 3 sections"
    );
}

#[test]
fn test_build_with_large_files() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024 * 1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(2 * 1024 * 1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("build with large files should succeed");

    assert!(uki.len() > 3 * 1024 * 1024);

    PeFile64::parse(&*uki).expect("large UKI should be valid PE64");
}

#[test]
fn test_build_handles_empty_cmdline() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", b"");

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("empty cmdline should be allowed");

    PeFile64::parse(&*uki).expect("should be valid PE");
}

#[test]
fn test_build_rejects_missing_stub() {
    let env = TestEnv::new();

    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let result = yuki::build(
        &env.temp.path().join("nonexistent.efi"),
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    );

    assert!(
        matches!(result, Err(yuki::YukiError::ReadError { .. })),
        "should fail with ReadError for missing stub, got: {:?}",
        result
    );
}

#[test]
fn test_build_rejects_invalid_pe_stub() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", b"this is not a PE file at all");
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let result = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    );

    assert!(
        matches!(result, Err(yuki::YukiError::PeParseError(_))),
        "should fail with PE parse error for invalid stub, got: {:?}",
        result
    );
}

#[test]
fn test_build_rejects_missing_kernel() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let result = yuki::build(
        &stub_path,
        &env.temp.path().join("nonexistent"),
        &initrd_path,
        &cmdline_path,
        None,
        None,
    );

    assert!(
        matches!(result, Err(yuki::YukiError::ReadError { .. })),
        "should fail with ReadError for missing kernel"
    );
}

#[test]
fn test_build_rejects_missing_initrd() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let result = yuki::build(
        &stub_path,
        &kernel_path,
        &env.temp.path().join("nonexistent"),
        &cmdline_path,
        None,
        None,
    );

    assert!(
        matches!(result, Err(yuki::YukiError::ReadError { .. })),
        "should fail with ReadError for missing initrd"
    );
}

#[test]
fn test_build_rejects_missing_cmdline() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));

    let result = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &env.temp.path().join("nonexistent"),
        None,
        None,
    );

    assert!(
        matches!(result, Err(yuki::YukiError::ReadError { .. })),
        "should fail with ReadError for missing cmdline"
    );
}

#[test]
fn test_build_rejects_missing_dtb_when_specified() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let result = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        Some(&env.temp.path().join("nonexistent.dtb")),
        None,
    );

    assert!(
        matches!(result, Err(yuki::YukiError::ReadError { .. })),
        "should fail with ReadError for missing DTB"
    );
}

#[test]
fn test_sections_contain_correct_data() {
    let env = TestEnv::new();

    let kernel_data = fake_kernel(1024);
    let initrd_data = fake_initrd(2048);
    let cmdline_data = sample_cmdline();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &kernel_data);
    let initrd_path = env.write_file("initrd.img", &initrd_data);
    let cmdline_path = env.write_file("cmdline.txt", &cmdline_data);

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("build should succeed");

    let pe = PeFile64::parse(&*uki).expect("should be valid PE");

    for section in pe.section_table().iter() {
        let name = &section.name;
        let offset = section.pointer_to_raw_data.get(LE) as usize;
        let virtual_size = section.virtual_size.get(LE) as usize;

        if name.starts_with(b".linux") {
            let section_data = &uki[offset..offset + virtual_size];
            assert!(
                section_data.starts_with(&kernel_data),
                ".linux section should contain kernel data"
            );
        } else if name.starts_with(b".initrd") {
            let section_data = &uki[offset..offset + virtual_size];
            assert!(
                section_data.starts_with(&initrd_data),
                ".initrd section should contain initrd data"
            );
        } else if name.starts_with(b".cmdline") {
            let section_data = &uki[offset..offset + virtual_size];
            assert!(
                section_data.starts_with(&cmdline_data),
                ".cmdline section should contain cmdline data"
            );
        }
    }
}

#[test]
fn test_linux_section_is_executable() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("build should succeed");

    let pe = PeFile64::parse(&*uki).expect("should be valid PE");

    let linux_section = pe
        .section_table()
        .iter()
        .find(|s| s.name.starts_with(b".linux"))
        .expect("should have .linux section");

    let chars = linux_section.characteristics.get(LE);

    assert!(
        chars & pe::IMAGE_SCN_MEM_EXECUTE != 0,
        ".linux section should be executable"
    );
    assert!(
        chars & pe::IMAGE_SCN_MEM_READ != 0,
        ".linux section should be readable"
    );
}

#[test]
fn test_data_sections_are_not_executable() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("build should succeed");

    let pe = PeFile64::parse(&*uki).expect("should be valid PE");

    for section in pe.section_table().iter() {
        let name = &section.name;
        if name.starts_with(b".cmdline") || name.starts_with(b".initrd") {
            let chars = section.characteristics.get(LE);
            assert!(
                chars & pe::IMAGE_SCN_MEM_EXECUTE == 0,
                "{:?} section should not be executable",
                std::str::from_utf8(name).unwrap_or("?")
            );
        }
    }
}

#[test]
fn test_output_is_efi_application() {
    let env = TestEnv::new();

    let stub_path = env.write_file("stub.efi", &generate_minimal_stub());
    let kernel_path = env.write_file("vmlinuz", &fake_kernel(1024));
    let initrd_path = env.write_file("initrd.img", &fake_initrd(1024));
    let cmdline_path = env.write_file("cmdline.txt", &sample_cmdline());

    let uki = yuki::build(
        &stub_path,
        &kernel_path,
        &initrd_path,
        &cmdline_path,
        None,
        None,
    )
    .expect("build should succeed");

    let pe = PeFile64::parse(&*uki).expect("should be valid PE");
    let subsystem = pe.nt_headers().optional_header.subsystem.get(LE);

    assert_eq!(
        subsystem,
        pe::IMAGE_SUBSYSTEM_EFI_APPLICATION,
        "output should be EFI application"
    );
}

#[test]
fn test_generated_stub_is_valid() {
    let stub = generate_minimal_stub();

    assert_eq!(&stub[0..2], b"MZ", "should have DOS signature");

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
