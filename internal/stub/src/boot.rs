use uefi::Guid;
use uefi::mem::memory_map::MemoryType;

// Linux GUID used to tag the initrd LoadFile2 provider
const LINUX_EFI_INITRD_MEDIA_GUID: Guid = Guid::from_bytes([
    0x27, 0xe4, 0x68, 0x55, 0xfc, 0x68, 0x3d, 0x4f, 0xac, 0x74, 0xca, 0x55, 0x52, 0x31, 0xcc, 0x68,
]);

// LoadFile2 protocol GUID
const EFI_LOAD_FILE2_PROTOCOL_GUID: Guid = Guid::from_bytes([
    0xc1, 0xc0, 0x06, 0x40, 0xb3, 0xfc, 0x3e, 0x40, 0x99, 0x6d, 0x4a, 0x6c, 0x87, 0x24, 0xe0, 0x6d,
]);

// Device Path protocol GUID
const DEVICE_PATH_PROTOCOL_GUID: Guid = Guid::from_bytes([
    0x91, 0x6e, 0x57, 0x09, 0x3f, 0x6d, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
]);

#[repr(C, packed)]
struct DevicePathHeader {
    type_: u8,
    sub_type: u8,
    length: [u8; 2],
}

#[repr(C, packed)]
struct VendorDevicePath {
    header: DevicePathHeader,
    guid: Guid,
}

#[repr(C, packed)]
struct EndDevicePath {
    header: DevicePathHeader,
}

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

type LoadFile2Fn = extern "efiapi" fn(
    this: *mut LoadFile2Protocol,
    file_path: *const u8,
    boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> uefi::Status;

#[repr(C)]
struct LoadFile2Protocol {
    load_file: LoadFile2Fn,
    initrd_data: *const u8,
    initrd_size: usize,
}

extern "efiapi" fn load_file2_impl(
    this: *mut LoadFile2Protocol,
    _file_path: *const u8,
    _boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> uefi::Status {
    unsafe {
        let proto = &*this;
        let size = proto.initrd_size;
        if buffer.is_null() {
            *buffer_size = size;
            return uefi::Status::BUFFER_TOO_SMALL;
        }
        if *buffer_size < size {
            *buffer_size = size;
            return uefi::Status::BUFFER_TOO_SMALL;
        }
        core::ptr::copy_nonoverlapping(proto.initrd_data, buffer, size);
        *buffer_size = size;
        uefi::Status::SUCCESS
    }
}

pub fn boot_linux(kernel: &[u8], initrd: &[u8], cmdline: &str) -> ! {
    let image_handle = uefi::boot::image_handle();

    let initrd_pages = initrd.len().div_ceil(4096);
    let initrd_addr = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        initrd_pages,
    )
    .expect("alloc initrd");
    unsafe {
        core::ptr::copy_nonoverlapping(initrd.as_ptr(), initrd_addr.as_ptr(), initrd.len());
    }

    let protocol_addr = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        1,
    )
    .expect("alloc LoadFile2");
    let proto = unsafe { &mut *(protocol_addr.as_ptr() as *mut LoadFile2Protocol) };
    proto.load_file = load_file2_impl;
    proto.initrd_data = initrd_addr.as_ptr() as *const u8;
    proto.initrd_size = initrd.len();

    let device_path_addr = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        1,
    )
    .expect("alloc device path");
    let dp = unsafe { &mut *(device_path_addr.as_ptr() as *mut InitrdDevicePath) };
    *dp = InitrdDevicePath::new();

    let mut handle: *mut core::ffi::c_void = core::ptr::null_mut();
    let bs = unsafe {
        uefi::table::system_table_raw()
            .unwrap()
            .as_ref()
            .boot_services
    };
    let st = unsafe {
        ((*bs).install_multiple_protocol_interfaces)(
            &mut handle,
            &EFI_LOAD_FILE2_PROTOCOL_GUID,
            proto as *mut LoadFile2Protocol as *mut core::ffi::c_void,
            &DEVICE_PATH_PROTOCOL_GUID,
            dp as *mut InitrdDevicePath as *mut core::ffi::c_void,
            core::ptr::null_mut::<core::ffi::c_void>(),
        )
    };
    if st.is_error() {
        panic!("install LoadFile2 failed: {:?}", st);
    }

    let mut cmd_utf16 = alloc::vec::Vec::new();
    for ch in cmdline.chars() {
        cmd_utf16.push(ch as u16);
    }
    cmd_utf16.push(0);

    let source = uefi::boot::LoadImageSource::FromBuffer {
        buffer: kernel,
        file_path: None,
    };
    let kh = uefi::boot::load_image(image_handle, source).expect("load kernel");

    let mut li = uefi::boot::open_protocol_exclusive::<uefi::proto::loaded_image::LoadedImage>(kh)
        .expect("open LoadedImage");
    let load_opts_pages = (cmd_utf16.len() * 2).div_ceil(4096);
    let load_opts_addr = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        load_opts_pages,
    )
    .expect("alloc cmdline");
    unsafe {
        core::ptr::copy_nonoverlapping(
            cmd_utf16.as_ptr(),
            load_opts_addr.as_ptr() as *mut u16,
            cmd_utf16.len(),
        );
        li.set_load_options(
            load_opts_addr.as_ptr() as *const u8,
            (cmd_utf16.len() * 2) as u32,
        );
    }

    let _ = uefi::boot::start_image(kh);

    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}
