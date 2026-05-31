//! Device Tree Blob (DTB) installation for ARM64 platforms.

use core::ptr;

use anyhow::{Context as _, Result, bail};
use uefi::Guid;
use uefi::boot::{MemoryType, allocate_pool, install_configuration_table};

use crate::info;

const EFI_DTB_TABLE_GUID: Guid = Guid::parse_or_panic("b1b621d5-f19c-41a5-830b-d9152c69aae0");

const DTB_MAGIC: u32 = 0xd00d_feed;

/// Validates a Device Tree Blob header.
fn validate_dtb(data: &[u8]) -> Result<()> {
    let magic_bytes = data
        .get(..4)
        .ok_or_else(|| anyhow::anyhow!("DTB too small (minimum 4 bytes for magic)"))?;
    let magic_bytes = <[u8; 4]>::try_from(magic_bytes).context("invalid DTB magic length")?;
    let magic = u32::from_be_bytes(magic_bytes);
    if magic != DTB_MAGIC {
        bail!("Invalid DTB magic: expected 0x{DTB_MAGIC:08x}, got 0x{magic:08x}");
    }

    Ok(())
}

/// Installs a Device Tree Blob into the UEFI System Configuration Table.
///
/// # Errors
///
/// Returns an error if the DTB is invalid or UEFI allocation/table installation fails.
pub fn install(data: &[u8]) -> Result<()> {
    validate_dtb(data)?;

    info!(
        "Installing DTB ({} bytes) into configuration table",
        data.len()
    );

    let dtb_ptr = allocate_pool(MemoryType::ACPI_RECLAIM, data.len())
        .context("Failed to allocate memory for DTB")?
        .as_ptr();

    // SAFETY: dtb_ptr is freshly allocated with sufficient size.
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), dtb_ptr, data.len());
    }

    // SAFETY: dtb_ptr is valid and points to a valid DTB.
    unsafe {
        install_configuration_table(&EFI_DTB_TABLE_GUID, dtb_ptr.cast())
            .context("Failed to install DTB configuration table")?;
    }

    info!("DTB installed at {:p}", dtb_ptr);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_dtb_too_small_zero_bytes() {
        // ARRANGE
        let data = b"";

        // ACT
        let err = validate_dtb(data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("DTB too small"), "{err}");
    }

    #[test]
    fn validate_dtb_too_small_three_bytes() {
        // ARRANGE
        let data = &[0xd0, 0x0d, 0xfe];

        // ACT
        let err = validate_dtb(data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("DTB too small"), "{err}");
    }

    #[test]
    fn validate_dtb_wrong_magic() {
        // ARRANGE
        let data = &[0x00, 0x00, 0x00, 0x00];

        // ACT
        let err = validate_dtb(data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("Invalid DTB magic"), "{err}");
    }

    #[test]
    fn validate_dtb_correct_magic_four_bytes() {
        // ARRANGE
        let data = &[0xd0, 0x0d, 0xfe, 0xed];

        // ACT + ASSERT
        validate_dtb(data).expect("valid DTB magic should pass");
    }

    #[test]
    fn validate_dtb_correct_magic_larger_blob() {
        // ARRANGE

        let mut blob = vec![0_u8; 64];
        let magic = blob.get_mut(..4).expect("test blob has magic bytes");
        magic.copy_from_slice(&[0xd0, 0x0d, 0xfe, 0xed]);

        // ACT + ASSERT
        validate_dtb(&blob).expect("valid DTB should pass");
    }

    #[test]
    fn validate_dtb_little_endian_magic_rejected() {
        // ARRANGE
        let data = &[0xed, 0xfe, 0x0d, 0xd0];

        // ACT
        let err = validate_dtb(data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("Invalid DTB magic"), "{err}");
    }
}
