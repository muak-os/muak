#![no_std]
#![no_main]

extern crate alloc;

use core::ffi::c_void;
use core::mem;
use core::ptr;
use core::slice;
use uefi::Guid;
use uefi::boot::{self, LoadImageSource, MemoryType, OpenProtocolAttributes, OpenProtocolParams};
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;

mod log;

// --- GUID Definitions ---
const LINUX_EFI_INITRD_MEDIA_GUID: Guid =
    Guid::parse_or_panic("5568e427-68fc-4f3d-ac74-ca555231cc68");
const LOAD_FILE2_PROTOCOL_GUID: Guid = Guid::parse_or_panic("4006c0c1-fcb3-403e-996d-4a6c8724e06d");
const DEVICE_PATH_PROTOCOL_GUID: Guid =
    Guid::parse_or_panic("09576e91-6d3f-11d2-8e39-00a0c969723b");

// --- LoadFile2 Protocol Definition ---
#[repr(C)]
struct LoadFile2 {
    load_file: unsafe extern "efiapi" fn(
        this: *mut LoadFile2,
        file_path: *const c_void,
        boot_policy: bool,
        buffer_size: *mut usize,
        buffer: *mut u8,
    ) -> Status,
}

// Initrd data pointer stored globally for LoadFile2 callback
// Using raw pointer + length instead of Option<&[u8]> for simpler statics
static mut INITRD_PTR: *const u8 = ptr::null();
static mut INITRD_LEN: usize = 0;

unsafe extern "efiapi" fn load_file2_callback(
    _this: *mut LoadFile2,
    _file_path: *const c_void,
    boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> Status {
    info!("[LoadFile2] Callback invoked, boot_policy={}", boot_policy);

    // Per UEFI spec, LoadFile2 should reject boot_policy=true
    if boot_policy {
        warn!("[LoadFile2] Rejecting boot_policy=true");
        return Status::UNSUPPORTED;
    }

    let data_ptr = unsafe { INITRD_PTR };
    let data_len = unsafe { INITRD_LEN };

    if data_ptr.is_null() || data_len == 0 {
        error!("[LoadFile2] No initrd data available");
        return Status::NOT_FOUND;
    }

    if buffer_size.is_null() {
        error!("[LoadFile2] buffer_size is null");
        return Status::INVALID_PARAMETER;
    }

    let available_size = unsafe { *buffer_size };
    unsafe { *buffer_size = data_len };

    // First call: kernel queries the size
    if buffer.is_null() || available_size < data_len {
        info!("[LoadFile2] Returning size: {} bytes", data_len);
        return Status::BUFFER_TOO_SMALL;
    }

    // Second call: copy the data
    info!(
        "[LoadFile2] Copying {} bytes to buffer {:p}",
        data_len, buffer
    );
    unsafe {
        ptr::copy_nonoverlapping(data_ptr, buffer, data_len);
    }

    info!("[LoadFile2] Copy complete, returning SUCCESS");
    Status::SUCCESS
}

// --- Minimal PE Definitions ---
#[repr(C)]
struct ImageDosHeader {
    e_magic: u16,
    _unused: [u16; 29],
    e_lfanew: u32,
}

#[repr(C)]
struct ImageFileHeader {
    machine: u16,
    number_of_sections: u16,
    _unused: [u8; 12],
    size_of_optional_header: u16,
    _unused2: u16,
}

#[repr(C)]
struct ImageSectionHeader {
    name: [u8; 8],
    virtual_size: u32,
    virtual_address: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
    _unused: [u8; 12],
    characteristics: u32,
}

fn section_name_equals(header: &ImageSectionHeader, name: &[u8]) -> bool {
    let header_name = &header.name;
    for i in 0..8 {
        let c = header_name[i];
        let match_c = if i < name.len() { name[i] } else { 0 };
        if c != match_c {
            return false;
        }
    }
    true
}

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();
    info!("Muak stub v0.2.0 starting...");

    let image_handle = boot::image_handle();

    // 1. Get Base Address
    let loaded_image = unsafe {
        boot::open_protocol::<LoadedImage>(
            OpenProtocolParams {
                handle: image_handle,
                agent: image_handle,
                controller: None,
            },
            OpenProtocolAttributes::GetProtocol,
        )
        .expect("Failed to open LoadedImage protocol")
    };

    let (base_addr, _image_size) = loaded_image.info();
    info!("Base address: {:p}", base_addr);

    // 2. Parse PE Sections
    let mut linux_section: Option<&'static [u8]> = None;
    let mut initrd_section: Option<&'static [u8]> = None;
    let mut cmdline_section: Option<&'static [u8]> = None;

    unsafe {
        let dos_header = &*(base_addr as *const ImageDosHeader);
        if dos_header.e_magic != 0x5A4D {
            error!("Invalid DOS header magic");
            return Status::LOAD_ERROR;
        }

        let pe_header_ptr = (base_addr as *const u8).add(dos_header.e_lfanew as usize);
        let file_header_ptr = pe_header_ptr.add(4) as *const ImageFileHeader;
        let file_header = &*file_header_ptr;

        let section_headers_ptr = (file_header_ptr as *const u8)
            .add(mem::size_of::<ImageFileHeader>())
            .add(file_header.size_of_optional_header as usize)
            as *const ImageSectionHeader;

        let sections =
            slice::from_raw_parts(section_headers_ptr, file_header.number_of_sections as usize);

        for section in sections {
            let sec_start = (base_addr as *const u8).add(section.virtual_address as usize);
            let sec_size = section.virtual_size as usize;
            let sec_data = slice::from_raw_parts(sec_start, sec_size);

            if section_name_equals(section, b".linux") {
                linux_section = Some(sec_data);
            } else if section_name_equals(section, b".initrd") {
                initrd_section = Some(sec_data);
            } else if section_name_equals(section, b".cmdline") {
                cmdline_section = Some(sec_data);
            }
        }
    }

    let kernel_bytes = match linux_section {
        Some(k) => k,
        None => {
            error!("No .linux section found!");
            return Status::NOT_FOUND;
        }
    };

    info!(
        "Kernel: {} bytes at {:p}",
        kernel_bytes.len(),
        kernel_bytes.as_ptr()
    );

    // 3. Install LoadFile2 Protocol for Initrd
    if let Some(initrd_bytes) = initrd_section {
        info!(
            "Installing LoadFile2 for Initrd ({} bytes at {:p})...",
            initrd_bytes.len(),
            initrd_bytes.as_ptr()
        );

        unsafe {
            // Store initrd location globally
            INITRD_PTR = initrd_bytes.as_ptr();
            INITRD_LEN = initrd_bytes.len();

            // Allocate LoadFile2 protocol instance from pool (not static)
            let load_file2_ptr =
                boot::allocate_pool(MemoryType::BOOT_SERVICES_DATA, mem::size_of::<LoadFile2>())
                    .expect("Failed to allocate LoadFile2 instance")
                    .as_ptr() as *mut LoadFile2;

            // Initialize the protocol
            (*load_file2_ptr).load_file = load_file2_callback;

            // Build Device Path manually
            let mut dp_bytes: [u8; 24] = [0; 24];
            // Type 4 (Media), SubType 3 (Vendor), Length 20
            dp_bytes[0] = 4;
            dp_bytes[1] = 3;
            dp_bytes[2] = 20;
            dp_bytes[3] = 0;

            // Copy GUID
            let guid_bytes = LINUX_EFI_INITRD_MEDIA_GUID.to_bytes();
            ptr::copy_nonoverlapping(guid_bytes.as_ptr(), dp_bytes.as_mut_ptr().add(4), 16);

            // End Node: Type 0x7F, SubType 0xFF, Length 4
            dp_bytes[20] = 0x7F;
            dp_bytes[21] = 0xFF;
            dp_bytes[22] = 4;
            dp_bytes[23] = 0;

            let dp_ptr = boot::allocate_pool(MemoryType::BOOT_SERVICES_DATA, 24)
                .expect("Failed to allocate device path")
                .as_ptr();
            ptr::copy_nonoverlapping(dp_bytes.as_ptr(), dp_ptr, 24);

            // Install DevicePath first, then LoadFile2 on the same handle
            let initrd_handle = boot::install_protocol_interface(
                None,
                &DEVICE_PATH_PROTOCOL_GUID,
                dp_ptr as *const c_void,
            )
            .expect("Failed to install DevicePath");

            boot::install_protocol_interface(
                Some(initrd_handle),
                &LOAD_FILE2_PROTOCOL_GUID,
                load_file2_ptr as *const c_void,
            )
            .expect("Failed to install LoadFile2");

            info!(
                "Initrd protocols installed on handle {:p}",
                initrd_handle.as_ptr()
            );
        }
    }

    // 4. Load Kernel
    info!("Loading kernel image...");
    let kernel_handle = boot::load_image(
        image_handle,
        LoadImageSource::FromBuffer {
            buffer: kernel_bytes,
            file_path: None,
        },
    )
    .expect("Failed to load kernel image");

    info!("Kernel loaded, handle: {:p}", kernel_handle.as_ptr());

    // 5. Set Command Line (allocate in pool memory so it persists)
    if let Some(cmd_bytes) = cmdline_section {
        let cmd_str = core::str::from_utf8(cmd_bytes)
            .unwrap_or("")
            .trim_matches(char::from(0));

        if !cmd_str.is_empty() {
            info!("Setting cmdline: {}", cmd_str);

            // Convert to UCS-2
            let char_count = cmd_str.chars().count() + 1; // +1 for null terminator
            let byte_size = char_count * 2;

            // Allocate from pool so it persists after this scope
            let cmdline_ptr = boot::allocate_pool(MemoryType::LOADER_DATA, byte_size)
                .expect("Failed to allocate cmdline buffer")
                .as_ptr() as *mut u16;

            // Copy characters as UCS-2
            unsafe {
                for (i, ch) in cmd_str.chars().enumerate() {
                    *cmdline_ptr.add(i) = ch as u16;
                }
                *cmdline_ptr.add(char_count - 1) = 0; // Null terminator
            }

            // Set load options on kernel
            let mut loaded_image_proto =
                boot::open_protocol_exclusive::<LoadedImage>(kernel_handle)
                    .expect("Failed to open LoadedImage for kernel");

            unsafe {
                loaded_image_proto.set_load_options(cmdline_ptr as *const u8, byte_size as u32);
            }

            info!("Cmdline set ({} bytes)", byte_size);
        }
    }

    // 6. Start Kernel
    info!("Starting kernel...");
    let result = boot::start_image(kernel_handle);

    // we should never get here
    match result {
        Ok(_) => {
            warn!("Kernel returned unexpectedly with success");
            Status::SUCCESS
        }
        Err(e) => {
            error!("Kernel returned with error: {:?}", e);
            e.status()
        }
    }
}
