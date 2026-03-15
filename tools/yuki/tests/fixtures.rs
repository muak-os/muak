//! Test fixtures for yuki integration tests.

use object::pe;

const DOS_HEADER_SIZE: usize = 64;
const PE_SIGNATURE_SIZE: usize = 4;
const COFF_HEADER_SIZE: usize = 20;
const OPTIONAL_HEADER_SIZE: usize = 240;
const SECTION_HEADER_SIZE: usize = 40;
const FILE_ALIGNMENT: usize = 512;
const SECTION_ALIGNMENT: usize = 4096;

/// Generates a minimal valid PE64 EFI stub.
pub fn generate_minimal_stub() -> Vec<u8> {
    let headers_size = DOS_HEADER_SIZE
        + PE_SIGNATURE_SIZE
        + COFF_HEADER_SIZE
        + OPTIONAL_HEADER_SIZE
        + SECTION_HEADER_SIZE;
    let headers_aligned = align_up(headers_size, FILE_ALIGNMENT);
    let section_size = FILE_ALIGNMENT;
    let total_size = headers_aligned + section_size;

    let mut pe = vec![0u8; total_size];

    pe[0] = b'M';
    pe[1] = b'Z';
    let pe_offset = DOS_HEADER_SIZE as u32;
    pe[0x3C..0x40].copy_from_slice(&pe_offset.to_le_bytes());

    let mut offset = DOS_HEADER_SIZE;

    pe[offset..offset + 4].copy_from_slice(b"PE\0\0");
    offset += PE_SIGNATURE_SIZE;

    pe[offset..offset + 2].copy_from_slice(&pe::IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
    offset += 2;
    pe[offset..offset + 2].copy_from_slice(&1u16.to_le_bytes());
    offset += 2;
    offset += 4; // TimeDateStamp
    offset += 4; // PointerToSymbolTable
    offset += 4; // NumberOfSymbols
    pe[offset..offset + 2].copy_from_slice(&(OPTIONAL_HEADER_SIZE as u16).to_le_bytes());
    offset += 2;
    let characteristics: u16 =
        pe::IMAGE_FILE_EXECUTABLE_IMAGE | pe::IMAGE_FILE_LARGE_ADDRESS_AWARE | pe::IMAGE_FILE_DLL;
    pe[offset..offset + 2].copy_from_slice(&characteristics.to_le_bytes());
    offset += 2;

    let opt_offset = offset;
    pe[offset..offset + 2].copy_from_slice(&pe::IMAGE_NT_OPTIONAL_HDR64_MAGIC.to_le_bytes());
    offset += 2;
    offset += 2; // Linker versions
    pe[offset..offset + 4].copy_from_slice(&(section_size as u32).to_le_bytes());
    offset += 4;
    offset += 4; // SizeOfInitializedData
    offset += 4; // SizeOfUninitializedData
    pe[offset..offset + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    offset += 4;
    pe[offset..offset + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    offset += 4;
    pe[offset..offset + 8].copy_from_slice(&0x10000u64.to_le_bytes());
    offset += 8;
    pe[offset..offset + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    offset += 4;
    pe[offset..offset + 4].copy_from_slice(&(FILE_ALIGNMENT as u32).to_le_bytes());
    offset += 4;
    offset += 4; // OS versions
    offset += 4; // Image versions
    offset += 4; // Subsystem versions
    offset += 4; // Win32VersionValue
    let size_of_image = align_up(SECTION_ALIGNMENT + section_size, SECTION_ALIGNMENT) as u32;
    pe[offset..offset + 4].copy_from_slice(&size_of_image.to_le_bytes());
    offset += 4;
    pe[offset..offset + 4].copy_from_slice(&(headers_aligned as u32).to_le_bytes());
    offset += 4;
    offset += 4; // CheckSum
    pe[offset..offset + 2].copy_from_slice(&pe::IMAGE_SUBSYSTEM_EFI_APPLICATION.to_le_bytes());
    offset += 2;
    offset += 2; // DllCharacteristics
    offset += 8; // SizeOfStackReserve
    offset += 8; // SizeOfStackCommit
    offset += 8; // SizeOfHeapReserve
    offset += 8; // SizeOfHeapCommit
    offset += 4; // LoaderFlags
    pe[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());

    let section_header_offset = opt_offset + OPTIONAL_HEADER_SIZE;
    let mut sh_offset = section_header_offset;

    pe[sh_offset..sh_offset + 5].copy_from_slice(b".text");
    sh_offset += 8;
    pe[sh_offset..sh_offset + 4].copy_from_slice(&(section_size as u32).to_le_bytes());
    sh_offset += 4;
    pe[sh_offset..sh_offset + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    sh_offset += 4;
    pe[sh_offset..sh_offset + 4].copy_from_slice(&(section_size as u32).to_le_bytes());
    sh_offset += 4;
    pe[sh_offset..sh_offset + 4].copy_from_slice(&(headers_aligned as u32).to_le_bytes());
    sh_offset += 4;
    sh_offset += 12; // Relocations/Linenumbers
    let section_chars: u32 =
        pe::IMAGE_SCN_CNT_CODE | pe::IMAGE_SCN_MEM_EXECUTE | pe::IMAGE_SCN_MEM_READ;
    pe[sh_offset..sh_offset + 4].copy_from_slice(&section_chars.to_le_bytes());

    pe[headers_aligned] = 0xC3; // RET

    pe
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

fn align_up(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    (value + alignment - 1) & !(alignment - 1)
}
