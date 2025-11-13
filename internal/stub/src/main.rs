#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod log;
mod boot;
mod cpio;
mod pe;

use uefi::allocator::Allocator;
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();

    let image_handle = uefi::boot::image_handle();

    info!("Muak EFI Stub v0.1.0");
    info!("Extracting UKI sections...");

    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
        .expect("Failed to get LoadedImage protocol");

    let sections = match pe::extract_sections(&loaded_image) {
        Ok(s) => {
            info!("Successfully extracted PE sections:");
            info!("  .linux:   {} bytes", s.kernel.len());
            info!("  .cmdline: {} bytes", s.cmdline.len());
            info!("  .initrd:  {} bytes", s.initrd.len());
            s
        }
        Err(e) => {
            error!("Failed to extract PE sections: {:?}", e);
            return Status::ABORTED;
        }
    };

    info!("Building enhanced initrd...");
    let enhanced_initrd = match cpio::build_enhanced_initrd(&sections) {
        Ok(data) => {
            info!("Enhanced initrd size: {} bytes", data.len());
            data
        }
        Err(e) => {
            error!("Failed to build enhanced initrd: {:?}", e);
            return Status::ABORTED;
        }
    };

    info!("Booting Linux kernel...");

    let cmdline =
        core::str::from_utf8(sections.cmdline).unwrap_or("console=ttyS0 console=tty0 init=/init");

    boot::boot_linux(sections.kernel, &enhanced_initrd, cmdline)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
