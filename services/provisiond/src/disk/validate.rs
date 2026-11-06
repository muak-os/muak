use std::fs::File;

use ::disk::plan::Plan;
use anyhow::{Context as _, Result};
use partdev::table;

use super::apply;
use super::constants::{MB, SECTOR_SIZE};

/// Validates that a disk can hold the plan by dry-running the exact placement.
///
/// # Errors
///
/// Returns an error when the disk cannot be opened, its partition table cannot
/// be read, or the plan does not fit the available space.
pub(crate) fn fits_plan(disk: &str, plan: &Plan) -> Result<()> {
    let file =
        File::open(disk).with_context(|| format!("Failed to open '{disk}' for size probe"))?;
    let sector_count = table::sector_count(&file)?;

    let mut gpt = apply::load_table(&file, sector_count, plan.wipe)?;
    apply::place_all(&mut gpt, plan).with_context(|| {
        let mib = sector_count
            .saturating_mul(SECTOR_SIZE)
            .checked_div(MB)
            .unwrap_or(0);

        format!("Disk '{disk}' cannot hold the layout ({mib} MiB available)")
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use ::disk::plan::{PartitionSpec, Size};
    use parttable::gpt::partition::{LINUX_FS_GUID, Partition};
    use parttable::gpt::table::Table;
    use tempfile::NamedTempFile;

    use super::*;

    fn path(disk: &NamedTempFile) -> &str {
        disk.path().to_str().expect("utf-8 path")
    }

    fn disk_image(sector_count: u64) -> NamedTempFile {
        let disk = NamedTempFile::new().expect("temp file");
        disk.as_file()
            .set_len(sector_count.saturating_mul(SECTOR_SIZE))
            .expect("resize");

        disk
    }

    fn spec(name: &str, size: Size) -> PartitionSpec {
        PartitionSpec {
            role: None,
            name: name.to_owned(),
            type_guid: LINUX_FS_GUID,
            size,
        }
    }

    fn fixed_plan(bytes: u64) -> Plan {
        Plan::wiped(vec![spec("TEST", Size::Fixed(bytes))])
    }

    fn disk_with_foreign_partition(sector_count: u64) -> NamedTempFile {
        let disk = disk_image(sector_count);
        let mut table = Table::create(sector_count, SECTOR_SIZE, [0xff; 16]).expect("table");

        table.set_partition(
            1,
            Partition {
                type_guid: LINUX_FS_GUID,
                unique_guid: [1_u8; 16],
                starting_lba: 2048,
                ending_lba: 4095,
                attributes: 0,
                name: "Apple_APFS".to_owned(),
            },
        );

        partdev::table::commit(disk.as_file(), &table, sector_count).expect("commit table");

        disk
    }

    #[test]
    fn fits_plan_accepts_the_exact_boundary_and_rejects_one_sector_less() {
        // ARRANGE
        // A 1 MiB fixed partition starts at LBA 2048 (the first MiB boundary
        // after first usable LBA 34) and spans 2048 sectors, so it ends at LBA
        // 4095. The last usable LBA is `sector_count - 34`, so the smallest
        // disk that holds it has 4129 sectors.
        let exact = disk_image(4129);
        let short = disk_image(4128);
        let plan = fixed_plan(MB);

        // ACT
        let exact_result = fits_plan(path(&exact), &plan);
        let error = fits_plan(path(&short), &plan).expect_err("one sector short must fail");

        // ASSERT
        assert!(exact_result.is_ok(), "{exact_result:?}");
        assert!(
            format!("{error:#}").contains("partition 1 ends after last usable LBA 4094"),
            "{error:#}"
        );
    }

    #[test]
    fn fits_plan_rejects_a_fixed_size_that_rounds_past_the_last_sector() {
        // ARRANGE
        // 1 MiB + one sector starts at LBA 2048 and spans 2049 sectors, ending
        // at LBA 4096, while a 4128-sector disk ends at usable LBA 4094. The
        // old heuristic charged 2 MiB plus one sector and accepted this disk.
        let disk = disk_image(4128);
        let plan = fixed_plan(MB + SECTOR_SIZE);

        // ACT
        let result = fits_plan(path(&disk), &plan);

        // ASSERT
        assert!(
            result.is_err(),
            "a fixed size past the last usable LBA must be rejected"
        );
    }

    #[test]
    fn fits_plan_fills_to_last_usable_after_a_fixed_partition() {
        // ARRANGE
        // The 1 MiB partition ends at LBA 4095, so the fill partition starts at
        // LBA 4096 and needs the last usable LBA to be at least 4096, which a
        // 4130-sector disk provides exactly (4130 - 34 = 4096).
        let exact = disk_image(4130);
        let short = disk_image(4129);
        let plan = Plan::wiped(vec![
            spec("TEST", Size::Fixed(MB)),
            spec("FILL", Size::Fill),
        ]);

        // ACT
        let exact_result = fits_plan(path(&exact), &plan);
        let error = fits_plan(path(&short), &plan).expect_err("fill must end past the disk");

        // ASSERT
        assert!(exact_result.is_ok(), "{exact_result:?}");
        assert!(
            format!("{error:#}").contains("partition 2 start LBA 4096 is after end LBA 4095"),
            "{error:#}"
        );
    }

    #[test]
    fn fits_plan_rejects_a_non_wipe_plan_on_a_blank_disk() {
        // ARRANGE
        let disk = disk_image(0x0002_0000);
        let mut plan = fixed_plan(MB);
        plan.wipe = false;

        // ACT
        let error = fits_plan(path(&disk), &plan).expect_err("blank disk must fail");

        // ASSERT
        assert!(
            format!("{error:#}").contains("no partition table"),
            "expected a table error, got: {error:#}"
        );
    }

    #[test]
    fn fits_plan_appends_a_non_wipe_plan_after_a_foreign_partition() {
        // ARRANGE
        // A 1 MiB foreign partition sits at the first aligned slot, so the plan
        // must start at LBA 4096. A 6177-sector disk ends exactly at LBA 6143,
        // the last sector the plan needs.
        let disk = disk_with_foreign_partition(6177);
        let mut plan = fixed_plan(MB);
        plan.wipe = false;

        // ACT
        let result = fits_plan(path(&disk), &plan);

        // ASSERT
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn fits_plan_is_table_aware_for_a_non_wipe_plan() {
        // ARRANGE
        // 5000 sectors hold the plan if it starts at the first aligned slot, but
        // not after the foreign partition, so only a table-aware dry run rejects
        // it.
        let disk = disk_with_foreign_partition(5000);
        let mut plan = fixed_plan(MB);
        plan.wipe = false;

        // ACT
        let result = fits_plan(path(&disk), &plan);

        // ASSERT
        assert!(
            result.is_err(),
            "placement must account for the foreign entry"
        );
    }
}
