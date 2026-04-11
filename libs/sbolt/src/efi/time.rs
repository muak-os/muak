//! EFI_TIME helpers

use std::time::SystemTime;

use uefi::runtime::{Daylight, Time, TimeParams};

use crate::{Error, Result};

/// Create an EFI_TIME structure for the current time
pub fn now() -> Result<Time> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();

    let days = secs / 86400;
    let time_of_day = secs % 86400;

    let hour = (time_of_day / 3600) as u8;
    let minute = ((time_of_day % 3600) / 60) as u8;
    let second = (time_of_day % 60) as u8;

    let (year, month, day) = days_to_ymd(days);

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
    .map_err(|e| Error::EfiVar(format!("invalid time parameters: {e}")))
}

/// Convert days since Unix epoch to year, month, day
fn days_to_ymd(days: u64) -> (u16, u8, u8) {
    // Algorithm from Howard Hinnant's date algorithms
    // http://howardhinnant.github.io/date_algorithms.html

    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    (y as u16, m as u8, d as u8)
}

/// Convert Time to raw bytes for authenticated variable signing
pub fn to_bytes(time: &Time) -> [u8; 16] {
    let mut bytes = [0u8; 16];

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
        let result = days_to_ymd(days);
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
    fn to_bytes_known_time() -> Result<()> {
        // ARRANGE
        let time = Time::new(TimeParams {
            year: 2024,
            month: 7,
            day: 15,
            hour: 10,
            minute: 30,
            second: 45,
            nanosecond: 123456789,
            time_zone: Some(60),
            daylight: Daylight::IN_DAYLIGHT,
        })
        .map_err(|e| Error::EfiVar(format!("{e:?}")))?;

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
            123456789
        );
        assert_eq!(i16::from_le_bytes([bytes[12], bytes[13]]), 60);
        assert_eq!(bytes[14], Daylight::IN_DAYLIGHT.bits());
        assert_eq!(bytes[15], 0);
        Ok(())
    }

    #[test]
    fn to_bytes_zero_timezone() -> Result<()> {
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
        .map_err(|e| Error::EfiVar(format!("{e:?}")))?;

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
        Ok(())
    }
}
