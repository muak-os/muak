const KB: u64 = 1024;
const MB: u64 = 1024 * KB;
const GB: u64 = 1024 * MB;
const TB: u64 = 1024 * GB;

/// Formats bytes into a human-readable size string.
pub fn format_size(bytes: u64) -> String {
    if bytes >= TB {
        format!("{:.2}TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}B")
    }
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
        let tb = 1024u64 * 1024 * 1024 * 1024;
        assert_eq!(format_size(tb), "1.00TB");
        assert_eq!(format_size(2 * tb), "2.00TB");
    }

    #[test]
    fn boundary_kb_to_mb() {
        assert!(format_size(1024 * 1024 - 1).ends_with("KB"));
        assert_eq!(format_size(1024 * 1024), "1MB");
    }
}
