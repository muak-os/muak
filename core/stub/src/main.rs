//! UEFI stub for Muak - Loads and starts the Linux kernel from a Unified Kernel Image

#![feature(uefi_std)]

#[cfg(target_arch = "aarch64")]
mod dtb;
mod loadfile2;
mod log;
mod pe;
mod peloader;
mod security;
mod tpm2;
mod util;

use std::os::uefi as uefi_std;

use anyhow::{Context, Result};
use base64ct::{Base64Unpadded, Encoding};
use uefi::Guid;
use uefi::Handle;
use uefi::proto::loaded_image::LoadedImage;

use crate::pe::{KernelPe, UkiSections};
use crate::util::strip_trailing_nuls;

const LINUX_INITRD_GUID: Guid = Guid::parse_or_panic("5568e427-68fc-4f3d-ac74-ca555231cc68");

const LUKS_KEY_PREFIX: &[u8] = b" luks.key=";

/// Initializes the UEFI crate with system table and image handle
fn setup_uefi_crate() {
    let st = uefi_std::env::system_table();
    let ih = uefi_std::env::image_handle();

    // SAFETY: UEFI firmware provides valid system table and image handle pointers
    // during the boot services phase. This is required setup for the `uefi` crate.
    unsafe {
        uefi::table::set_system_table(st.as_ptr().cast());

        let ih = Handle::from_ptr(ih.as_ptr().cast()).expect("Something's very wrong");
        uefi::boot::set_image_handle(ih);
    }
}

/// Entry point for the UEFI stub
fn main() -> Result<()> {
    setup_uefi_crate();

    info!("Muak stub v{} starting...", env!("CARGO_PKG_VERSION"));

    let image_handle = uefi::boot::image_handle();

    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
        .context("Failed to open LoadedImage protocol")?;

    info!(
        "Setup Mode: {}",
        if security::is_setup_mode() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "Secure Boot: {}",
        if security::is_secure_boot_enabled() {
            "enabled"
        } else {
            "disabled"
        }
    );

    let (base_addr, image_size) = loaded_image.info();
    info!("Base address: {:p}, size: {}", base_addr, image_size);

    let image_data =
        // SAFETY: base_addr and image_size come from UEFI's LoadedImage protocol,
        // which guarantees the image is valid and loaded in memory for the entire
        // boot services phase. The slice is used only for reading PE section data.
        unsafe { std::slice::from_raw_parts(base_addr as *const u8, image_size as usize) };
    let sections = UkiSections::parse(image_data)?;

    for (name, data) in sections.iter_sections() {
        match tpm2::measure_section(name, data) {
            Ok(()) => info!("TPM2: measured {} ({} bytes) into PCR#11", name, data.len()),
            Err(e) => warn!("TPM2: skipping measurement for {}: {}", name, e),
        }
    }

    info!(
        "Kernel: {} bytes at {:p}",
        sections.linux.len(),
        sections.linux.as_ptr()
    );

    let kernel = KernelPe::parse(sections.linux)?;
    info!(
        "Kernel PE: entry=0x{:x}, base=0x{:x}, size=0x{:x}",
        kernel.entry_point_rva, kernel.image_base, kernel.size_of_image
    );

    if let Some(initrd_bytes) = sections.initrd {
        loadfile2::install(initrd_bytes, &LINUX_INITRD_GUID)?;
    }

    #[cfg(target_arch = "aarch64")]
    if let Some(dtb_bytes) = sections.dtb {
        dtb::install(dtb_bytes)?;
    }

    let combined_cmdline: Vec<u8>;
    let cmdline: Option<&[u8]> = if let Some(luks_data) = sections.luks {
        let base_cmd = sections
            .cmdline
            .map(|c| strip_trailing_nuls(c))
            .unwrap_or(b"");
        let encoded_len = Base64Unpadded::encoded_len(luks_data);

        let total_len = base_cmd.len() + LUKS_KEY_PREFIX.len() + encoded_len;
        let mut cmd = Vec::with_capacity(total_len);
        cmd.extend_from_slice(base_cmd);
        cmd.extend_from_slice(LUKS_KEY_PREFIX);

        let start = cmd.len();
        cmd.resize(total_len, 0);
        Base64Unpadded::encode(luks_data, &mut cmd[start..])
            .context("Failed to encode LUKS key")?;

        combined_cmdline = cmd;
        info!("LUKS key embedded ({} bytes)", luks_data.len());
        Some(&combined_cmdline)
    } else {
        sections.cmdline
    };

    peloader::start(&kernel, cmdline, loaded_image, image_handle)?;

    unreachable!("Kernel entry point returned, which should never happen");
}
