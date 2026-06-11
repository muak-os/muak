use core::str;
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
        fs::write(&path, data).expect("failed to write fixture file");
        path
    }

    pub fn write_rootfs(&self) -> PathBuf {
        let rootfs = self.path("rootfs");
        fs::create_dir_all(rootfs.join("sbin")).expect("failed to create rootfs sbin");
        fs::write(rootfs.join("sbin/init"), b"rootfs-init").expect("failed to write rootfs init");
        rootfs
    }
}

fn parse_hex(field: &[u8]) -> u32 {
    let field = str::from_utf8(field).expect("cpio header field should be utf8");
    u32::from_str_radix(field, 16).expect("cpio header field should be hex")
}

pub fn parse_newc_archive(bytes: &[u8]) -> Vec<ArchiveEntry> {
    let mut offset = 0_usize;
    let mut entries = Vec::new();

    loop {
        let header_end = offset
            .checked_add(HEADER_LEN)
            .expect("cpio header offset should not overflow");
        assert!(header_end <= bytes.len(), "cpio header truncated");

        let header = bytes.get(offset..header_end).expect("cpio header exists");
        assert_eq!(
            header.get(..MAGIC.len()).expect("cpio magic exists"),
            MAGIC,
            "cpio archive should use newc format"
        );

        let mode = parse_hex(header.get(14..22).expect("cpio mode field exists"));
        let filesize = usize::try_from(parse_hex(
            header.get(54..62).expect("cpio filesize field exists"),
        ))
        .expect("cpio filesize should fit usize");
        let namesize = usize::try_from(parse_hex(
            header.get(94..102).expect("cpio namesize field exists"),
        ))
        .expect("cpio namesize should fit usize");

        let name_start = header_end;
        let name_end = name_start
            .checked_add(namesize)
            .expect("cpio filename end should not overflow");
        let name_without_nul = name_end
            .checked_sub(1)
            .expect("cpio filename should include trailing nul");
        let name = str::from_utf8(
            bytes
                .get(name_start..name_without_nul)
                .expect("cpio filename exists"),
        )
        .expect("cpio filename should be utf8")
        .to_owned();

        let data_start = name_end.next_multiple_of(4);
        let data_end = data_start
            .checked_add(filesize)
            .expect("cpio data end should not overflow");
        let data = bytes
            .get(data_start..data_end)
            .expect("cpio file data exists")
            .to_vec();
        offset = data_end.next_multiple_of(4);

        if name == TRAILER {
            break;
        }

        entries.push(ArchiveEntry { name, mode, data });
    }

    entries
}

pub fn decode_initramfs(path: &Path) -> Vec<ArchiveEntry> {
    let compressed = fs::read(path).expect("failed to read initramfs");
    let archive = zstd::decode_all(compressed.as_slice()).expect("failed to decode initramfs");
    parse_newc_archive(&archive)
}
