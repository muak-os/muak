//! Construction of `ICMPv6` Router Solicitation messages.

use core::net::Ipv6Addr;

use ip::v6;

use super::ROUTER_SOLICITATION;

const ND_OPT_SOURCE_LL_ADDR: u8 = 1;
const ALL_ROUTERS: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 2);
const NEXT_HEADER_ICMPV6: u8 = 58;

/// Constructs an `ICMPv6` Router Solicitation packet.
#[must_use = "dropping it means no solicitation is sent"]
pub fn router_solicitation(source: Ipv6Addr, mac: &[u8; 6]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(16);

    pkt.push(ROUTER_SOLICITATION);
    pkt.push(0);
    pkt.extend([0, 0]);

    pkt.extend([0, 0, 0, 0]);

    if !source.is_unspecified() {
        pkt.push(ND_OPT_SOURCE_LL_ADDR);
        pkt.push(1);
        pkt.extend(mac);
    }

    let checksum = v6::sum(source, ALL_ROUTERS, NEXT_HEADER_ICMPV6, &pkt);
    if let Some(field) = pkt.get_mut(2..4) {
        field.copy_from_slice(&checksum.to_be_bytes());
    }

    pkt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Ipv6Addr {
        Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)
    }

    fn unspecified() -> Ipv6Addr {
        Ipv6Addr::UNSPECIFIED
    }

    #[test]
    fn router_solicitation_structure() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let with_option = [
            ROUTER_SOLICITATION,
            0,
            0x15,
            0x94,
            0,
            0,
            0,
            0,
            ND_OPT_SOURCE_LL_ADDR,
            1,
            0xaa,
            0xbb,
            0xcc,
            0xdd,
            0xee,
            0xff,
        ];
        let without_option = [ROUTER_SOLICITATION, 0, 0x7b, 0xb8, 0, 0, 0, 0];

        // ACT
        let link_local = router_solicitation(source(), &mac);
        let unspecified_source = router_solicitation(unspecified(), &mac);

        // ASSERT
        assert_eq!(link_local.as_slice(), with_option.as_slice());
        assert_eq!(unspecified_source.as_slice(), without_option.as_slice());
    }

    #[test]
    fn router_solicitation_length() {
        // ARRANGE
        let mac = [0; 6];

        // ACT
        let link_local = router_solicitation(source(), &mac);
        let unspecified_source = router_solicitation(unspecified(), &mac);

        // ASSERT
        assert_eq!(link_local.len(), 16);
        assert_eq!(unspecified_source.len(), 8);
    }

    #[test]
    fn router_solicitation_checksum_validates() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let pkt = router_solicitation(source(), &mac);

        // ACT
        let total = v6::sum(source(), ALL_ROUTERS, NEXT_HEADER_ICMPV6, &pkt);

        // ASSERT
        assert_eq!(total, 0, "ICMPv6 checksum must validate to zero");
    }

    #[test]
    fn router_solicitation_checksum_validates_without_source_option() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let pkt = router_solicitation(unspecified(), &mac);

        // ACT
        let total = v6::sum(unspecified(), ALL_ROUTERS, NEXT_HEADER_ICMPV6, &pkt);

        // ASSERT
        assert_eq!(total, 0, "ICMPv6 checksum must validate to zero");
    }
}
