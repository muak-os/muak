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

        // Build CPIO header (110 bytes)
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

        // Align to 4-byte boundary after name
        let header_and_name_len = 110 + namesize;
        let padding = (4 - (header_and_name_len % 4)) % 4;
        for _ in 0..padding {
            result.push(0);
        }

        // Add file data
        result.extend_from_slice(&self.data);

        // Align to 4-byte boundary after data
        let data_padding = (4 - (filesize % 4)) % 4;
        for _ in 0..data_padding {
            result.push(0);
        }

        result
    }
}

pub fn build_enhanced_initrd(sections: &UkiSections) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut inode = 1u32;

    // First, copy the entire original initrd
    // The original initrd is already a complete CPIO archive
    result.extend_from_slice(sections.initrd);

    // Remove the TRAILER!!! from the original initrd so we can append to it
    // Find and remove the last TRAILER entry
    if let Some(trailer_pos) = find_trailer(&result) {
        result.truncate(trailer_pos);
        log::info!("Removed original TRAILER at offset {}", trailer_pos);
    } else {
        log::warn!("Could not find TRAILER in original initrd, continuing anyway");
    }

    // Calculate starting inode number (use a high number to avoid conflicts)
    inode = 100000;

    // Create /uki directory entry
    let uki_dir = CpioEntry::new("uki", &[], 0o40755); // Directory with 0755 perms
    result.extend_from_slice(&uki_dir.serialize(inode));
    inode += 1;

    // Add /uki/kernel
    log::info!("Adding /uki/kernel ({} bytes)", sections.kernel.len());
    let kernel_entry = CpioEntry::new("uki/kernel", sections.kernel, 0o100644); // Regular file
    result.extend_from_slice(&kernel_entry.serialize(inode));
    inode += 1;

    // Add /uki/cmdline.txt
    log::info!("Adding /uki/cmdline.txt ({} bytes)", sections.cmdline.len());
    let cmdline_entry = CpioEntry::new("uki/cmdline.txt", sections.cmdline, 0o100644);
    result.extend_from_slice(&cmdline_entry.serialize(inode));
    inode += 1;

    // Add /uki/initrd.img (the original compressed initrd)
    log::info!("Adding /uki/initrd.img ({} bytes)", sections.initrd.len());
    let initrd_entry = CpioEntry::new("uki/initrd.img", sections.initrd, 0o100644);
    result.extend_from_slice(&initrd_entry.serialize(inode));
    inode += 1;

    // Add TRAILER!!!
    let trailer = CpioEntry::new(CPIO_TRAILER, &[], 0);
    result.extend_from_slice(&trailer.serialize(0));

    log::info!("Enhanced initrd built: {} bytes total", result.len());

    Ok(result)
}

fn find_trailer(data: &[u8]) -> Option<usize> {
    // Search for the TRAILER!!! entry in the CPIO archive
    // Look for the pattern "070701" followed by zeros and then "TRAILER!!!"

    for i in 0..data.len().saturating_sub(110 + CPIO_TRAILER.len()) {
        if &data[i..i + 6] == CPIO_MAGIC.as_bytes() {
            // Found a potential header, check if it's the trailer
            // Name starts at offset 110 from magic
            let name_start = i + 110;
            if name_start + CPIO_TRAILER.len() < data.len() {
                if &data[name_start..name_start + CPIO_TRAILER.len()] == CPIO_TRAILER.as_bytes() {
                    return Some(i);
                }
            }
        }
    }

    None
}
