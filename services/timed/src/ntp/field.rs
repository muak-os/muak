//! `NTPv4` header field offsets.

/// Leap Indicator (2 bits) | Version (3 bits) | Mode (3 bits).
pub const LI_VN_MODE: usize = 0;

/// Transmit Timestamp seconds since NTP epoch (1900-01-01).
pub const TX_TIMESTAMP_SECS: usize = 40;

/// Transmit Timestamp fractional seconds.
pub const TX_TIMESTAMP_FRAC: usize = 44;
