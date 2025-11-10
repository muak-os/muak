use uefi::Guid;
use uefi::mem::memory_map::MemoryType;

/// LINUX_EFI_INITRD_MEDIA_GUID - Used by Linux kernel to identify initrd LoadFile2 protocol
/// GUID: 5568e427-68fc-4f3d-ac74-ca555231cc68
const LINUX_EFI_INITRD_MEDIA_GUID: Guid = Guid::from_bytes([
    0x27, 0xe4, 0x68, 0x55, // time_low (little-endian)
    0xfc, 0x68, // time_mid (little-endian)
    0x3d, 0x4f, // time_high_and_version (little-endian)
    0xac, 0x74, 0xca, 0x55, 0x52, 0x31, 0xcc, 0x68, // clock_seq and node
]);

/// EFI_LOAD_FILE2_PROTOCOL_GUID
const EFI_LOAD_FILE2_PROTOCOL_GUID: Guid = Guid::from_bytes([
    0xc1, 0xc0, 0x06, 0x40, // time_low (little-endian)
    0xb3, 0xfc, // time_mid (little-endian)
    0x3e, 0x40, // time_high_and_version (little-endian)
    0x99, 0x6d, 0x4a, 0x6c, 0x87, 0x24, 0xe0, 0x6d, // clock_seq and node
]);

/// EFI Device Path Protocol GUID
const DEVICE_PATH_PROTOCOL_GUID: Guid = Guid::from_bytes([
    0x09, 0x03, 0x03, 0x09, // time_low (little-endian)
    0xA4, 0x8F, // time_mid (little-endian)
    0xF1, 0x11, // time_high_and_version (little-endian)
    0x9F, 0x22, 0x00, 0x0A, 0xC9, 0x69, 0x72, 0x3B, // clock_seq and node
]);
#[repr(C, packed)]
struct VendorDevicePath {
    header: DevicePathHeader,
    guid: Guid,
}

#[repr(C, packed)]
struct DevicePathHeader {
    type_: u8,
    sub_type: u8,
    length: [u8; 2],
}

#[repr(C, packed)]
struct EndDevicePath {
    header: DevicePathHeader,
}

/// Complete device path for initrd: VendorMedia + End
#[repr(C, packed)]
struct InitrdDevicePath {
    vendor: VendorDevicePath,
    end: EndDevicePath,
}

impl InitrdDevicePath {
    fn new() -> Self {
        Self {
            vendor: VendorDevicePath {
                header: DevicePathHeader {
                    type_: 0x04,    // MEDIA_DEVICE_PATH
                    sub_type: 0x03, // MEDIA_VENDOR_DP
                    length: [
                        (core::mem::size_of::<VendorDevicePath>() & 0xFF) as u8,
                        ((core::mem::size_of::<VendorDevicePath>() >> 8) & 0xFF) as u8,
                    ],
                },
                guid: LINUX_EFI_INITRD_MEDIA_GUID,
            },
            end: EndDevicePath {
                header: DevicePathHeader {
                    type_: 0x7F,    // END_DEVICE_PATH_TYPE
                    sub_type: 0xFF, // END_ENTIRE_DEVICE_PATH_SUBTYPE
                    length: [
                        (core::mem::size_of::<EndDevicePath>() & 0xFF) as u8,
                        ((core::mem::size_of::<EndDevicePath>() >> 8) & 0xFF) as u8,
                    ],
                },
            },
        }
    }
}

/// LoadFile2 protocol function pointer type
type LoadFile2Fn = extern "efiapi" fn(
    this: *mut LoadFile2Protocol,
    file_path: *const u8, // DevicePath pointer
    boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> uefi::Status;

/// LoadFile2 Protocol structure
#[repr(C)]
struct LoadFile2Protocol {
    load_file: LoadFile2Fn,
    initrd_data: *const u8,
    initrd_size: usize,
}

/// LoadFile2 protocol implementation
extern "efiapi" fn load_file2_impl(
    this: *mut LoadFile2Protocol,
    _file_path: *const u8,
    _boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> uefi::Status {
    unsafe {
        let protocol = &*this;
        let required_size = protocol.initrd_size;

        if buffer.is_null() {
            // First call: return required size
            *buffer_size = required_size;
            return uefi::Status::BUFFER_TOO_SMALL;
        }

        if *buffer_size < required_size {
            *buffer_size = required_size;
            return uefi::Status::BUFFER_TOO_SMALL;
        }

        // Copy initrd data to buffer
        core::ptr::copy_nonoverlapping(protocol.initrd_data, buffer, required_size);
        *buffer_size = required_size;

        uefi::Status::SUCCESS
    }
}

/// Boot Linux kernel using LoadFile2 protocol for initrd
pub fn boot_linux(kernel: &[u8], initrd: &[u8], cmdline: &str) -> ! {
    log::info!("Preparing to boot Linux kernel via EFI stub...");

    let image_handle = uefi::boot::image_handle();

    // Allocate memory for initrd and copy it (needs to persist until kernel loads it)
    let initrd_pages = (initrd.len() + 4095) / 4096;
    let initrd_addr = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        initrd_pages,
    )
    .expect("Failed to allocate initrd memory");

    unsafe {
        core::ptr::copy_nonoverlapping(
            initrd.as_ptr(),
            initrd_addr.as_ptr() as *mut u8,
            initrd.len(),
        );
    }

    log::info!(
        "Initrd allocated at {:p} ({} bytes)",
        initrd_addr.as_ptr(),
        initrd.len()
    );

    // Create LoadFile2 protocol instance
    let protocol_pages = 1; // One page is enough for the protocol structure
    let protocol_addr = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        protocol_pages,
    )
    .expect("Failed to allocate LoadFile2 protocol memory");

    let protocol = unsafe { &mut *(protocol_addr.as_ptr() as *mut LoadFile2Protocol) };
    protocol.load_file = load_file2_impl;
    protocol.initrd_data = initrd_addr.as_ptr() as *const u8;
    protocol.initrd_size = initrd.len();

    // Create device path
    let device_path_pages = 1;
    let device_path_addr = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        device_path_pages,
    )
    .expect("Failed to allocate device path memory");

    let device_path = unsafe { &mut *(device_path_addr.as_ptr() as *mut InitrdDevicePath) };
    *device_path = InitrdDevicePath::new();

    // Install LoadFile2 protocol on a new handle
    log::info!("Installing LoadFile2 protocol for initrd...");

    let mut handle_ptr: *mut core::ffi::c_void = core::ptr::null_mut();
    let boot_services = unsafe {
        uefi::table::system_table_raw()
            .expect("System table not available")
            .as_ref()
            .boot_services
    };

    let status = unsafe {
        ((*boot_services).install_multiple_protocol_interfaces)(
            &mut handle_ptr,
            &EFI_LOAD_FILE2_PROTOCOL_GUID,
            protocol as *mut LoadFile2Protocol as *mut core::ffi::c_void,
            &DEVICE_PATH_PROTOCOL_GUID,
            device_path as *mut InitrdDevicePath as *mut core::ffi::c_void,
            core::ptr::null_mut::<core::ffi::c_void>(),
        )
    };

    if status.is_error() {
        log::error!("Failed to install LoadFile2 protocol: {:?}", status);
        panic!("Cannot continue without LoadFile2 protocol");
    }

    log::info!("LoadFile2 protocol installed successfully");

    // Convert cmdline to UTF-16 for UEFI (without initrd= parameter!)
    let mut cmdline_utf16 = alloc::vec::Vec::new();
    for ch in cmdline.chars() {
        cmdline_utf16.push(ch as u16);
    }
    cmdline_utf16.push(0); // Null terminator

    log::info!("Command line: {}", cmdline);

    // Load the kernel as an EFI image
    log::info!("Loading kernel image via LoadImage()...");

    let source = uefi::boot::LoadImageSource::FromBuffer {
        buffer: kernel,
        file_path: None,
    };

    let kernel_handle = match uefi::boot::load_image(image_handle, source) {
        Ok(h) => h,
        Err(e) => {
            log::error!("LoadImage failed: {:?}", e);
            panic!("Failed to load kernel image");
        }
    };

    log::info!("Kernel image loaded successfully");

    // Set load options (command line) for the kernel
    {
        let mut loaded_image = uefi::boot::open_protocol_exclusive::<
            uefi::proto::loaded_image::LoadedImage,
        >(kernel_handle)
        .expect("Failed to open LoadedImage protocol");

        // Allocate load options
        let load_options_pages = (cmdline_utf16.len() * 2 + 4095) / 4096;
        let load_options_addr = uefi::boot::allocate_pages(
            uefi::boot::AllocateType::AnyPages,
            MemoryType::LOADER_DATA,
            load_options_pages,
        )
        .expect("Failed to allocate load options");

        unsafe {
            core::ptr::copy_nonoverlapping(
                cmdline_utf16.as_ptr(),
                load_options_addr.as_ptr() as *mut u16,
                cmdline_utf16.len(),
            );

            loaded_image.set_load_options(
                load_options_addr.as_ptr() as *const u8,
                (cmdline_utf16.len() * 2) as u32,
            );
        }

        log::info!("Load options set");
    }

    // Start the kernel image
    log::info!("Starting kernel via StartImage()...");
    log::info!("Kernel will load initrd via LoadFile2 protocol...");

    match uefi::boot::start_image(kernel_handle) {
        Ok(_) => {
            log::error!("Kernel returned unexpectedly");
        }
        Err(e) => {
            log::error!("StartImage failed: {:?}", e);
        }
    }

    // Should never reach here
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}
