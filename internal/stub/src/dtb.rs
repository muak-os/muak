//! Device Tree Blob (DTB) installation for ARM64 platforms.
//!
//! This module handles installing a DTB into the UEFI System Configuration Table
//! so that the Linux kernel can find it during boot.

use anyhow::{Context, Result, bail};
use uefi::Guid;
use uefi::boot::MemoryType;

use crate::info;

/// UEFI Device Tree Table GUID (EFI_DTB_TABLE_GUID)
const EFI_DTB_TABLE_GUID: Guid = Guid::parse_or_panic("b1b621d5-f19c-41a5-830b-d9152c69aae0");

/// DTB magic number (big-endian: 0xd00dfeed)
const DTB_MAGIC: u32 = 0xd00dfeed;

/// Validates a Device Tree Blob header.
fn validate_dtb(data: &[u8]) -> Result<()> {
    if data.len() < 4 {
        bail!("DTB too small (minimum 4 bytes for magic)");
    }

    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    if magic != DTB_MAGIC {
        bail!(
            "Invalid DTB magic: expected 0x{:08x}, got 0x{:08x}",
            DTB_MAGIC,
            magic
        );
    }

    Ok(())
}

/// Installs a Device Tree Blob into the UEFI System Configuration Table.
///
/// This copies the DTB to EfiACPIReclaimMemory (which persists after ExitBootServices)
/// and installs it in the configuration table using EFI_DTB_TABLE_GUID.
///
/// # Arguments
/// * `data` - The raw DTB data
///
/// # Returns
/// Ok(()) on success, or an error if installation fails.
#[cfg(target_arch = "aarch64")]
pub fn install(data: &[u8]) -> Result<()> {
    validate_dtb(data)?;

    info!(
        "Installing DTB ({} bytes) into configuration table",
        data.len()
    );

    let dtb_ptr = uefi::boot::allocate_pool(MemoryType::ACPI_RECLAIM, data.len())
        .context("Failed to allocate memory for DTB")?
        .as_ptr();

    // SAFETY: dtb_ptr is freshly allocated with sufficient size
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), dtb_ptr, data.len());
    }

    // SAFETY: dtb_ptr is valid and points to a valid DTB
    unsafe {
        uefi::boot::install_configuration_table(&EFI_DTB_TABLE_GUID, dtb_ptr.cast())
            .context("Failed to install DTB configuration table")?;
    }

    info!("DTB installed at {:p}", dtb_ptr);

    Ok(())
}

/// Stub for non-aarch64 platforms - DTB installation is not supported.
#[cfg(not(target_arch = "aarch64"))]
pub fn install(_data: &[u8]) -> Result<()> {
    bail!("DTB installation is only supported on aarch64 platforms")
}
