use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;

use anyhow::{Result, bail};
use rustix::net::ipproto::ICMPV6;
use rustix::net::netdevice::name_to_index;
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketAddrV6, SocketFlags, SocketType, recvfrom, sendto,
    socket_with,
};
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::socket;

use super::address::generate_slaac_address;
use super::icmpv6::{
    ICMPV6_ROUTER_ADVERTISEMENT, RouterAdvertisement, build_router_solicitation,
    parse_router_advertisement,
};
use super::state::{AddressState, ManagedAddress, ManagedDns, ManagedRouter};
use super::{FALLBACK_DNS, create_icmpv6_filter, set_icmpv6_filter};

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

    address: Option<ManagedAddress>,
    router: Option<ManagedRouter>,
    dns_servers: Vec<ManagedDns>,

    event_tx: mpsc::Sender<SlaacEvent>,
}

impl SlaacManager {
    pub fn new(
        interface: String,
        mac: [u8; 6],
        event_tx: mpsc::Sender<SlaacEvent>,
    ) -> Result<Self> {
        let socket_fd = socket_with(
            AddressFamily::INET6,
            SocketType::RAW,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            Some(ICMPV6),
        )?;

        socket::socket_bind_device(&socket_fd, &interface)?;

        let ifindex = name_to_index(&socket_fd, &interface)?;

        let filter = create_icmpv6_filter();
        set_icmpv6_filter(socket_fd.as_raw_fd(), &filter)?;

        // Wrap in AsyncFd for tokio integration - completely safe!
        let socket = AsyncFd::new(socket_fd)?;

        Ok(Self {
            interface,
            mac,
            ifindex,
            socket,
            address: None,
            router: None,
            dns_servers: Vec::new(),
            event_tx,
        })
    }

    pub async fn run(mut self) {
        match self.initial_solicitation().await {
            Ok(()) => {
                kmsg::info!(@ "networkd", "SLAAC manager: initial configuration complete");
            }
            Err(e) => {
                let _ = self
                    .event_tx
                    .send(SlaacEvent::Failed {
                        reason: e.to_string(),
                    })
                    .await;
                return;
            }
        }

        self.monitoring_loop().await;
    }

    async fn initial_solicitation(&mut self) -> Result<()> {
        let rs_packet = build_router_solicitation(&self.mac);
        let dest = create_all_routers_sockaddr(self.ifindex);

        for attempt in 1..=MAX_RTR_SOLICITATIONS {
            kmsg::info!(
                @ "networkd",
                "SLAAC manager: sending RS on {} (attempt {}/{})",
                self.interface,
                attempt,
                MAX_RTR_SOLICITATIONS
            );

            sendto(self.socket.get_ref(), &rs_packet, SendFlags::empty(), &dest)?;

            match self.wait_for_ra(RTR_SOLICITATION_INTERVAL).await {
                Ok(ra) => {
                    self.process_initial_ra(ra).await?;
                    return Ok(());
                }
                Err(_) => {
                    if attempt < MAX_RTR_SOLICITATIONS {
                        kmsg::info!(@ "networkd", "SLAAC manager: no RA received, retrying...");
                    }
                }
            }
        }

        bail!(
            "no Router Advertisement received after {} attempts",
            MAX_RTR_SOLICITATIONS
        )
    }

    async fn process_initial_ra(&mut self, ra: RouterAdvertisement) -> Result<()> {
        let prefix = ra
            .prefixes
            .iter()
            .find(|p| p.autonomous && p.prefix_len <= 64)
            .ok_or_else(|| anyhow::anyhow!("no usable autonomous prefix in RA"))?;

        let address = generate_slaac_address(prefix.prefix, prefix.prefix_len, &self.mac);

        let managed_addr = ManagedAddress::new(
            address,
            prefix.prefix_len,
            ra.source,
            prefix.valid_lifetime,
            prefix.preferred_lifetime,
        );

        let managed_router = ManagedRouter::new(ra.source, ra.router_lifetime);

        let dns_servers: Vec<ManagedDns> = if ra.dns_servers.is_empty() {
            kmsg::info!(@ "networkd", "SLAAC manager: no RDNSS in RA, using fallback DNS");
            FALLBACK_DNS
                .iter()
                .map(|&s| ManagedDns::new(s, u32::MAX)) // Fallback never expires
                .collect()
        } else {
            ra.dns_servers
                .iter()
                .map(|&s| ManagedDns::new(s, ra.dns_lifetime))
                .collect()
        };

        let dns_addrs: Vec<Ipv6Addr> = dns_servers.iter().map(|d| d.server).collect();

        kmsg::info!(
            @ "networkd",
            "SLAAC manager: acquired {} via {}, {} DNS servers",
            address,
            ra.source,
            dns_addrs.len()
        );

        self.address = Some(managed_addr);
        self.router = Some(managed_router);
        self.dns_servers = dns_servers;

        let _ = self
            .event_tx
            .send(SlaacEvent::Configured {
                address,
                prefix_len: prefix.prefix_len,
                gateway: ra.source,
                dns: dns_addrs,
            })
            .await;

        Ok(())
    }

    async fn monitoring_loop(&mut self) {
        loop {
            let next_deadline = self.next_timer_deadline();

            tokio::select! {
                _ = tokio::time::sleep_until(next_deadline) => {
                    self.handle_timer_expiration().await;
                }

                result = self.try_recv_ra() => {
                    if let Ok(Some(ra)) = result {
                        self.process_unsolicited_ra(ra).await;
                    }
                }
            }

            if self.address.is_none() && self.router.is_none() {
                kmsg::info!(@ "networkd", "SLAAC manager: all IPv6 configuration expired, shutting down");
                break;
            }
        }
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

    async fn handle_timer_expiration(&mut self) {
        let now = Instant::now();

        if let Some(addr) = &mut self.address
            && addr.state == AddressState::Preferred
            && now >= addr.preferred_until
        {
            addr.state = AddressState::Deprecated;
            kmsg::info!(
                @ "networkd",
                "SLAAC manager: address {} deprecated",
                addr.address
            );
            let _ = self
                .event_tx
                .send(SlaacEvent::AddressDeprecated {
                    address: addr.address,
                })
                .await;
        }

        if let Some(addr) = &self.address
            && now >= addr.valid_until
        {
            kmsg::info!(
                @ "networkd",
                "SLAAC manager: address {} expired",
                addr.address
            );
            let _ = self
                .event_tx
                .send(SlaacEvent::AddressExpired {
                    address: addr.address,
                })
                .await;
            self.address = None;
        }

        if let Some(router) = &self.router
            && now >= router.expires_at
        {
            kmsg::info!(
                @ "networkd",
                "SLAAC manager: router {} expired",
                router.address
            );
            let _ = self
                .event_tx
                .send(SlaacEvent::RouterExpired {
                    router: router.address,
                })
                .await;
            self.router = None;
        }

        let had_dns = !self.dns_servers.is_empty();
        self.dns_servers.retain(|dns| now < dns.expires_at);

        if had_dns && self.dns_servers.is_empty() {
            kmsg::info!(@ "networkd", "SLAAC manager: all RDNSS expired, using fallback");
            self.dns_servers = FALLBACK_DNS
                .iter()
                .map(|&s| ManagedDns::new(s, u32::MAX))
                .collect();
        }

        if had_dns && !self.dns_servers.is_empty() {
            let servers: Vec<Ipv6Addr> = self.dns_servers.iter().map(|d| d.server).collect();
            let _ = self.event_tx.send(SlaacEvent::DnsUpdated { servers }).await;
        }
    }

    async fn process_unsolicited_ra(&mut self, ra: RouterAdvertisement) {
        kmsg::info!(@ "networkd", "SLAAC manager: received unsolicited RA from {}", ra.source);

        if ra.router_lifetime == 0 {
            if let Some(router) = &self.router
                && router.address == ra.source
            {
                kmsg::info!(
                    @ "networkd",
                    "SLAAC manager: router {} signaled departure (lifetime=0)",
                    ra.source
                );
                let _ = self
                    .event_tx
                    .send(SlaacEvent::RouterExpired { router: ra.source })
                    .await;
                self.router = None;

                if let Some(addr) = &self.address
                    && addr.router == ra.source
                {
                    kmsg::info!(
                        @ "networkd",
                        "SLAAC manager: router for {} went away, address will expire",
                        addr.address
                    );
                }
            }
            return;
        }

        if let Some(router) = &mut self.router
            && router.address == ra.source
        {
            router.refresh_lifetime(ra.router_lifetime);
        }

        if let Some(addr) = &mut self.address {
            for prefix in &ra.prefixes {
                if prefix.autonomous && addr.router == ra.source {
                    let expected_addr =
                        generate_slaac_address(prefix.prefix, prefix.prefix_len, &self.mac);
                    if expected_addr == addr.address {
                        addr.refresh_lifetimes(prefix.valid_lifetime, prefix.preferred_lifetime);
                        kmsg::info!(
                            @ "networkd",
                            "SLAAC manager: refreshed lifetimes for {}",
                            addr.address
                        );
                        break;
                    }
                }
            }
        }

        if !ra.dns_servers.is_empty() {
            for ra_dns in &ra.dns_servers {
                if let Some(managed) = self.dns_servers.iter_mut().find(|d| d.server == *ra_dns) {
                    managed.refresh_lifetime(ra.dns_lifetime);
                } else {
                    self.dns_servers
                        .push(ManagedDns::new(*ra_dns, ra.dns_lifetime));
                }
            }

            let servers: Vec<Ipv6Addr> = self.dns_servers.iter().map(|d| d.server).collect();
            let _ = self.event_tx.send(SlaacEvent::DnsUpdated { servers }).await;
        }
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
    let all_routers: Ipv6Addr = "ff02::2".parse().expect("valid IPv6 address");
    let std_sockaddr = std::net::SocketAddrV6::new(all_routers, 0, 0, ifindex);
    SocketAddrV6::new(
        *std_sockaddr.ip(),
        std_sockaddr.port(),
        std_sockaddr.flowinfo(),
        std_sockaddr.scope_id(),
    )
}
