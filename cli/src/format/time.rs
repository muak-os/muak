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

fn is_leap_year(year: u64) -> bool {
    year.is_multiple_of(4) && !year.is_multiple_of(100) || year.is_multiple_of(400)
}
