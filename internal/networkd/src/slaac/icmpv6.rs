use std::net::Ipv6Addr;

use anyhow::{Result, bail};

pub const ICMPV6_ROUTER_SOLICITATION: u8 = 133;
pub const ICMPV6_ROUTER_ADVERTISEMENT: u8 = 134;

const ND_OPT_SOURCE_LL_ADDR: u8 = 1;
const ND_OPT_PREFIX_INFO: u8 = 3;
const ND_OPT_RDNSS: u8 = 25;

#[derive(Debug, Clone)]
pub struct PrefixInfo {
    pub prefix: Ipv6Addr,
    pub prefix_len: u8,
    pub autonomous: bool,
    pub valid_lifetime: u32,
    pub preferred_lifetime: u32,
}

#[derive(Debug, Clone)]
pub struct RouterAdvertisement {
    pub hop_limit: u8,
    pub managed_flag: bool,
    pub other_flag: bool,
    pub router_lifetime: u16,
    pub source: Ipv6Addr,
    pub prefixes: Vec<PrefixInfo>,
    pub dns_servers: Vec<Ipv6Addr>,
    pub dns_lifetime: u32,
}

pub fn build_router_solicitation(mac: &[u8; 6]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(16);

    pkt.push(ICMPV6_ROUTER_SOLICITATION);
    pkt.push(0); // code
    pkt.extend([0, 0]); // checksum placeholder

    pkt.extend([0, 0, 0, 0]); // reserved

    pkt.push(ND_OPT_SOURCE_LL_ADDR);
    pkt.push(1); // length in units of 8 octets
    pkt.extend(mac);

    pkt
}

pub fn parse_router_advertisement(data: &[u8], source: Ipv6Addr) -> Result<RouterAdvertisement> {
    if data.len() < 16 {
        bail!("RA too short: {} bytes", data.len());
    }

    if data[0] != ICMPV6_ROUTER_ADVERTISEMENT {
        bail!("not a Router Advertisement: type={}", data[0]);
    }

    let hop_limit = data[4];
    let flags = data[5];
    let managed_flag = (flags & 0x80) != 0;
    let other_flag = (flags & 0x40) != 0;
    let router_lifetime = u16::from_be_bytes([data[6], data[7]]);

    let mut ra = RouterAdvertisement {
        hop_limit,
        managed_flag,
        other_flag,
        router_lifetime,
        source,
        prefixes: Vec::new(),
        dns_servers: Vec::new(),
        dns_lifetime: 0,
    };

    let mut pos = 16;
    while pos + 2 <= data.len() {
        let opt_type = data[pos];
        let opt_len_units = data[pos + 1] as usize;
        if opt_len_units == 0 {
            break;
        }
        let opt_len = opt_len_units * 8;
        if pos + opt_len > data.len() {
            break;
        }

        match opt_type {
            ND_OPT_PREFIX_INFO if opt_len >= 32 => {
                let prefix_len = data[pos + 2];
                let flags = data[pos + 3];
                let autonomous = (flags & 0x40) != 0;
                let valid_lifetime = u32::from_be_bytes([
                    data[pos + 4],
                    data[pos + 5],
                    data[pos + 6],
                    data[pos + 7],
                ]);
                let preferred_lifetime = u32::from_be_bytes([
                    data[pos + 8],
                    data[pos + 9],
                    data[pos + 10],
                    data[pos + 11],
                ]);
                let prefix_bytes: [u8; 16] = data[pos + 16..pos + 32].try_into()?;
                let prefix = Ipv6Addr::from(prefix_bytes);

                ra.prefixes.push(PrefixInfo {
                    prefix,
                    prefix_len,
                    autonomous,
                    valid_lifetime,
                    preferred_lifetime,
                });
            }
            ND_OPT_RDNSS if opt_len >= 24 => {
                ra.dns_lifetime = u32::from_be_bytes([
                    data[pos + 4],
                    data[pos + 5],
                    data[pos + 6],
                    data[pos + 7],
                ]);
                let addr_count = (opt_len - 8) / 16;
                for i in 0..addr_count {
                    let start = pos + 8 + i * 16;
                    let addr_bytes: [u8; 16] = data[start..start + 16].try_into()?;
                    ra.dns_servers.push(Ipv6Addr::from(addr_bytes));
                }
            }
            _ => {}
        }

        pos += opt_len;
    }

    Ok(ra)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_router_solicitation() {
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let pkt = build_router_solicitation(&mac);

        assert_eq!(pkt[0], ICMPV6_ROUTER_SOLICITATION);
        assert_eq!(pkt[1], 0); // code
        assert_eq!(pkt[8], ND_OPT_SOURCE_LL_ADDR);
        assert_eq!(pkt[9], 1); // length
        assert_eq!(&pkt[10..16], &mac);
    }

    #[test]
    fn test_parse_router_advertisement_minimal() {
        let mut data = vec![0u8; 16];
        data[0] = ICMPV6_ROUTER_ADVERTISEMENT;
        data[4] = 64; // hop limit
        data[5] = 0xC0; // M=1, O=1
        data[6] = 0x07;
        data[7] = 0x08; // router lifetime = 1800

        let source = "fe80::1".parse().unwrap();
        let ra = parse_router_advertisement(&data, source).unwrap();

        assert_eq!(ra.hop_limit, 64);
        assert!(ra.managed_flag);
        assert!(ra.other_flag);
        assert_eq!(ra.router_lifetime, 1800);
    }
}
