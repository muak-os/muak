//! Raw DHCPv4 packet construction and field extraction (RFC 2131).

use std::net::Ipv4Addr;

/// DHCPv4 option codes.
pub(crate) mod option {
    pub const SUBNET_MASK: u8 = 1;
    pub const ROUTER: u8 = 3;
    pub const DNS_SERVER: u8 = 6;
    pub const REQUESTED_IP: u8 = 50;
    pub const LEASE_TIME: u8 = 51;
    pub const MESSAGE_TYPE: u8 = 53;
    pub const SERVER_ID: u8 = 54;
    pub const PARAM_REQUEST_LIST: u8 = 55;
    pub const END: u8 = 255;
}

/// DHCPv4 message type values.
pub(crate) mod message_type {
    pub const DISCOVER: u8 = 1;
    pub const OFFER: u8 = 2;
    pub const REQUEST: u8 = 3;
    pub const ACK: u8 = 5;
    pub const NAK: u8 = 6;
}

/// RFC 2131 fixed-header field offsets.
pub(crate) mod field {
    pub const OP: usize = 0;
    pub const HTYPE: usize = 1;
    pub const HLEN: usize = 2;
    pub const HOPS: usize = 3;
    pub const XID: usize = 4;
    pub const FLAGS: usize = 10;
    pub const YIADDR: usize = 16;
    pub const CHADDR: usize = 28;
    pub const HEADER_LEN: usize = 236;
}

pub(crate) const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
pub(crate) const BOOTREQUEST: u8 = 1;
const HTYPE_ETHERNET: u8 = 1;
const HLEN_ETHERNET: u8 = 6;
pub(crate) const FLAG_BROADCAST: u16 = 0x8000;

pub(crate) const DHCP_CLIENT_PORT: u16 = 68;
pub(crate) const DHCP_SERVER_PORT: u16 = 67;

pub(crate) const DHCP_TIMEOUT_SECS: u64 = 10;
pub(crate) const DEFAULT_LEASE_SECS: u32 = 3600;
pub(crate) const DEFAULT_PREFIX_LEN: u8 = 24;

/// Builds the fixed 236-byte DHCP header + magic cookie with broadcast flag set.
pub(crate) fn build_header(xid: u32, mac: &[u8; 6]) -> Vec<u8> {
    let mut buf = vec![0u8; field::HEADER_LEN + MAGIC_COOKIE.len()];

    buf[field::OP] = BOOTREQUEST;
    buf[field::HTYPE] = HTYPE_ETHERNET;
    buf[field::HLEN] = HLEN_ETHERNET;
    buf[field::HOPS] = 0;

    buf[field::XID..field::XID + 4].copy_from_slice(&xid.to_be_bytes());
    buf[field::FLAGS..field::FLAGS + 2].copy_from_slice(&FLAG_BROADCAST.to_be_bytes());

    buf[field::CHADDR..field::CHADDR + 6].copy_from_slice(mac);

    buf[field::HEADER_LEN..field::HEADER_LEN + 4].copy_from_slice(&MAGIC_COOKIE);

    buf
}

/// Builds a header without the broadcast flag set (for unicast renewals).
pub(crate) fn build_unicast_header(xid: u32, mac: &[u8; 6]) -> Vec<u8> {
    let mut buf = build_header(xid, mac);
    buf[field::FLAGS..field::FLAGS + 2].copy_from_slice(&0u16.to_be_bytes());
    buf
}

/// Appends the parameter request list option to a DHCP message.
pub(crate) fn append_param_request_list(msg: &mut Vec<u8>) {
    const REQUESTED_PARAMS: &[u8] = &[
        option::SUBNET_MASK,
        option::ROUTER,
        option::DNS_SERVER,
        option::LEASE_TIME,
    ];
    msg.push(option::PARAM_REQUEST_LIST);
    msg.push(REQUESTED_PARAMS.len() as u8);
    msg.extend_from_slice(REQUESTED_PARAMS);
}

/// Builds a DHCP REQUEST message for renewal or rebinding.
pub(crate) fn build_request_message(
    xid: u32,
    mac: &[u8; 6],
    assigned_ip: Ipv4Addr,
    unicast: bool,
) -> Vec<u8> {
    let mut msg = if unicast {
        build_unicast_header(xid, mac)
    } else {
        build_header(xid, mac)
    };
    msg.extend(&[option::MESSAGE_TYPE, 1, message_type::REQUEST]);
    msg.extend(&[option::REQUESTED_IP, 4]);
    msg.extend(&assigned_ip.octets());
    append_param_request_list(&mut msg);
    msg.push(option::END);
    msg
}

/// Extracts `yiaddr` from a raw DHCP packet.
pub(crate) fn yiaddr(buf: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(
        buf[field::YIADDR],
        buf[field::YIADDR + 1],
        buf[field::YIADDR + 2],
        buf[field::YIADDR + 3],
    )
}

/// Extracts `xid` from a raw DHCP packet.
pub(crate) fn xid(buf: &[u8]) -> u32 {
    u32::from_be_bytes([
        buf[field::XID],
        buf[field::XID + 1],
        buf[field::XID + 2],
        buf[field::XID + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_minimal_packet(xid_val: u32, yiaddr_val: Ipv4Addr) -> Vec<u8> {
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let mut pkt = build_header(xid_val, &mac);
        pkt[field::YIADDR..field::YIADDR + 4].copy_from_slice(&yiaddr_val.octets());
        pkt
    }

    #[test]
    fn build_header_length() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

        // ACT
        let hdr = build_header(0x12345678, &mac);

        // ASSERT
        assert_eq!(hdr.len(), field::HEADER_LEN + MAGIC_COOKIE.len());
    }

    #[test]
    fn build_header_fields() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

        // ACT
        let hdr = build_header(0xDEADBEEF, &mac);

        // ASSERT
        assert_eq!(hdr[field::OP], BOOTREQUEST);
        assert_eq!(hdr[field::HTYPE], HTYPE_ETHERNET);
        assert_eq!(hdr[field::HLEN], HLEN_ETHERNET);
        assert_eq!(hdr[field::HOPS], 0);
        let xid_bytes = &hdr[field::XID..field::XID + 4];
        assert_eq!(xid_bytes, &0xDEADBEEFu32.to_be_bytes());
        assert_eq!(&hdr[field::CHADDR..field::CHADDR + 6], &mac);
        let cookie = &hdr[field::HEADER_LEN..field::HEADER_LEN + 4];
        assert_eq!(cookie, &MAGIC_COOKIE);
    }

    #[test]
    fn build_header_broadcast_flag_set() {
        // ARRANGE
        let mac = [0; 6];

        // ACT
        let hdr = build_header(1, &mac);

        // ASSERT
        let flags = u16::from_be_bytes([hdr[field::FLAGS], hdr[field::FLAGS + 1]]);
        assert_eq!(flags, FLAG_BROADCAST);
    }

    #[test]
    fn build_unicast_header_clears_broadcast_flag() {
        // ARRANGE
        let mac = [0; 6];

        // ACT
        let hdr = build_unicast_header(1, &mac);

        // ASSERT
        let flags = u16::from_be_bytes([hdr[field::FLAGS], hdr[field::FLAGS + 1]]);
        assert_eq!(flags, 0);
    }

    #[test]
    fn append_param_request_list_content() {
        // ARRANGE
        let mut msg = Vec::new();

        // ACT
        append_param_request_list(&mut msg);

        // ASSERT
        assert_eq!(msg[0], option::PARAM_REQUEST_LIST);
        assert_eq!(msg[1], 4);
        assert_eq!(msg[2], option::SUBNET_MASK);
        assert_eq!(msg[3], option::ROUTER);
        assert_eq!(msg[4], option::DNS_SERVER);
        assert_eq!(msg[5], option::LEASE_TIME);
    }

    #[test]
    fn build_request_message_unicast_no_broadcast_flag() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let ip = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let msg = build_request_message(0x1234, &mac, ip, true);

        // ASSERT
        let flags = u16::from_be_bytes([msg[field::FLAGS], msg[field::FLAGS + 1]]);
        assert_eq!(flags, 0);
    }

    #[test]
    fn build_request_message_broadcast_has_flag() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let ip = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let msg = build_request_message(0x1234, &mac, ip, false);

        // ASSERT
        let flags = u16::from_be_bytes([msg[field::FLAGS], msg[field::FLAGS + 1]]);
        assert_eq!(flags, FLAG_BROADCAST);
    }

    #[test]
    fn yiaddr_extraction() {
        // ARRANGE
        let pkt = make_minimal_packet(1, Ipv4Addr::new(10, 0, 0, 42));

        // ACT / ASSERT
        assert_eq!(yiaddr(&pkt), Ipv4Addr::new(10, 0, 0, 42));
    }

    #[test]
    fn xid_extraction() {
        // ARRANGE
        let pkt = make_minimal_packet(0xCAFEBABE, Ipv4Addr::UNSPECIFIED);

        // ACT / ASSERT
        assert_eq!(xid(&pkt), 0xCAFEBABE);
    }
}
