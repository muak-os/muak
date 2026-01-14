use std::ffi::OsString;
use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;

use anyhow::{Result, bail};
use nix::sys::socket::{
    AddressFamily, MsgFlags, SockFlag, SockProtocol, SockType, SockaddrIn6, recvfrom, sendto,
    setsockopt, socket, sockopt,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::address::generate_slaac_address;
use super::icmpv6::{
    ICMPV6_ROUTER_ADVERTISEMENT, build_router_solicitation, parse_router_advertisement,
};
use super::state::{AddressState, ManagedAddress, ManagedDns, ManagedRouter};
use super::{FALLBACK_DNS, create_icmpv6_filter, get_interface_index, set_icmpv6_filter};

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
    socket: OwnedFd,

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
        let ifindex = get_interface_index(&interface)?;

        let socket = socket(
            AddressFamily::Inet6,
            SockType::Raw,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            Some(SockProtocol::IcmpV6),
        )?;

        setsockopt(&socket, sockopt::BindToDevice, &OsString::from(&interface))?;

        let filter = create_icmpv6_filter();
        set_icmpv6_filter(socket.as_raw_fd(), &filter)?;

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

            sendto(
                self.socket.as_raw_fd(),
                &rs_packet,
                &dest,
                MsgFlags::empty(),
            )?;

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

    async fn process_initial_ra(&mut self, ra: super::icmpv6::RouterAdvertisement) -> Result<()> {
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

        if let Some(router) = &self.router {
            if router.expires_at < deadline {
                deadline = router.expires_at;
            }
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

        if let Some(addr) = &mut self.address {
            if addr.state == AddressState::Preferred && now >= addr.preferred_until {
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
        }

        if let Some(addr) = &self.address {
            if now >= addr.valid_until {
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
        }

        if let Some(router) = &self.router {
            if now >= router.expires_at {
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

    async fn process_unsolicited_ra(&mut self, ra: super::icmpv6::RouterAdvertisement) {
        kmsg::info!(@ "networkd", "SLAAC manager: received unsolicited RA from {}", ra.source);

        if ra.router_lifetime == 0 {
            if let Some(router) = &self.router {
                if router.address == ra.source {
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

                    if let Some(addr) = &self.address {
                        if addr.router == ra.source {
                            kmsg::info!(
                                @ "networkd",
                                "SLAAC manager: router for {} went away, address will expire",
                                addr.address
                            );
                        }
                    }
                }
            }
            return;
        }

        if let Some(router) = &mut self.router {
            if router.address == ra.source {
                router.refresh_lifetime(ra.router_lifetime);
            }
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

    async fn wait_for_ra(&self, timeout: Duration) -> Result<super::icmpv6::RouterAdvertisement> {
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

    async fn try_recv_ra(&self) -> Result<Option<super::icmpv6::RouterAdvertisement>> {
        let fd_raw = self.socket.as_raw_fd();

        let ready =
            tokio::task::spawn_blocking(move || poll_read(fd_raw, Duration::from_millis(100)))
                .await??;

        if !ready {
            return Ok(None);
        }

        let mut buf = [0u8; 1500];
        match recvfrom::<SockaddrIn6>(self.socket.as_raw_fd(), &mut buf) {
            Ok((len, Some(addr))) => {
                if buf[0] == ICMPV6_ROUTER_ADVERTISEMENT {
                    let source = addr.ip();
                    let ra = parse_router_advertisement(&buf[..len], source)?;
                    return Ok(Some(ra));
                }
                Ok(None)
            }
            Ok((len, None)) => {
                if buf[0] == ICMPV6_ROUTER_ADVERTISEMENT {
                    let ra = parse_router_advertisement(&buf[..len], Ipv6Addr::UNSPECIFIED)?;
                    return Ok(Some(ra));
                }
                Ok(None)
            }
            Err(nix::errno::Errno::EAGAIN) => Ok(None),
            Err(e) => bail!("recvfrom failed: {}", e),
        }
    }
}

fn create_all_routers_sockaddr(ifindex: u32) -> SockaddrIn6 {
    let all_routers: Ipv6Addr = "ff02::2".parse().unwrap();
    SockaddrIn6::from(std::net::SocketAddrV6::new(all_routers, 0, 0, ifindex))
}

fn poll_read(fd: i32, timeout: Duration) -> Result<bool> {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::os::fd::BorrowedFd;

    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let mut fds = [PollFd::new(borrowed_fd, PollFlags::POLLIN)];
    let timeout_ms = timeout.as_millis() as i32;

    let n = poll(&mut fds, PollTimeout::try_from(timeout_ms)?)?;
    Ok(n > 0)
}
