//! RFC 4862 SLAAC and RFC 4861 router discovery finite state machine.

#![warn(missing_docs)]

extern crate alloc;

mod address;
mod method;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::net::Ipv6Addr;
use core::time::Duration;
use std::time::Instant;

use ip_method::LinkInfo;

use crate::method::Action;
use crate::method::state::{ManagedAddress, ManagedDns, ManagedRouter};

const SOLICITATION_LEN: usize = 8;
const MAX_RTR_SOLICITATIONS: u32 = 3;
const RTR_SOLICITATION_INTERVAL: Duration = Duration::from_secs(4);

/// Addresses, routers, and DNS servers derived from router advertisements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// Address and prefix length pairs the method configured.
    pub addresses: Vec<(Ipv6Addr, u8)>,
    /// Known router addresses.
    pub routers: Vec<Ipv6Addr>,
    /// Learned DNS servers.
    pub dns: Vec<Ipv6Addr>,
}

/// RFC 4862 stateless address autoconfiguration method.
pub struct Slaac {
    link: LinkInfo,
    fallback_dns: Vec<Ipv6Addr>,
    open: bool,
    running: bool,
    solicited: bool,
    solicitation_attempt: u32,
    solicitation_at: Option<Instant>,
    retry_at: Option<Instant>,
    address: Option<ManagedAddress>,
    router: Option<ManagedRouter>,
    dns_servers: Vec<ManagedDns>,
    queue: VecDeque<Action>,
    solicitation: [u8; SOLICITATION_LEN],
}
