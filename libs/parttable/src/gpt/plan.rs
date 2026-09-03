//! Shared GPT planning.

use super::layout::{ALIGN_1_MIB_SECTORS, PlacementRequest};
use super::table::{ENTRIES_COUNT, Table};
use crate::error::{ParttableError, Result};

/// The standard 512-byte sector size used across Muak disk images.
pub const SECTOR_SIZE: u64 = 512;

/// The standard 1 MiB partition alignment in bytes.
pub const ALIGN_1_MIB_BYTES: u64 = ALIGN_1_MIB_SECTORS.saturating_mul(SECTOR_SIZE);

/// Returns the size in bytes of the primary GPT region for the given sector size.
#[must_use]
pub fn primary_gpt_size(sector_size: u64) -> u64 {
    let entries_sectors = entries_sector_count(sector_size);

    (2_u64.saturating_add(entries_sectors)).saturating_mul(sector_size)
}

/// Finds the smallest sector count, growing in `step_sectors` steps from
/// `base_sectors`, where `request` can be placed on a GPT with the given geometry.
///
/// # Errors
///
/// Returns an error when table creation fails, placement fails for a reason
/// other than "does not fit", or the sector count overflows while growing.
pub fn smallest_disk(
    base_sectors: u64,
    sector_size: u64,
    step_sectors: u64,
    disk_guid: [u8; 16],
    request: &PlacementRequest,
) -> Result<u64> {
    let mut sector_count = base_sectors;
    loop {
        let mut table = Table::create(sector_count, sector_size, disk_guid)?;
        match request.place(&mut table, sector_size) {
            Ok(_) => return Ok(sector_count),
            Err(ParttableError::InvalidPlacement(_)) => {
                sector_count = sector_count
                    .checked_add(step_sectors)
                    .ok_or_else(growth_overflow_error)?;
            }
            Err(err) => return Err(err),
        }
    }
}

fn entries_sector_count(sector_size: u64) -> u64 {
    u64::from(super::table::ENTRY_SIZE)
        .saturating_mul(u64::try_from(ENTRIES_COUNT).unwrap_or(0))
        .div_ceil(sector_size)
}

fn growth_overflow_error() -> ParttableError {
    ParttableError::InvalidPlacement("disk sector count overflowed while growing".to_owned())
}

#[cfg(test)]
mod tests {
    use esp::EFI_GUID;

    use super::super::layout::Size;
    use super::super::partition::LINUX_FS_GUID;
    use super::*;

    const DISK_GUID: [u8; 16] = [0xff; 16];

    #[test]
    fn primary_gpt_size_matches_table_geometry() {
        // ARRANGE
        let table = Table::create(ALIGN_1_MIB_SECTORS * 2, SECTOR_SIZE, DISK_GUID)
            .expect("table must be created");

        // ACT
        let from_table = table.primary_gpt_size();
        let from_plan = primary_gpt_size(SECTOR_SIZE);

        // ASSERT
        assert_eq!(from_table, from_plan);
        assert_eq!(from_plan, 34 * SECTOR_SIZE);
    }

    #[test]
    fn smallest_disk_returns_base_when_partition_fits() {
        // ARRANGE
        let base_sectors = ALIGN_1_MIB_SECTORS * 4;
        let request = PlacementRequest::new(EFI_GUID, [0xaa; 16], "EFI", Size::Bytes(1024 * 1024));

        // ACT
        let disk_sectors = smallest_disk(
            base_sectors,
            SECTOR_SIZE,
            ALIGN_1_MIB_SECTORS,
            DISK_GUID,
            &request,
        )
        .expect("smallest_disk must succeed");

        // ASSERT
        assert_eq!(disk_sectors, base_sectors);
    }

    #[test]
    fn smallest_disk_grows_until_the_partition_fits() {
        // ARRANGE
        let base_sectors = ALIGN_1_MIB_SECTORS * 2;
        let request =
            PlacementRequest::new(EFI_GUID, [0xaa; 16], "EFI", Size::Bytes(3 * 1024 * 1024));

        // ACT
        let disk_sectors = smallest_disk(
            base_sectors,
            SECTOR_SIZE,
            ALIGN_1_MIB_SECTORS,
            DISK_GUID,
            &request,
        )
        .expect("smallest_disk must succeed");

        // ASSERT
        assert!(disk_sectors > base_sectors);
        assert_eq!(disk_sectors.rem_euclid(ALIGN_1_MIB_SECTORS), 0);
    }

    #[test]
    fn smallest_disk_honors_a_custom_growth_step() {
        // ARRANGE
        let base_sectors = ALIGN_1_MIB_SECTORS * 2;
        let request =
            PlacementRequest::new(EFI_GUID, [0xaa; 16], "EFI", Size::Bytes(3 * 1024 * 1024));

        // ACT
        let coarse = smallest_disk(
            base_sectors,
            SECTOR_SIZE,
            ALIGN_1_MIB_SECTORS,
            DISK_GUID,
            &request,
        )
        .expect("1 MiB steps must succeed");
        let fine = smallest_disk(base_sectors, SECTOR_SIZE, 1, DISK_GUID, &request)
            .expect("1-sector steps must succeed");

        // ASSERT
        assert!(fine > base_sectors);
        assert!(
            fine < coarse,
            "a 1-sector growth step must find a smaller disk than 1 MiB steps"
        );
    }

    #[test]
    fn smallest_disk_propagates_table_creation_errors() {
        // ARRANGE
        let base_sectors = 10;
        let request = PlacementRequest::new(EFI_GUID, [0xaa; 16], "EFI", Size::Bytes(1024 * 1024));

        // ACT
        let result = smallest_disk(
            base_sectors,
            SECTOR_SIZE,
            ALIGN_1_MIB_SECTORS,
            DISK_GUID,
            &request,
        );

        // ASSERT
        assert!(matches!(result, Err(ParttableError::Gpt(_))));
    }

    #[test]
    fn smallest_disk_errors_when_sector_count_overflows() {
        // ARRANGE
        let base_sectors = u64::MAX - 1;
        let mut request =
            PlacementRequest::new(LINUX_FS_GUID, [0xaa; 16], "DATA", Size::Bytes(1024 * 1024));
        request.alignment_lba = 1;
        request.start = super::super::layout::Start::AtOrAfter(u64::MAX - 100);

        // ACT
        let result = smallest_disk(
            base_sectors,
            SECTOR_SIZE,
            ALIGN_1_MIB_SECTORS,
            DISK_GUID,
            &request,
        );

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("overflowed"))
        );
    }
}
