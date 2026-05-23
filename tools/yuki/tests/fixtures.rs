//! Test fixtures for yuki integration tests.

use object::pe;

const DOS_HEADER_SIZE: usize = 64;
const PE_SIGNATURE_SIZE: usize = 4;
const COFF_HEADER_SIZE: usize = 20;
const OPTIONAL_HEADER_SIZE: usize = 240;
const SECTION_HEADER_SIZE: usize = 40;
const FILE_ALIGNMENT: usize = 512;
const SECTION_ALIGNMENT: usize = 4096;
const EXTRA_SECTION_HEADER_SLOTS: usize = 4;

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

/// Writes the PE signature, COFF header, optional header, and section headers into the provided buffer.
fn write_pe_headers(buf: &mut [u8], coff_offset: usize, num_sections: u16) {
    let mut off = coff_offset;
    buf[off..off + 4].copy_from_slice(b"PE\0\0");
    off += PE_SIGNATURE_SIZE;

    buf[off..off + 2].copy_from_slice(&pe::IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
    off += 2;
    buf[off..off + 2].copy_from_slice(&num_sections.to_le_bytes());
    off += 2;
    off += 12;
    buf[off..off + 2].copy_from_slice(&(OPTIONAL_HEADER_SIZE as u16).to_le_bytes());
    off += 2;
    let characteristics: u16 =
        pe::IMAGE_FILE_EXECUTABLE_IMAGE | pe::IMAGE_FILE_LARGE_ADDRESS_AWARE | pe::IMAGE_FILE_DLL;
    buf[off..off + 2].copy_from_slice(&characteristics.to_le_bytes());
    off += 2;

    let opt_off = off;
    buf[off..off + 2].copy_from_slice(&pe::IMAGE_NT_OPTIONAL_HDR64_MAGIC.to_le_bytes());
    off += 2;
    off += 2;
    let section_size = FILE_ALIGNMENT;
    buf[off..off + 4].copy_from_slice(&(section_size as u32).to_le_bytes());
    off += 4;
    off += 8;
    buf[off..off + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    off += 4;
    buf[off..off + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    off += 4;
    buf[off..off + 8].copy_from_slice(&0x10000u64.to_le_bytes());
    off += 8;
    buf[off..off + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    off += 4;
    buf[off..off + 4].copy_from_slice(&(FILE_ALIGNMENT as u32).to_le_bytes());
    off += 4;
    off += 16;
    let size_of_image = (SECTION_ALIGNMENT * 2) as u32;
    buf[off..off + 4].copy_from_slice(&size_of_image.to_le_bytes());
    off += 4;
    let headers_aligned = ((opt_off
        + OPTIONAL_HEADER_SIZE
        + (usize::from(num_sections) + EXTRA_SECTION_HEADER_SLOTS) * SECTION_HEADER_SIZE
        + FILE_ALIGNMENT
        - 1)
        & !(FILE_ALIGNMENT - 1)) as u32;
    buf[off..off + 4].copy_from_slice(&headers_aligned.to_le_bytes());
    off += 4;
    off += 4;
    buf[off..off + 2].copy_from_slice(&pe::IMAGE_SUBSYSTEM_EFI_APPLICATION.to_le_bytes());
    off += 2;
    off += 2 + 8 + 8 + 8 + 8 + 4;
    buf[off..off + 4].copy_from_slice(&0u32.to_le_bytes());

    let sh_base = opt_off + OPTIONAL_HEADER_SIZE;
    buf[sh_base..sh_base + 5].copy_from_slice(b".text");
    buf[sh_base + 8..sh_base + 12].copy_from_slice(&(section_size as u32).to_le_bytes());
    buf[sh_base + 12..sh_base + 16].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    buf[sh_base + 16..sh_base + 20].copy_from_slice(&(section_size as u32).to_le_bytes());
    let section_rva = ((opt_off + OPTIONAL_HEADER_SIZE + SECTION_HEADER_SIZE + FILE_ALIGNMENT - 1)
        & !(FILE_ALIGNMENT - 1)) as u32;
    buf[sh_base + 20..sh_base + 24].copy_from_slice(&section_rva.to_le_bytes());
    buf[sh_base + 36..sh_base + 40].copy_from_slice(&0x60000020u32.to_le_bytes());

    for i in 1..num_sections as usize {
        let sh = sh_base + i * SECTION_HEADER_SIZE;
        buf[sh..sh + 5].copy_from_slice(b".null");
        buf[sh + 8..sh + 12].copy_from_slice(&1u32.to_le_bytes());
        let virt = ((i + 1) * SECTION_ALIGNMENT) as u32;
        buf[sh + 12..sh + 16].copy_from_slice(&virt.to_le_bytes());
    }
}

/// Generates a minimal valid PE64 EFI stub.
pub fn generate_minimal_stub() -> Vec<u8> {
    generate_stub_with_section_count(1)
}

/// Generates a fake Linux kernel image.
pub fn fake_kernel(size: usize) -> Vec<u8> {
    let mut kernel = Vec::with_capacity(size);
    kernel.extend_from_slice(b"KERNEL_MAGIC");
    while kernel.len() < size {
        kernel.push((kernel.len() % 256) as u8);
    }
    kernel.truncate(size);
    kernel
}

/// Generates a fake initrd image with gzip magic.
pub fn fake_initrd(size: usize) -> Vec<u8> {
    let mut initrd = Vec::with_capacity(size);
    initrd.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00]);
    while initrd.len() < size {
        initrd.push(0xAA);
    }
    initrd.truncate(size);
    initrd
}

/// Generates a sample kernel command line.
pub fn sample_cmdline() -> Vec<u8> {
    b"console=ttyS0 quiet".to_vec()
}

/// Generates a fake Device Tree Blob with FDT magic.
pub fn fake_dtb(size: usize) -> Vec<u8> {
    let mut dtb = Vec::with_capacity(size);
    dtb.extend_from_slice(&[0xd0, 0x0d, 0xfe, 0xed]);
    dtb.extend_from_slice(&[0x00, 0x00, 0x00, 0x11]);
    while dtb.len() < size {
        dtb.push(0x00);
    }
    dtb.truncate(size);
    dtb
}

/// Generates a minimal valid PE64 EFI stub with `n` sections declared in the COFF header.
pub fn generate_stub_with_section_count(n: u16) -> Vec<u8> {
    let section_table_size = SECTION_HEADER_SIZE * (usize::from(n) + EXTRA_SECTION_HEADER_SLOTS);
    let headers_raw = DOS_HEADER_SIZE
        + PE_SIGNATURE_SIZE
        + COFF_HEADER_SIZE
        + OPTIONAL_HEADER_SIZE
        + section_table_size;
    let headers_aligned = align_up(headers_raw, FILE_ALIGNMENT);
    let total_size = headers_aligned + FILE_ALIGNMENT;

    let mut pe = vec![0u8; total_size];
    pe[0] = b'M';
    pe[1] = b'Z';
    pe[0x3C..0x40].copy_from_slice(&(DOS_HEADER_SIZE as u32).to_le_bytes());

    write_pe_headers(&mut pe, DOS_HEADER_SIZE, n);
    pe[headers_aligned] = 0xC3;

    pe
}
