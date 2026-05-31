//! Direct PE kernel loader.
//!
//! Loads the embedded Linux kernel PE image by manually mapping its sections into memory and
//! jumping to the entry point.

mod cmdline;
mod mapping;
mod protection;
pub(crate) mod protocol;

use core::ffi::c_void;
use core::mem;

use anyhow::{Context as _, Result, anyhow};
use uefi::boot::ScopedProtocol;
use uefi::proto::loaded_image::LoadedImage;
use uefi::{Handle, Status};

use crate::pe::kernel::Image;
use crate::{info, warn};

type EfiEntryPoint = unsafe extern "efiapi" fn(Handle, *mut c_void) -> Status;

/// Maps the kernel into memory and transfers control to it.
///
/// # Errors
///
/// Returns an error if mapping, command-line allocation, or returned kernel status handling fails.
#[cfg(feature = "uefi")]
pub fn start(
    kernel: &Image<'_>,
    cmdline: Option<&[u8]>,
    mut loaded_image: ScopedProtocol<LoadedImage>,
    image_handle: Handle,
) -> Result<()> {
    let loaded_base = mapping::map_sections(kernel)?;

    if kernel.nx_compat {
        if let Some(proto) = protection::locate() {
            info!("EFI_MEMORY_ATTRIBUTE_PROTOCOL available, applying W^X");
            protection::apply(proto, loaded_base, kernel);
        } else {
            warn!("Kernel has NX_COMPAT but EFI_MEMORY_ATTRIBUTE_PROTOCOL not available");
        }
    }

    let image_size = u64::from(kernel.size);
    // SAFETY: loaded_base is valid allocated memory of `kernel.size` bytes.
    unsafe {
        loaded_image.set_image(loaded_base.cast::<c_void>(), image_size);
    }

    if let Some(cmdline_bytes) = cmdline {
        let (ptr, size) = cmdline::encode_ucs2(cmdline_bytes)?;
        if !ptr.is_null() {
            // SAFETY: ptr is valid pool memory.
            unsafe {
                loaded_image.set_load_options(ptr, size);
            }
            info!("Command line set ({size} bytes UCS-2)");
        }
    }

    let base_addr = u64::try_from(loaded_base.addr()).context("kernel base address exceeds u64")?;
    let entry_addr = base_addr
        .checked_add(u64::from(kernel.entry_point_rva))
        .context("kernel entry address overflow")?;
    info!("Jumping to kernel entry at 0x{entry_addr:x}");

    drop(loaded_image);

    let entry_addr = usize::try_from(entry_addr).context("kernel entry address exceeds usize")?;
    // SAFETY: entry_addr is within the mapped kernel image.
    let entry: EfiEntryPoint = unsafe { mem::transmute(entry_addr) };
    let system_table = std::os::uefi::env::system_table().as_ptr().cast();
    // SAFETY: entry points to the Linux EFI stub entry point and receives the current firmware
    // image handle and system table.
    let status = unsafe { entry(image_handle, system_table) };

    Err(anyhow!(
        "Kernel entry point returned with status: {status:?}"
    ))
}
