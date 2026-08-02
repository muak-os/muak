//! Timestamp formatting utilities.

use core::time::Duration;
use std::time::UNIX_EPOCH;

/// Separator style for timestamp formatting.
#[derive(Clone, Copy)]
pub enum Separator {
    /// Display format: "2024-01-15 14:30:00".
    Display,
    /// Filename format: "2024-01-15_14-30-00".
    Filename,
}

/// Formats a Unix timestamp into a human-readable string.
pub fn format_timestamp(timestamp: i64, separator: Separator) -> String {
    let duration = Duration::from_secs(u64::try_from(timestamp).unwrap_or(0));
    let system_time = UNIX_EPOCH.checked_add(duration).unwrap_or(UNIX_EPOCH);

    let duration_since_epoch = system_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = duration_since_epoch.as_secs();

    let days_since_epoch = secs.div_euclid(86400);
    let seconds_today = secs.rem_euclid(86400);

    let mut year = 1970;
    let mut days_left = days_since_epoch;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_left >= days_in_year {
            days_left = days_left.saturating_sub(days_in_year);
            year = year.saturating_add(1);
        } else {
            break;
        }
    }

    let days_in_months = if is_leap_year(year) {
        [31_u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31_u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month: u32 = 1;
    for &days_in_month in &days_in_months {
        if days_left >= days_in_month {
            days_left = days_left.saturating_sub(days_in_month);
            month = month.saturating_add(1);
        } else {
            break;
        }
    }

    let day = days_left.saturating_add(1);
    let hour = seconds_today.div_euclid(3600);
    let minute = seconds_today.rem_euclid(3600).div_euclid(60);
    let second = seconds_today.rem_euclid(60);

    match separator {
        Separator::Display => {
            format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
        }
        Separator::Filename => {
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
            format_timestamp(0, Separator::Display),
            "1970-01-01 00:00:00"
        );
    }

    #[test]
    fn known_date_display() {
        // ARRANGE & ACT
        let result = format_timestamp(1_705_329_000, Separator::Display);

        // ASSERT
        assert_eq!(result, "2024-01-15 14:30:00");
    }

    #[test]
    fn known_date_filename() {
        // ARRANGE & ACT
        let result = format_timestamp(1_705_329_000, Separator::Filename);

        // ASSERT
        assert_eq!(result, "2024-01-15_14-30-00");
    }

    #[test]
    fn leap_year_feb_29() {
        // ARRANGE & ACT
        let result = format_timestamp(951_782_400, Separator::Display);

        // ASSERT
        assert_eq!(result, "2000-02-29 00:00:00");
    }

    #[test]
    fn non_leap_century_year() {
        assert_eq!(
            format_timestamp(1_709_251_200, Separator::Display),
            "2024-03-01 00:00:00"
        );
    }

    #[test]
    fn year_boundary_new_years_eve() {
        // ARRANGE & ACT
        let result = format_timestamp(1_704_067_199, Separator::Display);

        // ASSERT
        assert_eq!(result, "2023-12-31 23:59:59");
    }

    #[test]
    fn year_boundary_new_years_day() {
        // ARRANGE & ACT
        let result = format_timestamp(1_704_067_200, Separator::Display);

        // ASSERT
        assert_eq!(result, "2024-01-01 00:00:00");
    }

    #[test]
    fn midnight_fields() {
        // ARRANGE & ACT
        let result = format_timestamp(0, Separator::Display);

        // ASSERT
        assert!(result.ends_with("00:00:00"));
    }

    #[test]
    fn end_of_day_fields() {
        // ARRANGE & ACT
        let result = format_timestamp(86399, Separator::Display);

        // ASSERT
        assert_eq!(result, "1970-01-01 23:59:59");
    }
}
