use anyhow::{Context, Result};
use clap::Parser;
use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;
use object::read::pe::{ImageNtHeaders, PeFile64};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::mem;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "yuki")]
#[command(about = "Static UKI builder - adds PE sections to EFI stubs", long_about = None)]
struct Args {
    #[arg(short, long)]
    stub: PathBuf,

    #[arg(short, long)]
    linux: PathBuf,

    #[arg(short, long)]
    initrd: PathBuf,

    #[arg(short, long)]
    cmdline: PathBuf,

    #[arg(short, long)]
    output: PathBuf,
}

// PE section characteristics
const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;

fn align_to(value: u32, alignment: u32) -> u32 {
    if alignment == 0 {
        return value;
    }
    (value + alignment - 1) & !(alignment - 1)
}

fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn write_u32(buf: &mut [u8], off: usize, val: u32) {
    buf[off..off + 4].copy_from_slice(&val.to_le_bytes());
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Read the stub binary
    let mut stub_data = Vec::new();
    File::open(&args.stub)
        .context("Failed to open stub file")?
        .read_to_end(&mut stub_data)
        .context("Failed to read stub file")?;

    let linux_data = fs::read(&args.linux).context("Failed to read Linux kernel")?;
    let initrd_data = fs::read(&args.initrd).context("Failed to read initrd")?;
    let cmdline_data = fs::read(&args.cmdline).context("Failed to read cmdline")?;

    // Parse PE
    let pe = PeFile64::parse(&stub_data[..]).context("Failed to parse PE file")?;
    let nt_headers = pe.nt_headers();
    let sections = pe.section_table();

    // COFF + Optional header offsets
    let pe_offset = u32::from_le_bytes([
        stub_data[0x3c],
        stub_data[0x3d],
        stub_data[0x3e],
        stub_data[0x3f],
    ]) as usize;
    let file_header_offset = pe_offset + 4; // skip PE signature
    let optional_header_offset = file_header_offset + mem::size_of::<object::pe::ImageFileHeader>();
    let optional_header_size = nt_headers.file_header().size_of_optional_header.get(LE) as usize;
    let section_table_offset = optional_header_offset + optional_header_size;

    // Read alignments from Optional Header (PE32+)
    let section_alignment = read_u32(&stub_data, optional_header_offset + 32);
    let file_alignment = read_u32(&stub_data, optional_header_offset + 36);

    // Determine last section ends
    let last_section_file_end = sections
        .iter()
        .map(|s| s.pointer_to_raw_data.get(LE) + s.size_of_raw_data.get(LE))
        .max()
        .unwrap_or(0);

    let last_section_virtual_end = sections
        .iter()
        .map(|s| s.virtual_address.get(LE) + align_to(s.virtual_size.get(LE), section_alignment))
        .max()
        .unwrap_or(0);

    let sections_to_add = vec![
        (".cmdline", cmdline_data),
        (".linux", linux_data),
        (".initrd", initrd_data),
    ];

    // Prepare new sections with proper alignments and flags
    let mut new_sections = Vec::new();
    let mut current_file_offset = align_to(last_section_file_end, file_alignment);
    let mut current_virtual_address = align_to(last_section_virtual_end, section_alignment);
    let current_section_count = nt_headers.file_header().number_of_sections.get(LE);

    let mut max_virtual_end = last_section_virtual_end;

    for (name, data) in &sections_to_add {
        let virtual_size = data.len() as u32;
        let size_of_raw_data = align_to(virtual_size, file_alignment);

        let mut section = ImageSectionHeader::default();

        // name (max 8 bytes)
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(8);
        section.name[..name_len].copy_from_slice(&name_bytes[..name_len]);

        section.virtual_size.set(LE, virtual_size);
        section.virtual_address.set(LE, current_virtual_address);
        section.size_of_raw_data.set(LE, size_of_raw_data);
        section.pointer_to_raw_data.set(LE, current_file_offset);

        // Match flags:
        // - .cmdline: alloc,readonly
        // - .linux:   alloc,readonly,code
        // - .initrd:  alloc,readonly
        let characteristics = match *name {
            ".linux" => IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ,
            _ => IMAGE_SCN_MEM_READ,
        };
        section.characteristics.set(LE, characteristics);

        max_virtual_end = max_virtual_end
            .max(current_virtual_address + align_to(virtual_size, section_alignment));

        new_sections.push((section, data.clone()));
        current_file_offset = current_file_offset + size_of_raw_data;
        current_virtual_address =
            current_virtual_address + align_to(virtual_size, section_alignment);
    }

    let mut output = stub_data.clone();

    // Update NumberOfSections in COFF header (offset +2 in file header)
    let new_section_count = current_section_count + sections_to_add.len() as u16;
    let section_count_offset = file_header_offset + 2;
    output[section_count_offset..section_count_offset + 2]
        .copy_from_slice(&new_section_count.to_le_bytes());

    output.resize(current_file_offset as usize, 0);

    // Write new section headers into section table
    for (i, (section_header, _)) in new_sections.iter().enumerate() {
        let offset = section_table_offset
            + (current_section_count as usize + i) * mem::size_of::<ImageSectionHeader>();
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                section_header as *const _ as *const u8,
                mem::size_of::<ImageSectionHeader>(),
            )
        };
        output[offset..offset + header_bytes.len()].copy_from_slice(header_bytes);
    }

    // Write section data
    for (section_header, data) in &new_sections {
        let off = section_header.pointer_to_raw_data.get(LE) as usize;
        output[off..off + data.len()].copy_from_slice(data);
    }

    // Update SizeOfImage in Optional Header to cover new sections
    let size_of_image_off = optional_header_offset + 56; // DWORD SizeOfImage
    let new_size_of_image = align_to(max_virtual_end, section_alignment);
    write_u32(&mut output, size_of_image_off, new_size_of_image);

    let mut out_file = File::create(&args.output).context("Failed to create output file")?;
    out_file
        .write_all(&output)
        .context("Failed to write output file")?;

    println!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        output.len()
    );

    Ok(())
}
