//! RFC 894 Ethernet II frame codec.

#![warn(missing_docs)]

use thiserror::Error;

/// Length of a link-layer address in bytes.
pub const ALEN: usize = 6;

/// Length of an Ethernet II header in bytes.
pub const HEADER_LEN: usize = 14;

/// Link-layer broadcast address.
pub const BROADCAST: [u8; ALEN] = [0xff; ALEN];

/// `EtherType` values carried by this codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EtherType {
    /// IPv4, RFC 791.
    Ipv4,
    /// IPv6, RFC 8200.
    Ipv6,
    /// ARP, RFC 826.
    Arp,
}

impl EtherType {
    /// Returns the `EtherType` value as it appears on the wire.
    #[must_use = "callers need the wire value"]
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::Ipv4 => 0x0800,
            Self::Ipv6 => 0x86dd,
            Self::Arp => 0x0806,
        }
    }

    /// Returns the `EtherType` for `value`, or `None` when this codec does not carry it.
    #[must_use = "callers need to detect unhandled ether types"]
    pub const fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0800 => Some(Self::Ipv4),
            0x86dd => Some(Self::Ipv6),
            0x0806 => Some(Self::Arp),
            _ => None,
        }
    }
}

/// Ethernet II header, as defined by RFC 894.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Link-layer destination.
    pub destination: [u8; ALEN],
    /// Link-layer source.
    pub source: [u8; ALEN],
    /// Payload protocol.
    pub ether_type: EtherType,
}

impl Header {
    /// Builds a header from its fields.
    #[must_use = "callers need the header to write it"]
    pub const fn new(destination: [u8; ALEN], source: [u8; ALEN], ether_type: EtherType) -> Self {
        Self {
            destination,
            source,
            ether_type,
        }
    }

    /// Parses the header at the start of `frame` and returns it with the payload that follows.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Truncated`] when `frame` is shorter than [`HEADER_LEN`] and
    /// [`Failure::UnknownEtherType`] when the `EtherType` is not one this codec carries.
    pub fn parse(frame: &[u8]) -> Result<(Self, &[u8]), Failure> {
        let (head, payload) = frame
            .split_at_checked(HEADER_LEN)
            .ok_or(Failure::Truncated(frame.len(), HEADER_LEN))?;
        let wire = word(head, 2 * ALEN);
        let ether_type = EtherType::from_u16(wire).ok_or(Failure::UnknownEtherType(wire))?;

        Ok((
            Self {
                destination: mac(head, 0),
                source: mac(head, ALEN),
                ether_type,
            },
            payload,
        ))
    }

    /// Writes the header at the start of `frame` and returns the payload area that follows.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::TooSmall`] when `frame` cannot hold [`HEADER_LEN`] bytes.
    pub fn write(self, frame: &mut [u8]) -> Result<&mut [u8], Failure> {
        let (head, payload) = frame
            .split_at_mut_checked(HEADER_LEN)
            .ok_or(Failure::TooSmall(HEADER_LEN))?;
        put(head, 0, &self.destination);
        put(head, ALEN, &self.source);
        put(head, 2 * ALEN, &self.ether_type.as_u16().to_be_bytes());

        Ok(payload)
    }
}

/// Ethernet II codec failures.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum Failure {
    /// Frame is shorter than the header it must hold.
    #[error("truncated frame: {0} bytes available, {1} bytes required")]
    Truncated(usize, usize),
    /// `EtherType` is not one this codec carries.
    #[error("unknown EtherType {0:#06x}")]
    UnknownEtherType(u16),
    /// Caller supplied buffer cannot hold a header.
    #[error("buffer too small: {0} bytes required")]
    TooSmall(usize),
}

// Reads a link-layer address at `offset` from an already length-checked header.
fn mac(head: &[u8], offset: usize) -> [u8; ALEN] {
    let mut value = [0_u8; ALEN];
    if let Some(bytes) = head.get(offset..offset.saturating_add(ALEN)) {
        value.copy_from_slice(bytes);
    }

    value
}

// Reads a big-endian 16 bit word at `offset` from an already length-checked header.
fn word(head: &[u8], offset: usize) -> u16 {
    let mut value = [0_u8; 2];
    if let Some(bytes) = head.get(offset..offset.saturating_add(2)) {
        value.copy_from_slice(bytes);
    }

    u16::from_be_bytes(value)
}

// Writes `bytes` at `offset` into an already length-checked header.
fn put(head: &mut [u8], offset: usize, bytes: &[u8]) {
    if let Some(slot) = head.get_mut(offset..offset.saturating_add(bytes.len())) {
        slot.copy_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints() -> ([u8; ALEN], [u8; ALEN]) {
        (
            [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
            [0x02, 0x00, 0x00, 0x00, 0x00, 0x02],
        )
    }

    #[test]
    fn write_then_parse_round_trips_header_and_payload() {
        // ARRANGE
        let (destination, source) = endpoints();
        let header = Header::new(destination, source, EtherType::Ipv4);
        let payload = [0x45_u8, 0x00, 0x00];
        let mut frame = [0_u8; HEADER_LEN + 3];

        // ACT
        let area = header
            .write(&mut frame)
            .expect("frame holds the header and payload");
        area.copy_from_slice(&payload);
        let (parsed, data) = Header::parse(&frame).expect("parse reads a written frame");

        // ASSERT
        assert_eq!(parsed, header);
        assert_eq!(data, payload.as_slice());
    }

    #[test]
    fn broadcast_destination_round_trips() {
        // ARRANGE
        let (_, source) = endpoints();
        let header = Header::new(BROADCAST, source, EtherType::Arp);
        let mut frame = [0_u8; HEADER_LEN];

        // ACT
        let _area = header.write(&mut frame).expect("frame holds the header");
        let (parsed, _data) = Header::parse(&frame).expect("parse reads a written frame");

        // ASSERT
        assert_eq!(parsed.destination, [0xff; ALEN]);
    }

    #[test]
    fn ether_types_use_the_rfc_894_values() {
        // ARRANGE
        let values = [
            (EtherType::Ipv4, 0x0800_u16),
            (EtherType::Ipv6, 0x86dd_u16),
            (EtherType::Arp, 0x0806_u16),
        ];

        // ACT / ASSERT
        for (ether_type, wire) in values {
            assert_eq!(ether_type.as_u16(), wire);
            assert_eq!(EtherType::from_u16(wire), Some(ether_type));
        }
    }

    #[test]
    fn parse_reads_each_ether_type_from_the_wire() {
        // ARRANGE
        let (destination, source) = endpoints();

        // ACT / ASSERT
        for (wire, expected) in [
            (0x0800_u16, EtherType::Ipv4),
            (0x86dd_u16, EtherType::Ipv6),
            (0x0806_u16, EtherType::Arp),
        ] {
            let header = Header::new(destination, source, expected);
            let mut frame = [0_u8; HEADER_LEN];
            header.write(&mut frame).expect("frame holds the header");

            let (parsed, _data) = Header::parse(&frame).expect("parse reads a written frame");

            assert_eq!(parsed.ether_type.as_u16(), wire);
            assert_eq!(parsed.ether_type, expected);
        }
    }

    #[test]
    fn parse_rejects_truncated_frame() {
        // ARRANGE
        let frame = [0_u8; HEADER_LEN - 1];

        // ACT
        let result = Header::parse(&frame);

        // ASSERT
        assert_eq!(result, Err(Failure::Truncated(HEADER_LEN - 1, HEADER_LEN)));
    }

    #[test]
    fn parse_rejects_unknown_ether_type() {
        // ARRANGE
        let (destination, source) = endpoints();
        let header = Header::new(destination, source, EtherType::Ipv4);
        let mut frame = [0_u8; HEADER_LEN];
        header.write(&mut frame).expect("frame holds the header");
        frame
            .get_mut(2 * ALEN..HEADER_LEN)
            .expect("frame has an EtherType field")
            .copy_from_slice(&0x0801_u16.to_be_bytes());

        // ACT
        let result = Header::parse(&frame);

        // ASSERT
        assert_eq!(result, Err(Failure::UnknownEtherType(0x0801)));
    }

    #[test]
    fn write_rejects_too_small_buffer() {
        // ARRANGE
        let (destination, source) = endpoints();
        let header = Header::new(destination, source, EtherType::Ipv4);
        let mut frame = [0_u8; HEADER_LEN - 1];

        // ACT
        let result = header.write(&mut frame);

        // ASSERT
        assert_eq!(result, Err(Failure::TooSmall(HEADER_LEN)));
    }

    #[test]
    fn from_u16_rejects_unhandled_ether_types() {
        // ARRANGE
        let value = 0x8100_u16;

        // ACT
        let ether_type = EtherType::from_u16(value);

        // ASSERT
        assert_eq!(ether_type, None);
    }
}
