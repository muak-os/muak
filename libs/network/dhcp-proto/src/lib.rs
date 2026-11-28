//! No std, sans-io, allocation-free RFC 2131/2132 `DHCPv4` message codec.

#![no_std]
#![warn(missing_docs)]

pub mod builder;
pub mod error;
pub(crate) mod field;
pub mod header;
pub mod message_type;
pub(crate) mod option;
pub mod options;
pub mod validate;
pub mod xid;

use core::net::Ipv4Addr;

/// Magic cookie that starts the DHCP options section.
pub const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
/// `BOOTREQUEST` value of the DHCP `op` field.
pub const BOOTREQUEST: u8 = 1;
/// Broadcast flag of the DHCP `flags` field.
pub const FLAG_BROADCAST: u16 = 0x8000;

/// UDP port a DHCP client listens on.
pub const DHCP_CLIENT_PORT: u16 = 68;
/// UDP port a DHCP server listens on.
pub const DHCP_SERVER_PORT: u16 = 67;

/// Parsed DHCP options extracted from a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedOptions<'a> {
    /// Message type option.
    pub message_type: Option<u8>,
    /// Server identifier option.
    pub server_id: Option<Ipv4Addr>,
    /// Subnet mask option.
    pub subnet_mask: Option<Ipv4Addr>,
    /// Router option.
    pub router: Option<Ipv4Addr>,
    /// DNS server options.
    pub dns_servers: DnsServers<'a>,
    /// Lease time option, in seconds.
    pub lease_time: Option<u32>,
}

/// Iterator over the DNS server addresses declared by a DHCP options section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsServers<'a> {
    options: &'a [u8],
    pending: &'a [u8],
}

impl<'a> DnsServers<'a> {
    /// Wraps the options section that follows the magic cookie.
    #[must_use = "the wrapped iterator is the only result"]
    pub(crate) const fn new(options: &'a [u8]) -> Self {
        Self {
            options,
            pending: &[],
        }
    }

    fn take_pending(&mut self) -> Option<Ipv4Addr> {
        let (chunk, rest) = self.pending.split_first_chunk::<4>()?;
        self.pending = rest;

        Some(Ipv4Addr::from(*chunk))
    }

    fn split_next(&mut self) -> Option<(u8, &'a [u8])> {
        let mut rest = self.options;
        let code = core::iter::from_fn(|| take_code(&mut rest)).find(|code| *code != 0)?;
        let length = *rest.first()?;
        let data_end = 1_usize.saturating_add(usize::from(length));
        let data = rest.get(1..data_end)?;
        self.options = rest.get(data_end..)?;

        Some((code, data))
    }
}

impl Iterator for DnsServers<'_> {
    type Item = Ipv4Addr;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(address) = self.take_pending() {
            return Some(address);
        }
        let data = core::iter::from_fn(|| self.split_next())
            .find_map(|(code, data)| is_dns_option(code, data).then_some(data))?;
        self.pending = data;

        self.take_pending()
    }
}

impl core::iter::FusedIterator for DnsServers<'_> {}

fn take_code(rest: &mut &[u8]) -> Option<u8> {
    let code = *rest.first()?;
    if code == option::END {
        return None;
    }
    *rest = rest.get(1..)?;

    Some(code)
}

const fn is_dns_option(code: u8, data: &[u8]) -> bool {
    code == option::DNS_SERVER && data.len() >= 4 && data.len().is_multiple_of(4)
}
