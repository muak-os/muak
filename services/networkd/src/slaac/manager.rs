//! Manages the IPv6 SLAAC state machine for a single network interface.

use std::net::Ipv6Addr;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use netlib::slaac::address::generate;
use netlib::slaac::icmpv6::{
    ICMPV6_ROUTER_ADVERTISEMENT, RouterAdvertisement, build_router_solicitation,
    parse_router_advertisement,
};
use netlib::socket;
use rustix::net::ipproto::ICMPV6;
use rustix::net::netdevice::name_to_index;
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketAddrV6, SocketFlags, SocketType, recvfrom, sendto,
    socket_with,
};
use tokio::io::unix::AsyncFd;
use tokio::time::Instant;

use super::state::{AddressState, ManagedAddress, ManagedDns, ManagedRouter};
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

pub struct SlaacManager {
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

#[allow(clippy::excessive_nesting)]
impl SlaacManager {
    pub async fn new(
        interface: String,
        mac: [u8; 6],
        config: Arc<config::NetworkConfig>,
    ) -> Result<Self> {
        let iface_clone = interface.clone();
        let (socket_fd, ifindex) = tokio::task::spawn_blocking(move || -> Result<_> {
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
        if !self.solicited {
            self.solicited = true;
            return match self.initial_solicitation().await {
                Ok(event) => event,
                Err(e) => SlaacEvent::Failed {
                    reason: e.to_string(),
                },
            };
        }
        self.next_monitoring_event().await
    }

    async fn next_monitoring_event(&mut self) -> SlaacEvent {
        loop {
            let next_deadline = self.next_timer_deadline();

            let event = tokio::select! {
                _ = tokio::time::sleep_until(next_deadline) => {
                    self.handle_timer_expiration().await
                }
                result = self.try_recv_ra() => {
                    match result {
                        Ok(Some(ra)) => self.process_unsolicited_ra(ra).await,
                        _ => None,
                    }
                }
            };

            if let Some(e) = event {
                return e;
            }

            if self.address.is_none() && self.router.is_none() {
                return SlaacEvent::Failed {
                    reason: "all IPv6 configuration expired".to_string(),
                };
            }
        }
    }

    async fn initial_solicitation(&mut self) -> Result<SlaacEvent> {
        let rs_packet = build_router_solicitation(&self.mac);
        let dest = create_all_routers_sockaddr(self.ifindex);

        for attempt in 1..=MAX_RTR_SOLICITATIONS {
            kmsg::info!(
                "SLAAC: sending RS on {} (attempt {}/{})",
                self.interface,
                attempt,
                MAX_RTR_SOLICITATIONS
            );

            sendto(self.socket.get_ref(), &rs_packet, SendFlags::empty(), &dest)?;

            match self.wait_for_ra(RTR_SOLICITATION_INTERVAL).await {
                Ok(ra) => return self.process_initial_ra(ra).await,
                Err(_) => {
                    if attempt < MAX_RTR_SOLICITATIONS {
                        kmsg::info!("SLAAC: no RA on {}, retrying...", self.interface);
                    }
                }
            }
        }

        bail!(
            "no Router Advertisement received after {} attempts",
            MAX_RTR_SOLICITATIONS
        )
    }

    async fn process_initial_ra(&mut self, ra: RouterAdvertisement) -> Result<SlaacEvent> {
        let prefix = ra
            .prefixes
            .iter()
            .find(|p| p.autonomous && p.prefix_len <= 64)
            .ok_or_else(|| anyhow::anyhow!("no usable autonomous prefix in RA"))?;

        let address = generate(prefix.prefix, prefix.prefix_len, &self.mac)
            .ok_or_else(|| anyhow::anyhow!("invalid prefix_len {} in RA", prefix.prefix_len))?;

        let managed_addr = ManagedAddress::new(
            address,
            prefix.prefix_len,
            ra.source,
            prefix.valid_lifetime,
            prefix.preferred_lifetime,
        );

        let managed_router = ManagedRouter::new(ra.source, ra.router_lifetime as u64);

        let dns_servers: Vec<ManagedDns> = if ra.dns_servers.is_empty() {
            kmsg::info!(
                "SLAAC: no RDNSS in RA on {}, using fallback DNS",
                self.interface
            );
            self.config
                .ipv6_dns()
                .map(|s| ManagedDns::new(s, u64::MAX))
                .collect()
        } else {
            ra.dns_servers
                .iter()
                .map(|&s| ManagedDns::new(s, ra.dns_lifetime as u64))
                .collect()
        };

        let dns_addrs: Vec<Ipv6Addr> = dns_servers.iter().map(|d| d.value).collect();

        kmsg::info!(
            "SLAAC: acquired {} via {}, {} DNS servers on {}",
            address,
            ra.source,
            dns_addrs.len(),
            self.interface
        );

        self.address = Some(managed_addr);
        self.router = Some(managed_router);
        self.dns_servers = dns_servers;

        Ok(SlaacEvent::Configured {
            address,
            prefix_len: prefix.prefix_len,
            gateway: ra.source,
            dns: dns_addrs,
        })
    }

    /// Checks all timers and emits at most one event.
    async fn handle_timer_expiration(&mut self) -> Option<SlaacEvent> {
        let now = Instant::now();

        if let Some(addr) = &mut self.address
            && addr.state == AddressState::Preferred
            && now >= addr.preferred_until
        {
            addr.state = AddressState::Deprecated;
            kmsg::info!(
                "SLAAC: address {} deprecated on {}",
                addr.address,
                self.interface
            );
            return Some(SlaacEvent::AddressDeprecated {
                address: addr.address,
            });
        }

        if let Some(addr) = &self.address
            && now >= addr.valid_until
        {
            kmsg::info!(
                "SLAAC: address {} expired on {}",
                addr.address,
                self.interface
            );
            let address = addr.address;
            self.address = None;
            return Some(SlaacEvent::AddressExpired { address });
        }

        if let Some(router) = &self.router
            && now >= router.expires_at
        {
            kmsg::info!(
                "SLAAC: router {} expired on {}",
                router.value,
                self.interface
            );
            let router_addr = router.value;
            self.router = None;
            return Some(SlaacEvent::RouterExpired {
                router: router_addr,
            });
        }

        let had_dns = !self.dns_servers.is_empty();
        let before_len = self.dns_servers.len();
        self.dns_servers.retain(|dns| now < dns.expires_at);
        let dns_changed = self.dns_servers.len() != before_len;

        if had_dns && self.dns_servers.is_empty() {
            kmsg::info!(
                "SLAAC: all RDNSS expired on {}, using fallback",
                self.interface
            );
            self.dns_servers = self
                .config
                .ipv6_dns()
                .map(|s| ManagedDns::new(s, u64::MAX))
                .collect();
        }

        if dns_changed {
            let servers: Vec<Ipv6Addr> = self.dns_servers.iter().map(|d| d.value).collect();
            return Some(SlaacEvent::DnsUpdated { servers });
        }

        None
    }

    async fn process_unsolicited_ra(&mut self, ra: RouterAdvertisement) -> Option<SlaacEvent> {
        kmsg::info!(
            "SLAAC: unsolicited RA from {} on {}",
            ra.source,
            self.interface
        );

        if ra.router_lifetime == 0 {
            if let Some(router) = &self.router
                && router.value == ra.source
            {
                kmsg::info!(
                    "SLAAC: router {} signaled departure (lifetime=0) on {}",
                    ra.source,
                    self.interface
                );
                self.router = None;
                return Some(SlaacEvent::RouterExpired { router: ra.source });
            }
            return None;
        }

        if let Some(router) = &mut self.router
            && router.value == ra.source
        {
            router.refresh_lifetime(ra.router_lifetime as u64);
        }

        if let Some(addr) = &mut self.address {
            for prefix in &ra.prefixes {
                if prefix.autonomous
                    && prefix.prefix_len <= 64
                    && addr.router == ra.source
                    && let Some(expected_addr) =
                        generate(prefix.prefix, prefix.prefix_len, &self.mac)
                    && expected_addr == addr.address
                {
                    addr.refresh_lifetimes(prefix.valid_lifetime, prefix.preferred_lifetime);
                    kmsg::info!(
                        "SLAAC: refreshed lifetimes for {} on {}",
                        addr.address,
                        self.interface
                    );
                    break;
                }
            }
        }

        if !ra.dns_servers.is_empty() {
            for ra_dns in &ra.dns_servers {
                if let Some(managed) = self.dns_servers.iter_mut().find(|d| d.value == *ra_dns) {
                    managed.refresh_lifetime(ra.dns_lifetime as u64);
                } else {
                    self.dns_servers
                        .push(ManagedDns::new(*ra_dns, ra.dns_lifetime as u64));
                }
            }

            let servers: Vec<Ipv6Addr> = self.dns_servers.iter().map(|d| d.value).collect();
            return Some(SlaacEvent::DnsUpdated { servers });
        }

        None
    }

    fn next_timer_deadline(&self) -> Instant {
        let mut deadline = Instant::now() + Duration::from_secs(3600);

        if let Some(addr) = &self.address {
            if addr.state == AddressState::Preferred && addr.preferred_until < deadline {
                deadline = addr.preferred_until;
            }
            if addr.valid_until < deadline {
                deadline = addr.valid_until;
            }
        }

        if let Some(router) = &self.router
            && router.expires_at < deadline
        {
            deadline = router.expires_at;
        }

        for dns in &self.dns_servers {
            if dns.expires_at < deadline {
                deadline = dns.expires_at;
            }
        }

        deadline
    }

    async fn wait_for_ra(&self, timeout: Duration) -> Result<RouterAdvertisement> {
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timeout waiting for RA");
            }

            match tokio::time::timeout(remaining, self.try_recv_ra()).await {
                Ok(Ok(Some(ra))) => return Ok(ra),
                Ok(Ok(None)) => continue,
                Ok(Err(e)) => return Err(e),
                Err(_) => bail!("timeout waiting for RA"),
            }
        }
    }

    async fn try_recv_ra(&self) -> Result<Option<RouterAdvertisement>> {
        let mut guard = self.socket.readable().await?;

        let mut buf = [0u8; 1500];

        let recv_result = guard.try_io(|inner| {
            recvfrom(inner.get_ref(), &mut buf, RecvFlags::empty()).map_err(|e| match e {
                rustix::io::Errno::WOULDBLOCK => {
                    std::io::Error::from(std::io::ErrorKind::WouldBlock)
                }
                _ => std::io::Error::from_raw_os_error(e.raw_os_error()),
            })
        });

        match recv_result {
            Ok(Ok((_, len, Some(addr)))) => {
                if addr.address_family() == AddressFamily::INET6
                    && buf[0] == ICMPV6_ROUTER_ADVERTISEMENT
                {
                    match SocketAddrV6::try_from(addr) {
                        Ok(sockaddr_v6) => {
                            let source = *sockaddr_v6.ip();
                            let ra = parse_router_advertisement(&buf[..len], source)?;
                            return Ok(Some(ra));
                        }
                        Err(_) => {
                            return Ok(None);
                        }
                    }
                }
                Ok(None)
            }
            Ok(Ok((_, len, None))) => {
                if buf[0] == ICMPV6_ROUTER_ADVERTISEMENT {
                    let ra = parse_router_advertisement(&buf[..len], Ipv6Addr::UNSPECIFIED)?;
                    return Ok(Some(ra));
                }
                Ok(None)
            }
            Ok(Err(e)) => {
                bail!("recvfrom failed: {}", e)
            }
            Err(_would_block) => {
                guard.clear_ready();
                Ok(None)
            }
        }
    }
}

fn create_all_routers_sockaddr(ifindex: u32) -> SocketAddrV6 {
    let all_routers = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 2);
    let std_sockaddr = std::net::SocketAddrV6::new(all_routers, 0, 0, ifindex);
    SocketAddrV6::new(
        *std_sockaddr.ip(),
        std_sockaddr.port(),
        std_sockaddr.flowinfo(),
        std_sockaddr.scope_id(),
    )
}
