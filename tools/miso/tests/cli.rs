//! CLI integration tests for the `miso` binary.

use std::fs;
use std::process::Command;

use tempfile::TempDir;

fn miso_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_miso"))
}

fn fake_uki(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("test.efi");
    let mut data = vec![0u8; 4096];
    data[0] = b'M';
    data[1] = b'Z';
    fs::write(&path, &data).expect("write fake UKI");
    path
}

#[test]
fn iso_subcommand_produces_valid_output() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.iso");

    // ACT
    let status = miso_bin()
        .args([
            "iso",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(status.success(), "miso iso must exit 0");
    let bytes = fs::read(&output).expect("read output iso");
    assert!(!bytes.is_empty(), "output ISO must not be empty");
}

#[test]
fn raw_subcommand_produces_valid_output() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.raw");

    // ACT
    let status = miso_bin()
        .args([
            "raw",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(status.success(), "miso raw must exit 0");
    let bytes = fs::read(&output).expect("read output raw image");
    assert!(!bytes.is_empty(), "output RAW must not be empty");
}

#[test]
fn raw_subcommand_with_compression_produces_zstd_output() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.raw.zst");

    // ACT
    let status = miso_bin()
        .args([
            "raw",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--compression-level",
            "3",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(status.success(), "miso raw with compression must exit 0");
    let bytes = fs::read(&output).expect("read compressed raw output");
    let raw = zstd::decode_all(&bytes[..]).expect("decode zstd output");
    let mut cursor = std::io::Cursor::new(raw);
    let gpt = gptman::GPT::find_from(&mut cursor).expect("valid GPT");
    assert!(gpt.iter().any(|(_, p)| p.is_used()));
}

#[test]
fn raw_subcommand_with_invalid_compression_level_exits_nonzero() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.raw.zst");

    // ACT
    let status = miso_bin()
        .args([
            "raw",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--compression-level",
            "999999",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(
        !status.success(),
        "invalid compression level must exit non-zero"
    );
}

#[test]
fn iso_subcommand_with_explicit_arch_x86_64() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.iso");

    // ACT
    let status = miso_bin()
        .args([
            "iso",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--arch",
            "x86_64",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(status.success());
}

#[test]
fn raw_subcommand_with_explicit_arch_aarch64() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.raw");

    // ACT
    let status = miso_bin()
        .args([
            "raw",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--arch",
            "aarch64",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(status.success());
}

#[test]
fn iso_subcommand_with_extra_file() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let extra = dir.path().join("config.txt");
    fs::write(&extra, b"arm_64bit=1").expect("write extra file");
    let output = dir.path().join("out.iso");
    let file_spec = format!("{}:overlays/config.txt", extra.to_str().unwrap());

    // ACT
    let status = miso_bin()
        .args([
            "iso",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--file",
            &file_spec,
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(status.success());
}

#[test]
fn raw_subcommand_with_extra_file() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let extra = dir.path().join("blob.dat");
    fs::write(&extra, b"firmware").expect("write extra file");
    let output = dir.path().join("out.raw");
    let file_spec = format!("{}:firmware/blob.dat", extra.to_str().unwrap());

    // ACT
    let status = miso_bin()
        .args([
            "raw",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--file",
            &file_spec,
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(status.success());
}

#[test]
fn iso_unsupported_arch_exits_nonzero() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.iso");

    // ACT
    let status = miso_bin()
        .args([
            "iso",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--arch",
            "riscv64",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(!status.success(), "unsupported arch must exit non-zero");
}

#[test]
fn raw_unsupported_arch_exits_nonzero() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.raw");

    // ACT
    let status = miso_bin()
        .args([
            "raw",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--arch",
            "riscv64",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(!status.success(), "unsupported arch must exit non-zero");
}

#[test]
fn missing_uki_file_exits_nonzero() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let output = dir.path().join("out.iso");

    // ACT
    let status = miso_bin()
        .args([
            "iso",
            "--uki",
            "/nonexistent/path/uki.efi",
            "--output",
            output.to_str().unwrap(),
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(!status.success(), "missing UKI must exit non-zero");
}

#[test]
fn invalid_file_spec_exits_nonzero() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.iso");

    // ACT
    let status = miso_bin()
        .args([
            "iso",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--file",
            "no-colon-here",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(!status.success(), "invalid file spec must exit non-zero");
}

#[test]
fn missing_extra_file_exits_nonzero() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.iso");

    // ACT
    let status = miso_bin()
        .args([
            "iso",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--file",
            "/nonexistent/src.dat:dst/path.dat",
        ])
        .status()
        .expect("failed to run miso");

    // ASSERT
    assert!(!status.success(), "missing extra file must exit non-zero");
}

#[test]
fn iso_output_has_cd001_magic() {
    // ARRANGE
    let dir = TempDir::new().expect("tempdir");
    let uki = fake_uki(&dir);
    let output = dir.path().join("out.iso");

    miso_bin()
        .args([
            "iso",
            "--uki",
            uki.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .status()
        .expect("run miso");

    // ACT
    let bytes = fs::read(&output).expect("read iso");

    // ASSERT
    let offset = 2048 * 16 + 1;
    assert_eq!(
        &bytes[offset..offset + 5],
        b"CD001",
        "ISO 9660 magic must be present"
    );
}
