//! Manages the IPv6 SLAAC state machine for a single network interface.

extern crate alloc;

use alloc::sync::Arc;
use core::net::Ipv6Addr;
use core::time::Duration;
use std::os::fd::OwnedFd;

use anyhow::{Result, bail};
use netlib::slaac::address::generate;
use netlib::slaac::icmpv6::{
    ICMPV6_ROUTER_ADVERTISEMENT, RouterAdvertisement, build_router_solicitation,
    parse_router_advertisement,
};
use netlib::socket;
use rustix::io::Errno;
use rustix::net::ipproto::ICMPV6;
use rustix::net::netdevice::name_to_index;
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketAddrV6, SocketFlags, SocketType, recvfrom, sendto,
    socket_with,
};
use tokio::io::unix::AsyncFd;
use tokio::task::spawn_blocking;
use tokio::time::{Instant, timeout};

use super::state::{AddressState, Expiring, ManagedAddress, ManagedDns, ManagedRouter};
use super::{create_icmpv6_filter, set_icmpv6_filter};

const RTR_SOLICITATION_INTERVAL: Duration = Duration::from_secs(4);
const MAX_RTR_SOLICITATIONS: u32 = 3;

#[derive(Debug, Clone)]
pub enum SlaacEvent {
    Configured {
        address: Ipv6Addr,
        prefix_len: u8,
        gateway: Ipv6Addr,
        dns: Vec<Ipv6Addr>,
    },
    AddressDeprecated {
        address: Ipv6Addr,
    },
    AddressExpired {
        address: Ipv6Addr,
    },
    RouterExpired {
        router: Ipv6Addr,
    },
    DnsUpdated {
        servers: Vec<Ipv6Addr>,
    },
    Failed {
        reason: String,
    },
}

pub struct Manager {
    interface: String,
    mac: [u8; 6],
    ifindex: u32,
    socket: AsyncFd<OwnedFd>,
    solicited: bool,
    config: Arc<config::NetworkConfig>,

    address: Option<ManagedAddress>,
    router: Option<ManagedRouter>,
    dns_servers: Vec<ManagedDns>,
}

impl Manager {
    /// Opens a raw IPv6 ICMP socket bound to the given interface.
    ///
    /// # Errors
    ///
    /// Returns an error if the raw socket cannot be opened, the device cannot be
    /// bound, or the `ICMPv6` filter cannot be applied.
    pub async fn new(
        interface: String,
        mac: [u8; 6],
        config: Arc<config::NetworkConfig>,
    ) -> Result<Self> {
        let iface_clone = interface.clone();
        let (socket_fd, ifindex) = spawn_blocking(move || -> Result<_> {
            let fd = socket_with(
                AddressFamily::INET6,
                SocketType::RAW,
                SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
                Some(ICMPV6),
            )?;
            socket::bind_device(&fd, &iface_clone)?;
            let idx = name_to_index(&fd, &iface_clone)?;
            let filter = create_icmpv6_filter();
            set_icmpv6_filter(&fd, &filter)?;
            Ok((fd, idx))
        })
        .await??;

        let socket = AsyncFd::new(socket_fd)?;

        Ok(Self {
            interface,
            mac,
            ifindex,
            socket,
            solicited: false,
            config,
            address: None,
            router: None,
            dns_servers: Vec::new(),
        })
    }

    /// Performs the initial RS/RA solicitation on first call, then returns monitoring events.
    pub async fn next_event(&mut self) -> SlaacEvent {
        if self.solicited {
            return next_monitoring_event(self).await;
        }
        self.solicited = true;
        match initial_solicitation(self).await {
            Ok(event) => event,
            Err(e) => SlaacEvent::Failed {
                reason: e.to_string(),
            },
        }
    }
}

async fn next_monitoring_event(manager: &mut Manager) -> SlaacEvent {
    loop {
        let next_deadline = next_timer_deadline(manager);
        let remaining = next_deadline.saturating_duration_since(Instant::now());

        let event = match timeout(remaining, try_recv_ra(manager)).await {
            Ok(Ok(Some(ra))) => process_unsolicited_ra(manager, &ra),
            Ok(Ok(None) | Err(_)) => None,
            Err(_) => handle_timer_expiration(manager),
        };

        if let Some(e) = event {
            return e;
        }

        if manager.address.is_none() && manager.router.is_none() {
            return SlaacEvent::Failed {
                reason: "all IPv6 configuration expired".to_owned(),
            };
        }
    }
}

async fn initial_solicitation(manager: &mut Manager) -> Result<SlaacEvent> {
    let rs_packet = build_router_solicitation(&manager.mac);
    let dest = create_all_routers_sockaddr(manager.ifindex);

    for attempt in 1..=MAX_RTR_SOLICITATIONS {
        kmsg::info!(
            "SLAAC: sending RS on {} (attempt {attempt}/{MAX_RTR_SOLICITATIONS})",
            manager.interface
        );

        sendto(
            manager.socket.get_ref(),
            &rs_packet,
            SendFlags::empty(),
            &dest,
        )?;

        if let Ok(ra) = wait_for_ra(manager, RTR_SOLICITATION_INTERVAL).await {
            return process_initial_ra(manager, &ra);
        }
        if attempt < MAX_RTR_SOLICITATIONS {
            kmsg::info!("SLAAC: no RA on {}, retrying...", manager.interface);
        }
    }

    bail!("no Router Advertisement received after {MAX_RTR_SOLICITATIONS} attempts")
}

async fn wait_for_ra(manager: &Manager, wait_timeout: Duration) -> Result<RouterAdvertisement> {
    let deadline = Instant::now()
        .checked_add(wait_timeout)
        .unwrap_or_else(Instant::now);

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timeout waiting for RA");
        }

        match timeout(remaining, try_recv_ra(manager)).await {
            Ok(Ok(Some(ra))) => return Ok(ra),
            Ok(Ok(None)) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => bail!("timeout waiting for RA"),
        }
    }
}

async fn try_recv_ra(manager: &Manager) -> Result<Option<RouterAdvertisement>> {
    let mut guard = manager.socket.readable().await?;

    let mut buf = [0_u8; 1500];

    let recv_result = guard.try_io(|inner| {
        recvfrom(inner.get_ref(), &mut buf, RecvFlags::empty()).map_err(|error| match error {
            Errno::WOULDBLOCK => std::io::Error::from(std::io::ErrorKind::WouldBlock),
            _ => std::io::Error::from_raw_os_error(error.raw_os_error()),
        })
    });

    let Ok(Ok((_, len, addr))) = recv_result else {
        if let Ok(Err(error)) = recv_result {
            bail!("recvfrom failed: {error}");
        }
        guard.clear_ready();
        return Ok(None);
    };

    let Some(addr) = addr else {
        if byte_at(&buf, 0) == ICMPV6_ROUTER_ADVERTISEMENT {
            let ra = parse_router_advertisement(
                buf.get(..len).unwrap_or_default(),
                Ipv6Addr::UNSPECIFIED,
            )?;
            return Ok(Some(ra));
        }
        return Ok(None);
    };

    if addr.address_family() != AddressFamily::INET6
        || byte_at(&buf, 0) != ICMPV6_ROUTER_ADVERTISEMENT
    {
        return Ok(None);
    }

    let Ok(sockaddr_v6) = SocketAddrV6::try_from(addr) else {
        return Ok(None);
    };
    let source = *sockaddr_v6.ip();
    let ra = parse_router_advertisement(buf.get(..len).unwrap_or_default(), source)?;

    Ok(Some(ra))
}

fn process_initial_ra(manager: &mut Manager, ra: &RouterAdvertisement) -> Result<SlaacEvent> {
    let prefix = ra
        .prefixes
        .iter()
        .find(|prefix| prefix.autonomous && prefix.prefix_len <= 64)
        .ok_or_else(|| anyhow::anyhow!("no usable autonomous prefix in RA"))?;

    let address = generate(prefix.prefix, prefix.prefix_len, &manager.mac)
        .ok_or_else(|| anyhow::anyhow!("invalid prefix_len {} in RA", prefix.prefix_len))?;

    let managed_addr = ManagedAddress::new(
        address,
        prefix.prefix_len,
        ra.source,
        prefix.valid_lifetime,
        prefix.preferred_lifetime,
    );

    let managed_router = ManagedRouter::new(ra.source, u64::from(ra.router_lifetime));

    let dns_servers: Vec<ManagedDns> = if ra.dns_servers.is_empty() {
        kmsg::info!(
            "SLAAC: no RDNSS in RA on {}, using fallback DNS",
            manager.interface
        );
        manager
            .config
            .ipv6_dns()
            .map(|server| ManagedDns::new(server, u64::MAX))
            .collect()
    } else {
        ra.dns_servers
            .iter()
            .map(|&server| ManagedDns::new(server, u64::from(ra.dns_lifetime)))
            .collect()
    };

    let dns_addrs: Vec<Ipv6Addr> = dns_servers.iter().map(|dns| dns.value).collect();

    kmsg::info!(
        "SLAAC: acquired {address} via {}, {} DNS servers on {}",
        ra.source,
        dns_addrs.len(),
        manager.interface
    );

    let prefix_len = managed_addr.prefix_len;
    manager.address = Some(managed_addr);
    manager.router = Some(managed_router);
    manager.dns_servers = dns_servers;

    Ok(SlaacEvent::Configured {
        address,
        prefix_len,
        gateway: ra.source,
        dns: dns_addrs,
    })
}

/// Checks all timers and emits at most one event.
fn process_unsolicited_ra(manager: &mut Manager, ra: &RouterAdvertisement) -> Option<SlaacEvent> {
    kmsg::info!(
        "SLAAC: unsolicited RA from {} on {}",
        ra.source,
        manager.interface
    );

    if ra.router_lifetime == 0 {
        if let Some(router) = manager.router.as_mut()
            && router.value == ra.source
        {
            kmsg::info!(
                "SLAAC: router {} signaled departure (lifetime=0) on {}",
                ra.source,
                manager.interface
            );
            manager.router = None;
            return Some(SlaacEvent::RouterExpired { router: ra.source });
        }
        return None;
    }

    if let Some(router) = manager.router.as_mut()
        && router.value == ra.source
    {
        router.refresh_lifetime(u64::from(ra.router_lifetime));
    }

    refresh_address_from_ra(manager, ra);

    if !ra.dns_servers.is_empty() {
        refresh_dns_from_ra(manager, ra);

        let servers: Vec<Ipv6Addr> = manager.dns_servers.iter().map(|dns| dns.value).collect();
        return Some(SlaacEvent::DnsUpdated { servers });
    }

    None
}

fn handle_timer_expiration(manager: &mut Manager) -> Option<SlaacEvent> {
    let now = Instant::now();

    if let Some(addr) = manager.address.as_mut()
        && addr.state == AddressState::Preferred
        && !addr.is_preferred()
    {
        addr.state = AddressState::Deprecated;
        kmsg::info!(
            "SLAAC: address {} deprecated on {}",
            addr.address,
            manager.interface
        );
        return Some(SlaacEvent::AddressDeprecated {
            address: addr.address,
        });
    }

    if let Some(addr) = manager.address.as_ref()
        && !addr.is_valid()
    {
        kmsg::info!(
            "SLAAC: address {} expired on {}",
            addr.address,
            manager.interface
        );
        let address = addr.address;
        manager.address = None;
        return Some(SlaacEvent::AddressExpired { address });
    }

    if let Some(router) = manager.router.as_ref()
        && now >= router.expires_at
    {
        kmsg::info!(
            "SLAAC: router {} expired on {}",
            router.value,
            manager.interface
        );
        let router_addr = router.value;
        manager.router = None;
        return Some(SlaacEvent::RouterExpired {
            router: router_addr,
        });
    }

    let had_dns = !manager.dns_servers.is_empty();
    let before_len = manager.dns_servers.len();
    manager.dns_servers.retain(Expiring::is_valid);
    let dns_changed = manager.dns_servers.len() != before_len;

    if had_dns && manager.dns_servers.is_empty() {
        kmsg::info!(
            "SLAAC: all RDNSS expired on {}, using fallback",
            manager.interface
        );
        manager.dns_servers = manager
            .config
            .ipv6_dns()
            .map(|server| ManagedDns::new(server, u64::MAX))
            .collect();
    }

    if dns_changed {
        let servers: Vec<Ipv6Addr> = manager.dns_servers.iter().map(|dns| dns.value).collect();
        return Some(SlaacEvent::DnsUpdated { servers });
    }

    None
}

/// Refreshes the tracked address from an RA matching the current router, returning whether it matched.
fn refresh_address_from_ra(manager: &mut Manager, ra: &RouterAdvertisement) -> bool {
    let Some(addr) = manager.address.as_mut() else {
        return false;
    };
    for prefix in &ra.prefixes {
        let generated = generate(prefix.prefix, prefix.prefix_len, &manager.mac);
        if prefix.autonomous
            && prefix.prefix_len <= 64
            && addr.router == ra.source
            && generated == Some(addr.address)
        {
            addr.refresh_lifetimes(prefix.valid_lifetime, prefix.preferred_lifetime);
            kmsg::info!(
                "SLAAC: refreshed lifetimes for {} on {}",
                addr.address,
                manager.interface
            );
            return true;
        }
    }

    false
}

/// Merges RDNSS servers from an RA into the tracked list, refreshing existing entries.
fn refresh_dns_from_ra(manager: &mut Manager, ra: &RouterAdvertisement) {
    for ra_dns in &ra.dns_servers {
        if let Some(managed) = manager
            .dns_servers
            .iter_mut()
            .find(|dns| dns.value == *ra_dns)
        {
            managed.refresh_lifetime(u64::from(ra.dns_lifetime));
        } else {
            manager
                .dns_servers
                .push(ManagedDns::new(*ra_dns, u64::from(ra.dns_lifetime)));
        }
    }
}

fn next_timer_deadline(manager: &Manager) -> Instant {
    let mut deadline = Instant::now()
        .checked_add(Duration::from_hours(1))
        .unwrap_or_else(Instant::now);

    if let Some(addr) = manager.address.as_ref() {
        if addr.state == AddressState::Preferred && addr.preferred_until < deadline {
            deadline = addr.preferred_until;
        }
        if addr.valid_until < deadline {
            deadline = addr.valid_until;
        }
    }

    if let Some(router) = manager.router.as_ref()
        && router.expires_at < deadline
    {
        deadline = router.expires_at;
    }

    for dns in &manager.dns_servers {
        if dns.expires_at < deadline {
            deadline = dns.expires_at;
        }
    }

    deadline
}

fn create_all_routers_sockaddr(ifindex: u32) -> SocketAddrV6 {
    let all_routers = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 2);
    let std_sockaddr = core::net::SocketAddrV6::new(all_routers, 0, 0, ifindex);
    SocketAddrV6::new(
        *std_sockaddr.ip(),
        std_sockaddr.port(),
        std_sockaddr.flowinfo(),
        std_sockaddr.scope_id(),
    )
}

fn byte_at(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or(0)
}
