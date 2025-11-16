use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use uefi::Result;

use crate::pe::UkiSections;

const CPIO_MAGIC: &str = "070701";
const CPIO_TRAILER: &str = "TRAILER!!!";

struct CpioEntry {
    name: String,
    data: Vec<u8>,
    mode: u32,
}

impl CpioEntry {
    fn new(name: &str, data: &[u8], mode: u32) -> Self {
        Self {
            name: String::from(name),
            data: data.to_vec(),
            mode,
        }
    }

    fn serialize(&self, inode: u32) -> Vec<u8> {
        let mut result = Vec::new();

        let name_with_null = format!("{}\0", self.name);
        let namesize = name_with_null.len();
        let filesize = self.data.len();

        let header = format!(
            "{magic}{inode:08x}{mode:08x}{uid:08x}{gid:08x}{nlink:08x}{mtime:08x}{filesize:08x}{devmajor:08x}{devminor:08x}{rdevmajor:08x}{rdevminor:08x}{namesize:08x}{check:08x}",
            magic = CPIO_MAGIC,
            inode = inode,
            mode = self.mode,
            uid = 0,
            gid = 0,
            nlink = 1,
            mtime = 0,
            filesize = filesize,
            devmajor = 0,
            devminor = 0,
            rdevmajor = 0,
            rdevminor = 0,
            namesize = namesize,
            check = 0,
        );

        result.extend_from_slice(header.as_bytes());
        result.extend_from_slice(name_with_null.as_bytes());

        let header_and_name_len = 110 + namesize;
        let padding = (4 - (header_and_name_len % 4)) % 4;
        result.extend(core::iter::repeat_n(0, padding));

        result.extend_from_slice(&self.data);

        let data_padding = (4 - (filesize % 4)) % 4;
        result.extend(core::iter::repeat_n(0, data_padding));

        result
    }
}

pub fn build_enhanced_initrd(sections: &UkiSections) -> Result<Vec<u8>> {
    // The Linux kernel supports multiple concatenated cpio archives.
    // Since the original initrd is compressed (xz), we need to PREPEND
    // an uncompressed cpio archive with our files BEFORE the compressed one.
    // The kernel will extract both in order.

    let mut result = Vec::new();
    let mut inode = 1u32;

    let root_dir = CpioEntry::new(".", &[], 0o40755);
    result.extend_from_slice(&root_dir.serialize(inode));
    inode += 1;

    let run_dir = CpioEntry::new("run", &[], 0o40755); // Directory with 0755 perms
    result.extend_from_slice(&run_dir.serialize(inode));
    inode += 1;

    let uki_dir = CpioEntry::new("run/uki", &[], 0o40755); // Directory with 0755 perms
    result.extend_from_slice(&uki_dir.serialize(inode));
    inode += 1;

    info!("Adding /run/uki/bzImage ({} bytes)", sections.kernel.len());
    let kernel_entry = CpioEntry::new("run/uki/bzImage", sections.kernel, 0o100644); // Regular file
    result.extend_from_slice(&kernel_entry.serialize(inode));
    inode += 1;

    info!(
        "Adding /run/uki/cmdline.txt ({} bytes)",
        sections.cmdline.len()
    );
    let cmdline_entry = CpioEntry::new("run/uki/cmdline.txt", sections.cmdline, 0o100644);
    result.extend_from_slice(&cmdline_entry.serialize(inode));
    inode += 1;

    info!(
        "Adding /run/uki/initrd.img ({} bytes)",
        sections.initrd.len()
    );
    let initrd_entry = CpioEntry::new("run/uki/initrd.img", sections.initrd, 0o100644);
    result.extend_from_slice(&initrd_entry.serialize(inode));
    inode += 1;

    if let Some(stub_data) = sections.stub {
        info!("Adding /run/uki/muak-stub.efi ({} bytes)", stub_data.len());
        let stub_entry = CpioEntry::new("run/uki/stub.efi", stub_data, 0o100644);
        result.extend_from_slice(&stub_entry.serialize(inode));
    } else {
        warn!(".stub section not found");
    }

    let trailer = CpioEntry::new(CPIO_TRAILER, &[], 0);
    result.extend_from_slice(&trailer.serialize(0));

    info!(
        "Prepended uncompressed cpio archive: {} bytes",
        result.len()
    );

    result.extend_from_slice(sections.initrd);

    info!("Enhanced initrd built: {} bytes total", result.len());

    Ok(result)
}
