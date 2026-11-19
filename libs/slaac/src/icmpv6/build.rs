//! Construction of `ICMPv6` Router Solicitation messages.

use super::ROUTER_SOLICITATION;

const ND_OPT_SOURCE_LL_ADDR: u8 = 1;

/// Constructs an `ICMPv6` Router Solicitation packet with a source link-layer address option.
#[must_use = "dropping it means no solicitation is sent"]
pub fn router_solicitation(mac: &[u8; 6]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(16);

    pkt.push(ROUTER_SOLICITATION);
    pkt.push(0);
    pkt.extend([0, 0]);

    pkt.extend([0, 0, 0, 0]);

    pkt.push(ND_OPT_SOURCE_LL_ADDR);
    pkt.push(1);
    pkt.extend(mac);

    pkt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_solicitation_structure() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

        // ACT
        let pkt = router_solicitation(&mac);

        // ASSERT
        let expected = [
            ROUTER_SOLICITATION,
            0,
            0,
            0,
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
        assert_eq!(pkt.as_slice(), expected.as_slice());
    }

    #[test]
    fn router_solicitation_length() {
        // ARRANGE
        let mac = [0; 6];

        // ACT
        let pkt = router_solicitation(&mac);

        // ASSERT
        assert_eq!(pkt.len(), 16);
    }
}
