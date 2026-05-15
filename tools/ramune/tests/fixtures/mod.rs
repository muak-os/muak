#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

const HEADER_LEN: usize = 110;
const MAGIC: &[u8; 6] = b"070701";
const TRAILER: &str = "TRAILER!!!";

#[derive(Debug)]
pub struct ArchiveEntry {
    pub name: String,
    pub mode: u32,
    pub data: Vec<u8>,
}

pub struct TestEnv {
    temp: TempDir,
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TestEnv {
    pub fn new() -> Self {
        Self {
            temp: TempDir::new().expect("failed to create temp dir"),
        }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }

    pub fn write(&self, name: &str, data: &[u8]) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, data).unwrap_or_else(|e| panic!("failed to write {name}: {e}"));
        path
    }

    pub fn write_rootfs(&self) -> PathBuf {
        let rootfs = self.path("rootfs");
        fs::create_dir_all(rootfs.join("sbin")).expect("failed to create rootfs sbin");
        fs::write(rootfs.join("sbin/init"), b"rootfs-init").expect("failed to write rootfs init");
        rootfs
    }

    pub fn write_extension(&self, name: &str, data: &[u8]) -> PathBuf {
        let dir = self.path(name);
        fs::create_dir_all(&dir).expect("failed to create extension dir");
        fs::write(dir.join("payload.txt"), data).expect("failed to write extension payload");
        dir
    }
}

fn align4(offset: usize) -> usize {
    (offset + 3) & !3
}

fn parse_hex(field: &[u8]) -> u32 {
    let field = std::str::from_utf8(field).expect("cpio header field should be utf8");
    u32::from_str_radix(field, 16).expect("cpio header field should be hex")
}

pub fn parse_newc_archive(bytes: &[u8]) -> Vec<ArchiveEntry> {
    let mut offset = 0;
    let mut entries = Vec::new();

    loop {
        assert!(offset + HEADER_LEN <= bytes.len(), "cpio header truncated");

        let header = &bytes[offset..offset + HEADER_LEN];
        assert_eq!(&header[..6], MAGIC, "cpio archive should use newc format");

        let mode = parse_hex(&header[14..22]);
        let filesize = parse_hex(&header[54..62]) as usize;
        let namesize = parse_hex(&header[94..102]) as usize;

        let name_start = offset + HEADER_LEN;
        let name_end = name_start + namesize;
        let name = std::str::from_utf8(&bytes[name_start..name_end - 1])
            .expect("cpio filename should be utf8")
            .to_string();

        let data_start = align4(name_end);
        let data_end = data_start + filesize;
        let data = bytes[data_start..data_end].to_vec();
        offset = align4(data_end);

        if name == TRAILER {
            break;
        }

        entries.push(ArchiveEntry { name, mode, data });
    }

    entries
}

pub fn decode_initramfs(path: &Path) -> Vec<ArchiveEntry> {
    let compressed = fs::read(path).expect("failed to read initramfs");
    let archive = zstd::decode_all(&compressed[..]).expect("failed to decode initramfs");
    parse_newc_archive(&archive)
}

pub fn decode_extension_archive(path: &Path, base_len: usize) -> Vec<ArchiveEntry> {
    let image = fs::read(path).expect("failed to read extended initramfs");
    assert!(
        image.len() > base_len,
        "extended image should contain appended archive"
    );
    let archive = zstd::decode_all(&image[base_len..]).expect("failed to decode extension archive");
    parse_newc_archive(&archive)
}
