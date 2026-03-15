//! SNTP client implementation (RFC 4330 / NTPv4 wire format).
//!
//! Sends a single NTPv4 client request and parses the server response to extract
//! the transmit timestamp. No clock discipline or PLL — just direct time setting.

use std::net::ToSocketAddrs;
use std::time::Duration;

use anyhow::{Result, bail};
use rustix::time::{ClockId, Timespec, clock_settime};
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// NTP packet size in bytes (fixed 48-byte structure).
const NTP_PACKET_SIZE: usize = 48;

/// NTP port per RFC 4330
const NTP_PORT: u16 = 123;

/// Timeout for a single NTP request/response exchange.
const NTP_TIMEOUT: Duration = Duration::from_secs(5);

/// NTP epoch offset: seconds between 1900-01-01 and 1970-01-01 (UNIX epoch).
const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;

/// NTP version 4.
const NTP_VERSION: u8 = 4;

/// NTPv4 header field offsets.
mod field {
    /// Leap Indicator (2 bits) | Version (3 bits) | Mode (3 bits).
    pub const LI_VN_MODE: usize = 0;

    /// Transmit Timestamp — seconds since NTP epoch (1900-01-01).
    pub const TX_TIMESTAMP_SECS: usize = 40;

    /// Transmit Timestamp — fractional seconds.
    pub const TX_TIMESTAMP_FRAC: usize = 44;
}

/// NTP client mode (3).
const MODE_CLIENT: u8 = 3;

/// Build a 48-byte SNTP client request packet.
fn build_request() -> [u8; NTP_PACKET_SIZE] {
    let mut packet = [0u8; NTP_PACKET_SIZE];

    // LI = 0 (no warning), VN = 4 (NTPv4), Mode = 3 (client)
    packet[field::LI_VN_MODE] = (NTP_VERSION << 3) | MODE_CLIENT;

    packet
}

/// Parse the transmit timestamp from an NTP response packet into a UNIX `Timespec`.
fn parse_response(packet: &[u8; NTP_PACKET_SIZE]) -> Result<Timespec> {
    let secs = u32::from_be_bytes(
        packet[field::TX_TIMESTAMP_SECS..field::TX_TIMESTAMP_SECS + 4]
            .try_into()
            .expect("slice is 4 bytes"),
    );

    let frac = u32::from_be_bytes(
        packet[field::TX_TIMESTAMP_FRAC..field::TX_TIMESTAMP_FRAC + 4]
            .try_into()
            .expect("slice is 4 bytes"),
    );

    if secs == 0 {
        bail!("Server returned zero timestamp (kiss-o'-death or unsynchronized)");
    }

    let unix_secs = u64::from(secs)
        .checked_sub(NTP_EPOCH_OFFSET)
        .ok_or_else(|| anyhow::anyhow!("NTP timestamp before Unix epoch"))?;

    let nanos = ((u64::from(frac)) * 1_000_000_000) >> 32;

    Ok(Timespec {
        tv_sec: unix_secs as i64,
        tv_nsec: nanos as i64,
    })
}

/// Perform a single SNTP time synchronization against a given server.
pub async fn sync(server: &str) -> Result<Duration> {
    let addr = format!("{server}:{NTP_PORT}")
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve NTP server: {server}"))?;

    let bind_addr = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };

    let socket = UdpSocket::bind(bind_addr).await?;
    socket.connect(addr).await?;

    let request = build_request();
    socket.send(&request).await?;

    let mut buf = [0u8; NTP_PACKET_SIZE];
    let n = timeout(NTP_TIMEOUT, socket.recv(&mut buf)).await??;

    if n < NTP_PACKET_SIZE {
        bail!("Received truncated NTP response ({n} bytes, expected {NTP_PACKET_SIZE})");
    }

    let server_time = parse_response(&buf)?;

    let before = rustix::time::clock_gettime(ClockId::Realtime);

    clock_settime(ClockId::Realtime, server_time)?;

    let offset_secs = (server_time.tv_sec - before.tv_sec).unsigned_abs();
    let offset = Duration::from_secs(offset_secs);

    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_sets_correct_fields() {
        // ARRANGE
        let packet = build_request();
        assert_eq!(packet.len(), NTP_PACKET_SIZE);

        // ACT
        let li = (packet[0] >> 6) & 0x03;
        let vn = (packet[0] >> 3) & 0x07;
        let mode = packet[0] & 0x07;

        // ASSERT
        assert_eq!(li, 0, "Leap indicator should be 0");
        assert_eq!(vn, 4, "Version should be 4");
        assert_eq!(mode, 3, "Mode should be 3 (client)");
        assert!(packet[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn parse_response_valid() {
        // ARRANGE
        let mut packet = [0u8; NTP_PACKET_SIZE];

        let ntp_secs: u32 = 3_913_056_000;
        packet[field::TX_TIMESTAMP_SECS..field::TX_TIMESTAMP_SECS + 4]
            .copy_from_slice(&ntp_secs.to_be_bytes());

        // ACT
        let ts = parse_response(&packet).expect("Should parse successfully");

        // ASSERT
        assert_eq!(ts.tv_sec, 1_704_067_200);
        assert_eq!(ts.tv_nsec, 0);
    }

    #[test]
    fn parse_response_with_fraction() {
        // ARRANGE
        let mut packet = [0u8; NTP_PACKET_SIZE];

        let ntp_secs: u32 = 3_913_056_000;
        packet[field::TX_TIMESTAMP_SECS..field::TX_TIMESTAMP_SECS + 4]
            .copy_from_slice(&ntp_secs.to_be_bytes());

        let frac: u32 = 2_147_483_648;
        packet[field::TX_TIMESTAMP_FRAC..field::TX_TIMESTAMP_FRAC + 4]
            .copy_from_slice(&frac.to_be_bytes());

        // ACT
        let ts = parse_response(&packet).expect("Should parse successfully");

        // ASSERT
        assert_eq!(ts.tv_sec, 1_704_067_200);
        assert_eq!(ts.tv_nsec, 500_000_000);
    }

    #[test]
    fn parse_response_zero_timestamp() {
        // ARRANGE
        let packet = [0u8; NTP_PACKET_SIZE];

        // ACT
        let result = parse_response(&packet);

        // ASSERT
        assert!(result.is_err(), "Zero timestamp should be rejected");
    }
}
