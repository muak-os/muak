//! `EFI_TIME` helpers.

use std::time::SystemTime;

use uefi::runtime::{Daylight, Time, TimeParams};

use crate::error::{Result, SboltError};

const SECONDS_PER_MINUTE: u64 = 60;
const MINUTES_PER_HOUR: u64 = 60;
const HOURS_PER_DAY: u64 = 24;
const SECONDS_PER_HOUR: u64 = SECONDS_PER_MINUTE * MINUTES_PER_HOUR;
const SECONDS_PER_DAY: u64 = SECONDS_PER_HOUR * HOURS_PER_DAY;
const DAYS_PER_ERA: u64 = 146_097;
const DAYS_PER_SUBERA: u64 = 36_524;
const DAYS_PER_QUADRENNIUM: u64 = 1_460;
const DAYS_PER_COMMON_YEAR: u64 = 365;
const MONTHS_PER_CYCLE: u64 = 153;
const MONTH_SCALE: u64 = 5;
const CIVIL_FROM_UNIX_EPOCH_OFFSET: u64 = 719_468;

/// Create an `EFI_TIME` structure for the current time.
///
/// # Errors
///
/// Returns an error if the computed calendar values cannot be represented as a
/// valid UEFI time.
pub fn now() -> Result<Time> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();

    let days = secs.div_euclid(SECONDS_PER_DAY);
    let time_of_day = secs.rem_euclid(SECONDS_PER_DAY);

    let hour = u8::try_from(time_of_day.div_euclid(SECONDS_PER_HOUR))
        .map_err(|_hour_error| SboltError::EfiVar("hour out of range".into()))?;
    let minute = u8::try_from(
        time_of_day
            .rem_euclid(SECONDS_PER_HOUR)
            .div_euclid(SECONDS_PER_MINUTE),
    )
    .map_err(|_minute_error| SboltError::EfiVar("minute out of range".into()))?;
    let second = u8::try_from(time_of_day.rem_euclid(SECONDS_PER_MINUTE))
        .map_err(|_second_error| SboltError::EfiVar("second out of range".into()))?;

    let (year, month, day) = days_to_ymd(days)?;

    Time::new(TimeParams {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanosecond: 0,
        time_zone: Some(0),
        daylight: Daylight::empty(),
    })
    .map_err(|e| SboltError::EfiVar(format!("invalid time parameters: {e}")))
}

/// Convert days since Unix epoch to year, month, day.
fn days_to_ymd(days: u64) -> Result<(u16, u8, u8)> {
    let shifted_days = days
        .checked_add(CIVIL_FROM_UNIX_EPOCH_OFFSET)
        .ok_or_else(|| SboltError::EfiVar("day offset overflow".into()))?;
    let era = shifted_days.div_euclid(DAYS_PER_ERA);
    let era_days = era
        .checked_mul(DAYS_PER_ERA)
        .ok_or_else(|| SboltError::EfiVar("era day count overflow".into()))?;
    let day_of_era = shifted_days
        .checked_sub(era_days)
        .ok_or_else(|| SboltError::EfiVar("day-of-era underflow".into()))?;
    let adjusted_year_days = day_of_era
        .checked_sub(day_of_era.div_euclid(DAYS_PER_QUADRENNIUM))
        .and_then(|value| value.checked_add(day_of_era.div_euclid(DAYS_PER_SUBERA)))
        .and_then(|value| value.checked_sub(day_of_era.div_euclid(DAYS_PER_ERA - 1)))
        .ok_or_else(|| SboltError::EfiVar("year-of-era arithmetic overflow".into()))?;
    let year_of_era = adjusted_year_days.div_euclid(DAYS_PER_COMMON_YEAR);
    let year = year_of_era
        .checked_add(
            era.checked_mul(400)
                .ok_or_else(|| SboltError::EfiVar("calendar year overflow".into()))?,
        )
        .ok_or_else(|| SboltError::EfiVar("calendar year overflow".into()))?;
    let elapsed_year_days = DAYS_PER_COMMON_YEAR
        .checked_mul(year_of_era)
        .and_then(|value| value.checked_add(year_of_era.div_euclid(4)))
        .and_then(|value| value.checked_sub(year_of_era.div_euclid(100)))
        .ok_or_else(|| SboltError::EfiVar("day-of-year arithmetic overflow".into()))?;
    let day_of_year = day_of_era
        .checked_sub(elapsed_year_days)
        .ok_or_else(|| SboltError::EfiVar("day-of-year underflow".into()))?;
    let month_prime = MONTH_SCALE
        .checked_mul(day_of_year)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| SboltError::EfiVar("month calculation overflow".into()))?
        .div_euclid(MONTHS_PER_CYCLE);
    let month_offset = MONTHS_PER_CYCLE
        .checked_mul(month_prime)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| SboltError::EfiVar("day calculation overflow".into()))?
        .div_euclid(MONTH_SCALE);
    let day = day_of_year
        .checked_sub(month_offset)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| SboltError::EfiVar("day calculation overflow".into()))?;
    let month = if month_prime < 10 {
        month_prime
            .checked_add(3)
            .ok_or_else(|| SboltError::EfiVar("month overflow".into()))?
    } else {
        month_prime
            .checked_sub(9)
            .ok_or_else(|| SboltError::EfiVar("month underflow".into()))?
    };
    let adjusted_year = if month <= 2 {
        year.checked_add(1)
            .ok_or_else(|| SboltError::EfiVar("calendar year overflow".into()))?
    } else {
        year
    };

    Ok((
        u16::try_from(adjusted_year)
            .map_err(|_year_error| SboltError::EfiVar("calendar year exceeds u16".into()))?,
        u8::try_from(month)
            .map_err(|_month_error| SboltError::EfiVar("calendar month exceeds u8".into()))?,
        u8::try_from(day)
            .map_err(|_day_error| SboltError::EfiVar("calendar day exceeds u8".into()))?,
    ))
}

/// Convert `Time` to raw bytes for authenticated variable signing.
#[must_use]
pub fn to_bytes(time: &Time) -> [u8; 16] {
    let mut bytes = [0_u8; 16];

    bytes[0..2].copy_from_slice(&time.year().to_le_bytes());
    bytes[2] = time.month();
    bytes[3] = time.day();
    bytes[4] = time.hour();
    bytes[5] = time.minute();
    bytes[6] = time.second();
    bytes[7] = 0;
    bytes[8..12].copy_from_slice(&time.nanosecond().to_le_bytes());
    let tz = time.time_zone().unwrap_or(0);
    bytes[12..14].copy_from_slice(&tz.to_le_bytes());
    bytes[14] = time.daylight().bits();
    bytes[15] = 0;

    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_ymd(days: u64, expected: (u16, u8, u8)) {
        let result = days_to_ymd(days).expect("valid calendar date");
        assert_eq!(
            result, expected,
            "days_to_ymd({days}): expected {expected:?}, got {result:?}"
        );
    }

    #[test]
    fn days_to_ymd_unix_epoch() {
        // ACT & ASSERT
        assert_ymd(0, (1970, 1, 1));
    }

    #[test]
    fn days_to_ymd_day_one() {
        // ACT & ASSERT
        assert_ymd(1, (1970, 1, 2));
    }

    #[test]
    fn days_to_ymd_end_of_first_year() {
        // ACT & ASSERT
        assert_ymd(364, (1970, 12, 31));
    }

    #[test]
    fn days_to_ymd_start_of_1971() {
        // ACT & ASSERT
        assert_ymd(365, (1971, 1, 1));
    }

    #[test]
    fn days_to_ymd_start_of_1972() {
        // ACT & ASSERT
        assert_ymd(730, (1972, 1, 1));
    }

    #[test]
    fn days_to_ymd_leap_day_1972() {
        // ACT & ASSERT
        assert_ymd(789, (1972, 2, 29));
    }

    #[test]
    fn days_to_ymd_march_1_1972() {
        // ACT & ASSERT
        assert_ymd(790, (1972, 3, 1));
    }

    #[test]
    fn days_to_ymd_y2k() {
        // ACT & ASSERT
        assert_ymd(10957, (2000, 1, 1));
    }

    #[test]
    fn days_to_ymd_2020() {
        // ACT & ASSERT
        assert_ymd(18262, (2020, 1, 1));
    }

    #[test]
    fn days_to_ymd_2024_leap_day() {
        // ACT & ASSERT
        assert_ymd(19723 + 59, (2024, 2, 29));
    }

    #[test]
    fn days_to_ymd_2024_march_1() {
        // ACT & ASSERT
        assert_ymd(19723 + 60, (2024, 3, 1));
    }

    #[test]
    fn to_bytes_known_time() {
        // ARRANGE
        let time = Time::new(TimeParams {
            year: 2024,
            month: 7,
            day: 15,
            hour: 10,
            minute: 30,
            second: 45,
            nanosecond: 123_456_789,
            time_zone: Some(60),
            daylight: Daylight::IN_DAYLIGHT,
        })
        .expect("valid time");

        // ACT
        let bytes = to_bytes(&time);

        // ASSERT
        assert_eq!(bytes.len(), 16);

        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 2024);
        assert_eq!(bytes[2], 7);
        assert_eq!(bytes[3], 15);
        assert_eq!(bytes[4], 10);
        assert_eq!(bytes[5], 30);
        assert_eq!(bytes[6], 45);
        assert_eq!(bytes[7], 0);
        assert_eq!(
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            123_456_789
        );
        assert_eq!(i16::from_le_bytes([bytes[12], bytes[13]]), 60);
        assert_eq!(bytes[14], Daylight::IN_DAYLIGHT.bits());
        assert_eq!(bytes[15], 0);
    }

    #[test]
    fn to_bytes_zero_timezone() {
        // ARRANGE
        let time = Time::new(TimeParams {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            nanosecond: 0,
            time_zone: Some(0),
            daylight: Daylight::empty(),
        })
        .expect("valid time");

        // ACT
        let bytes = to_bytes(&time);

        // ASSERT
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 1970);
        assert_eq!(bytes[2], 1);
        assert_eq!(bytes[3], 1);
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
        assert_eq!(&bytes[8..12], &[0, 0, 0, 0]);
        assert_eq!(&bytes[12..14], &[0, 0]);
        assert_eq!(bytes[14], 0);
        assert_eq!(bytes[15], 0);
    }

    #[test]
    fn to_bytes_defaults_missing_timezone_to_zero() {
        // ARRANGE
        let time = Time::new(TimeParams {
            year: 2026,
            month: 5,
            day: 28,
            hour: 1,
            minute: 2,
            second: 3,
            nanosecond: 4,
            time_zone: None,
            daylight: Daylight::empty(),
        })
        .expect("valid time");

        // ACT
        let bytes = to_bytes(&time);

        // ASSERT
        assert_eq!(&bytes[12..14], &[0, 0]);
    }

    #[test]
    fn days_to_ymd_rejects_unrepresentable_future_year() {
        // ARRANGE
        let days = u64::MAX - CIVIL_FROM_UNIX_EPOCH_OFFSET + 1;

        // ACT
        let result = days_to_ymd(days);

        // ASSERT
        result.expect_err("future year should be unrepresentable");
    }

    #[test]
    fn now_returns_valid_time() {
        // ACT
        let time = now().expect("current EFI time");

        // ASSERT
        assert!(time.year() >= 2024);
        assert!((1..=12).contains(&time.month()));
        assert!((1..=31).contains(&time.day()));
        assert!(time.hour() < 24);
        assert!(time.minute() < 60);
        assert!(time.second() < 60);
        assert_eq!(time.time_zone(), Some(0));
    }
}
