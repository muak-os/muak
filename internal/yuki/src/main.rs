use anyhow::{Context, Result};
use clap::Parser;
use object::LittleEndian as LE;
use object::pe::{ImageFileHeader, ImageSectionHeader};
use object::read::pe::{ImageNtHeaders, PeFile64};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::mem;
use std::path::PathBuf;

mod config;

#[derive(Parser, Debug)]
#[command(name = "yuki")]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
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

    let mut stub_data = Vec::new();
    File::open(&args.stub)
        .with_context(|| format!("Failed to open stub file: {}", args.stub.display()))?
        .read_to_end(&mut stub_data)
        .with_context(|| format!("Failed to read stub file: {}", args.stub.display()))?;

    let linux_data = fs::read(&args.linux)
        .with_context(|| format!("Failed to read linux kernel: {}", args.linux.display()))?;
    let initrd_data = fs::read(&args.initrd)
        .with_context(|| format!("Failed to read initrd: {}", args.initrd.display()))?;
    let cmdline_data = fs::read(&args.cmdline)
        .with_context(|| format!("Failed to read cmdline: {}", args.cmdline.display()))?;

    let original_stub_len = stub_data.len();

    let (
        file_header_offset,
        optional_header_offset,
        section_table_offset,
        section_alignment,
        file_alignment,
        last_section_file_end,
        last_section_virtual_end,
        current_section_count,
    ) = {
        let pe = PeFile64::parse(&stub_data[..]).context("Failed to parse PE file")?;
        let nt_headers = pe.nt_headers();
        let sections = pe.section_table();

        let pe_offset = u32::from_le_bytes([
            stub_data[config::DOS_HEADER_PE_OFFSET],
            stub_data[config::DOS_HEADER_PE_OFFSET + 1],
            stub_data[config::DOS_HEADER_PE_OFFSET + 2],
            stub_data[config::DOS_HEADER_PE_OFFSET + 3],
        ]) as usize;
        let file_header_offset = pe_offset + config::PE_SIGNATURE_SIZE;
        let optional_header_offset = file_header_offset + mem::size_of::<ImageFileHeader>();
        let optional_header_size =
            nt_headers.file_header().size_of_optional_header.get(LE) as usize;
        let section_table_offset = optional_header_offset + optional_header_size;

        let section_alignment = read_u32(
            &stub_data,
            optional_header_offset + config::OPT_HEADER_SECTION_ALIGNMENT,
        );
        let file_alignment = read_u32(
            &stub_data,
            optional_header_offset + config::OPT_HEADER_FILE_ALIGNMENT,
        );

        let last_section_file_end = sections
            .iter()
            .map(|s| s.pointer_to_raw_data.get(LE) + s.size_of_raw_data.get(LE))
            .max()
            .unwrap_or(0);

        let last_section_virtual_end = sections
            .iter()
            .map(|s| {
                s.virtual_address.get(LE) + align_to(s.virtual_size.get(LE), section_alignment)
            })
            .max()
            .unwrap_or(0);

        let current_section_count = nt_headers.file_header().number_of_sections.get(LE);

        (
            file_header_offset,
            optional_header_offset,
            section_table_offset,
            section_alignment,
            file_alignment,
            last_section_file_end,
            last_section_virtual_end,
            current_section_count,
        )
    };

    let sections_to_add: [(&str, &[u8]); 4] = [
        (".cmdline", &cmdline_data),
        (".linux", &linux_data),
        (".initrd", &initrd_data),
        (".stub", &[]),
    ];

    let mut new_sections: Vec<(ImageSectionHeader, usize, usize)> = Vec::new();
    let mut current_file_offset = align_to(last_section_file_end, file_alignment);
    let mut current_virtual_address = align_to(last_section_virtual_end, section_alignment);

    let mut max_virtual_end = last_section_virtual_end;

    for (name, data) in &sections_to_add {
        let is_stub_section = *name == ".stub";
        let data_len = if is_stub_section {
            original_stub_len
        } else {
            data.len()
        };
        let virtual_size = data_len as u32;
        let size_of_raw_data = align_to(virtual_size, file_alignment);

        let mut section = ImageSectionHeader::default();

        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(config::SECTION_NAME_MAX_LEN);
        section.name[..name_len].copy_from_slice(&name_bytes[..name_len]);

        section.virtual_size.set(LE, virtual_size);
        section.virtual_address.set(LE, current_virtual_address);
        section.size_of_raw_data.set(LE, size_of_raw_data);
        section.pointer_to_raw_data.set(LE, current_file_offset);

        let characteristics = if is_stub_section || *name == ".linux" {
            config::IMAGE_SCN_CNT_CODE | config::IMAGE_SCN_MEM_EXECUTE | config::IMAGE_SCN_MEM_READ
        } else {
            config::IMAGE_SCN_MEM_READ
        };
        section.characteristics.set(LE, characteristics);

        max_virtual_end = max_virtual_end
            .max(current_virtual_address + align_to(virtual_size, section_alignment));

        new_sections.push((section, current_file_offset as usize, data_len));
        current_file_offset += size_of_raw_data;
        current_virtual_address += align_to(virtual_size, section_alignment);
    }

    let new_section_count = current_section_count + sections_to_add.len() as u16;
    let section_count_offset = file_header_offset + config::COFF_NUMBER_OF_SECTIONS;

    stub_data.resize(current_file_offset as usize, 0);

    stub_data[section_count_offset..section_count_offset + 2]
        .copy_from_slice(&new_section_count.to_le_bytes());

    for (i, (section_header, _, _)) in new_sections.iter().enumerate() {
        let offset = section_table_offset
            + (current_section_count as usize + i) * mem::size_of::<ImageSectionHeader>();
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                section_header as *const _ as *const u8,
                mem::size_of::<ImageSectionHeader>(),
            )
        };
        stub_data[offset..offset + header_bytes.len()].copy_from_slice(header_bytes);
    }

    for (i, (_, file_offset, data_len)) in new_sections.iter().enumerate() {
        let (name, data) = sections_to_add[i];
        if name == ".stub" {
            stub_data.copy_within(0..original_stub_len, *file_offset);
        } else {
            stub_data[*file_offset..*file_offset + *data_len].copy_from_slice(data);
        }
    }

    let size_of_image_off = optional_header_offset + config::OPT_HEADER_SIZE_OF_IMAGE;
    let new_size_of_image = align_to(max_virtual_end, section_alignment);
    write_u32(&mut stub_data, size_of_image_off, new_size_of_image);

    let mut out_file = File::create(&args.output)
        .with_context(|| format!("Failed to create output file: {}", args.output.display()))?;
    out_file
        .write_all(&stub_data)
        .with_context(|| format!("Failed to write to output file: {}", args.output.display()))?;

    println!(
        "Successfully created UKI at {} ({} bytes)",
        args.output.display(),
        stub_data.len()
    );

    Ok(())
}
