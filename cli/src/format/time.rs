//! Timestamp formatting utilities.

use std::time::{Duration, UNIX_EPOCH};

/// Separator style for timestamp formatting.
pub enum TimeSeparator {
    /// Display format: "2024-01-15 14:30:00"
    Display,
    /// Filename format: "2024-01-15_14-30-00"
    Filename,
}

/// Formats a Unix timestamp into a human-readable string.
pub fn format_timestamp(timestamp: i64, separator: TimeSeparator) -> String {
    let duration = Duration::from_secs(timestamp as u64);
    let system_time = UNIX_EPOCH + duration;

    let duration_since_epoch = system_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = duration_since_epoch.as_secs();

    let days_since_epoch = secs / 86400;
    let seconds_today = secs % 86400;

    let mut year = 1970;
    let mut days_left = days_since_epoch;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_left >= days_in_year {
            days_left -= days_in_year;
            year += 1;
        } else {
            break;
        }
    }

    let days_in_months = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for &days_in_month in &days_in_months {
        if days_left >= days_in_month as u64 {
            days_left -= days_in_month as u64;
            month += 1;
        } else {
            break;
        }
    }

    let day = days_left + 1;
    let hour = seconds_today / 3600;
    let minute = (seconds_today % 3600) / 60;
    let second = seconds_today % 60;

    match separator {
        TimeSeparator::Display => {
            format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
        }
        TimeSeparator::Filename => {
            format!("{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
        }
    }
}

/// Determines if a year is a leap year.
fn is_leap_year(year: u64) -> bool {
    year.is_multiple_of(4) && !year.is_multiple_of(100) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch() {
        assert_eq!(
            format_timestamp(0, TimeSeparator::Display),
            "1970-01-01 00:00:00"
        );
    }

    #[test]
    fn known_date_display() {
        // ARRANGE & ACT
        let result = format_timestamp(1705329000, TimeSeparator::Display);

        // ASSERT
        assert_eq!(result, "2024-01-15 14:30:00");
    }

    #[test]
    fn known_date_filename() {
        // ARRANGE & ACT
        let result = format_timestamp(1705329000, TimeSeparator::Filename);

        // ASSERT
        assert_eq!(result, "2024-01-15_14-30-00");
    }

    #[test]
    fn leap_year_feb_29() {
        // ARRANGE & ACT
        let result = format_timestamp(951782400, TimeSeparator::Display);

        // ASSERT
        assert_eq!(result, "2000-02-29 00:00:00");
    }

    #[test]
    fn non_leap_century_year() {
        assert_eq!(
            format_timestamp(1709251200, TimeSeparator::Display),
            "2024-03-01 00:00:00"
        );
    }

    #[test]
    fn year_boundary_new_years_eve() {
        // ARRANGE & ACT
        let result = format_timestamp(1704067199, TimeSeparator::Display);

        // ASSERT
        assert_eq!(result, "2023-12-31 23:59:59");
    }

    #[test]
    fn year_boundary_new_years_day() {
        // ARRANGE & ACT
        let result = format_timestamp(1704067200, TimeSeparator::Display);

        // ASSERT
        assert_eq!(result, "2024-01-01 00:00:00");
    }

    #[test]
    fn midnight_fields() {
        // ARRANGE & ACT
        let s = format_timestamp(0, TimeSeparator::Display);

        // ASSERT
        assert!(s.ends_with("00:00:00"));
    }

    #[test]
    fn end_of_day_fields() {
        // ARRANGE & ACT
        let result = format_timestamp(86399, TimeSeparator::Display);

        // ASSERT
        assert_eq!(result, "1970-01-01 23:59:59");
    }
}
