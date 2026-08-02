const KB: u64 = 1024;
const MB: u64 = 1024 * KB;
const GB: u64 = 1024 * MB;
const TB: u64 = 1024 * GB;

/// Formats bytes into a human-readable size string.
pub fn format_size(bytes: u64) -> String {
    if bytes >= TB {
        format_with_unit(bytes, TB, 2, "TB")
    } else if bytes >= GB {
        format_with_unit(bytes, GB, 2, "GB")
    } else if bytes >= MB {
        format_with_unit(bytes, MB, 0, "MB")
    } else if bytes >= KB {
        format_with_unit(bytes, KB, 0, "KB")
    } else {
        format!("{bytes}B")
    }
}

/// Formats `bytes` as a value in `unit`, keeping `decimals` fraction digits.
fn format_with_unit(bytes: u64, unit: u64, decimals: u32, suffix: &str) -> String {
    let whole = bytes.div_euclid(unit);
    if decimals == 0 {
        return format!("{whole}{suffix}");
    }
    let factor = 10_u64.pow(decimals);
    let fraction = bytes.rem_euclid(unit).wrapping_mul(factor).div_euclid(unit);
    format!(
        "{whole}.{fraction:0width$}{suffix}",
        width = usize::try_from(decimals).unwrap_or(0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(1), "1B");
        assert_eq!(format_size(1023), "1023B");
    }

    #[test]
    fn kilobytes() {
        assert_eq!(format_size(1024), "1KB");
        assert_eq!(format_size(2048), "2KB");
        assert_eq!(format_size(1024 * 1023), "1023KB");
    }

    #[test]
    fn megabytes() {
        assert_eq!(format_size(1024 * 1024), "1MB");
        assert_eq!(format_size(5 * 1024 * 1024), "5MB");
    }

    #[test]
    fn gigabytes() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00GB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.00GB");
    }

    #[test]
    fn terabytes() {
        let tb = 1024_u64 * 1024 * 1024 * 1024;
        assert_eq!(format_size(tb), "1.00TB");
        assert_eq!(format_size(2 * tb), "2.00TB");
    }

    #[test]
    fn boundary_kb_to_mb() {
        // ARRANGE
        let below_mb = 1024 * 1024 - 1;
        let at_mb = 1024 * 1024;

        // ACT & ASSERT
        assert!(format_size(below_mb).ends_with("KB"));
        assert_eq!(format_size(at_mb), "1MB");
    }
}
