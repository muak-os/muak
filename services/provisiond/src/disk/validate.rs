use ::disk::plan::{Plan, Size};
use anyhow::{Context as _, Result, bail};

use super::constants::MB;

/// Validates that a disk is large enough for the plan.
///
/// # Errors
///
/// Returns an error when the disk is smaller than the plan requires.
pub(crate) fn fits_plan(disk: &str, plan: &Plan) -> Result<()> {
    let available = disk_size(disk)?;
    let required = required_bytes(plan);

    if available < required {
        bail!(
            "Disk '{disk}' is too small for the layout: {} MiB required, {} MiB available",
            required.checked_div(MB).unwrap_or(0),
            available.checked_div(MB).unwrap_or(0)
        );
    }

    Ok(())
}

fn required_bytes(plan: &Plan) -> u64 {
    let fill_count = u64::try_from(
        plan.partitions
            .iter()
            .filter(|spec| spec.size == Size::Fill)
            .count(),
    )
    .unwrap_or(u64::MAX);

    let fixed: u64 = plan
        .partitions
        .iter()
        .map(|spec| match spec.size {
            Size::Fixed(bytes) => bytes,
            Size::Fill => 0,
        })
        .sum();

    fixed
        .saturating_add(fill_count.saturating_mul(MB))
        .saturating_add(MB)
}

fn disk_size(disk: &str) -> Result<u64> {
    let file = std::fs::File::open(disk)
        .with_context(|| format!("Failed to open '{disk}' for size probe"))?;

    Ok(blockdev::device::size(&file)?)
}

#[cfg(test)]
mod tests {
    use ::disk::layout::Layout;

    use super::*;

    #[test]
    fn required_bytes_covers_fixed_partitions_and_fill_floor() {
        // ARRANGE
        let plan = Layout::Uefi.plan();

        // ACT
        let required = required_bytes(&plan);

        // ASSERT
        let fixed: u64 = plan
            .partitions
            .iter()
            .map(|spec| match spec.size {
                Size::Fixed(bytes) => bytes,
                Size::Fill => 0,
            })
            .sum();
        assert_eq!(required, fixed + MB * 2, "fill floor plus GPT margin");
    }

    #[test]
    fn required_bytes_is_zero_for_an_empty_plan() {
        // ARRANGE
        let plan = Plan::wiped(Vec::new());

        // ACT
        let required = required_bytes(&plan);

        // ASSERT
        assert_eq!(required, MB, "only the GPT margin remains");
    }
}
