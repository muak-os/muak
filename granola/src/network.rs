use dhcproto::v4::{Decodable, DhcpOption, DhcpOptions, Encodable, Message, Opcode};
use dhcproto::{Decoder, Encoder};
use futures::stream::TryStreamExt;
use netlink_packet_route::link::LinkAttribute;
use rtnetlink::{new_connection, Handle};
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_TIMEOUT: Duration = Duration::from_secs(10);

fn net_log(msg: &str) {
    crate::log(msg);
}

pub async fn setup_networking() -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
    net_log("Creating netlink connection");
    let (connection, handle, _) = new_connection()?;
    net_log("Spawning connection task");
    let connection_handle = tokio::spawn(connection);
    net_log("Connection task spawned");

    bring_up_loopback(&handle).await?;
    net_log("Loopback interface configured");

    let iface = wait_for_interface(&handle, "eth0", 10, Duration::from_millis(500)).await;
    let interface_name = if let Some(name) = iface {
        Some(name)
    } else {
        find_first_ethernet(&handle).await?
    };

    if let Some(interface_name) = interface_name {
        net_log(&format!("Using interface {}", interface_name));
        bring_up_interface(&handle, &interface_name).await?;

        let mac = get_interface_mac(&handle, &interface_name).await;
        if mac.is_none() {
            net_log("Warning: could not read MAC address; DHCP may fail");
        }

        match request_dhcp(&interface_name, mac).await? {
            Some(lease) => {
                configure_interface(&handle, &interface_name, &lease).await?;
                if !lease.dns.is_empty() {
                    write_resolv_conf(&lease.dns);
                }
            }
            None => net_log("DHCP failed or timed out; leaving interface up without address"),
        }
    } else {
        net_log("No ethernet interface found");
    }

    Ok(connection_handle)
}

async fn bring_up_loopback(handle: &Handle) -> Result<(), Box<dyn std::error::Error>> {
    let mut links = handle.link().get().match_name("lo".to_string()).execute();

    if let Some(link) = links.try_next().await? {
        let _ = handle.link().set(link.header.index).up().execute().await;

        let addr = Ipv4Addr::new(127, 0, 0, 1);
        let _ = handle
            .address()
            .add(link.header.index, addr.into(), 8)
            .execute()
            .await;
    }

    Ok(())
}

async fn wait_for_interface(
    handle: &Handle,
    name: &str,
    attempts: usize,
    delay: Duration,
) -> Option<String> {
    for _ in 0..attempts {
        if has_interface(handle, name).await.unwrap_or(false) {
            return Some(name.to_string());
        }
        net_log("Waiting for ethernet interface");
        tokio::time::sleep(delay).await;
    }
    None
}

async fn has_interface(handle: &Handle, name: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    Ok(links.try_next().await?.is_some())
}

async fn find_first_ethernet(
    handle: &Handle,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let mut links = handle.link().get().execute();

    while let Some(link) = links.try_next().await? {
        if let Some(name) = link.attributes.iter().find_map(|attr| match attr {
            LinkAttribute::IfName(n) => Some(n.clone()),
            _ => None,
        }) {
            if name.starts_with("eth") || name.starts_with("enp") || name.starts_with("ens") {
                return Ok(Some(name));
            }
        }
    }

    net_log("No ethernet interface found in detected interfaces");
    Ok(None)
}

async fn bring_up_interface(handle: &Handle, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    if let Some(link) = links.try_next().await? {
        handle.link().set(link.header.index).up().execute().await?;
    }

    Ok(())
}

async fn get_interface_mac(handle: &Handle, name: &str) -> Option<[u8; 6]> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    if let Ok(Some(link)) = links.try_next().await {
        for attr in link.attributes {
            if let LinkAttribute::Address(addr) = attr {
                if addr.len() == 6 {
                    let mut mac = [0u8; 6];
                    mac.copy_from_slice(&addr[..6]);
                    return Some(mac);
                }
            }
        }
    }
    None
}

struct DhcpLease {
    ip: Ipv4Addr,
    mask: Ipv4Addr,
    router: Option<Ipv4Addr>,
    dns: Vec<Ipv4Addr>,
}

async fn request_dhcp(
    interface: &str,
    mac: Option<[u8; 6]>,
) -> Result<Option<DhcpLease>, Box<dyn std::error::Error>> {
    tokio::time::sleep(Duration::from_millis(100)).await;

    let socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], DHCP_CLIENT_PORT))).await?;
    socket.set_broadcast(true)?;

    unsafe {
        let fd = socket.as_raw_fd();
        let iface_cstr = std::ffi::CString::new(interface)?;
        let ret = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            iface_cstr.as_ptr() as *const libc::c_void,
            iface_cstr.as_bytes_with_nul().len() as libc::socklen_t,
        );
        if ret != 0 {
            return Err("Failed to bind socket to device".into());
        }
    }

    let mac = mac.unwrap_or([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);

    net_log("Sending DHCPDISCOVER");
    let discover = create_dhcp_discover(mac)?;
    let mut buf = Vec::new();
    let mut encoder = Encoder::new(&mut buf);
    discover.encode(&mut encoder)?;

    let broadcast_addr = SocketAddr::from(([255, 255, 255, 255], DHCP_SERVER_PORT));
    socket.send_to(&buf, broadcast_addr).await?;

    let mut recv_buf = vec![0u8; 1500];
    match timeout(DHCP_TIMEOUT, socket.recv_from(&mut recv_buf)).await {
        Ok(Ok((len, _))) => {
            let mut decoder = Decoder::new(&recv_buf[..len]);
            if let Ok(offer) = Message::decode(&mut decoder) {
                net_log(&format!("Got DHCPOFFER for {}", offer.yiaddr()));
                let offered_ip = offer.yiaddr();
                let request = create_dhcp_request(&offer, offered_ip, mac)?;

                buf.clear();
                let mut encoder = Encoder::new(&mut buf);
                request.encode(&mut encoder)?;
                net_log("Sending DHCPREQUEST");
                socket.send_to(&buf, broadcast_addr).await?;

                match timeout(DHCP_TIMEOUT, socket.recv_from(&mut recv_buf)).await {
                    Ok(Ok((len, _))) => {
                        let mut decoder = Decoder::new(&recv_buf[..len]);
                        if let Ok(ack) = Message::decode(&mut decoder) {
                            if ack.opcode() == Opcode::BootReply {
                                let ip = ack.yiaddr();
                                let mask =
                                    match ack.opts().get(dhcproto::v4::OptionCode::SubnetMask) {
                                        Some(DhcpOption::SubnetMask(m)) => *m,
                                        _ => Ipv4Addr::new(255, 255, 255, 0),
                                    };
                                let router = match ack.opts().get(dhcproto::v4::OptionCode::Router)
                                {
                                    Some(DhcpOption::Router(list)) => list.first().copied(),
                                    _ => None,
                                };
                                let dns = match ack
                                    .opts()
                                    .get(dhcproto::v4::OptionCode::DomainNameServer)
                                {
                                    Some(DhcpOption::DomainNameServer(list)) => list.clone(),
                                    _ => Vec::new(),
                                };
                                net_log(&format!(
                                    "DHCP ACK: ip {} mask {}{}",
                                    ip,
                                    mask,
                                    router.map(|g| format!(" gw {}", g)).unwrap_or_default()
                                ));
                                return Ok(Some(DhcpLease {
                                    ip,
                                    mask,
                                    router,
                                    dns,
                                }));
                            }
                        }
                    }
                    _ => {
                        net_log("No DHCPACK, timed out");
                        return Ok(None);
                    }
                }
            }
        }
        _ => {
            net_log("No DHCPOFFER, timed out");
            return Ok(None);
        }
    }

    Ok(None)
}

fn create_dhcp_discover(mac: [u8; 6]) -> Result<Message, Box<dyn std::error::Error>> {
    let mut msg = Message::default();
    msg.set_opcode(dhcproto::v4::Opcode::BootRequest);
    msg.set_htype(dhcproto::v4::HType::Eth);
    msg.set_xid(rand::random());
    msg.set_flags(dhcproto::v4::Flags::default().set_broadcast());
    msg.set_chaddr(&mac);

    let mut opts = DhcpOptions::new();
    opts.insert(dhcproto::v4::DhcpOption::MessageType(
        dhcproto::v4::MessageType::Discover,
    ));
    opts.insert(dhcproto::v4::DhcpOption::ParameterRequestList(vec![
        dhcproto::v4::OptionCode::SubnetMask,
        dhcproto::v4::OptionCode::Router,
        dhcproto::v4::OptionCode::DomainNameServer,
    ]));
    msg.set_opts(opts);

    Ok(msg)
}

fn create_dhcp_request(
    offer: &Message,
    requested_ip: Ipv4Addr,
    mac: [u8; 6],
) -> Result<Message, Box<dyn std::error::Error>> {
    let mut msg = Message::default();
    msg.set_opcode(dhcproto::v4::Opcode::BootRequest);
    msg.set_htype(dhcproto::v4::HType::Eth);
    msg.set_xid(offer.xid());
    msg.set_flags(dhcproto::v4::Flags::default().set_broadcast());
    msg.set_chaddr(&mac);

    let mut opts = DhcpOptions::new();
    opts.insert(dhcproto::v4::DhcpOption::MessageType(
        dhcproto::v4::MessageType::Request,
    ));
    opts.insert(dhcproto::v4::DhcpOption::RequestedIpAddress(requested_ip));

    if let Some(server_id) = offer
        .opts()
        .get(dhcproto::v4::OptionCode::ServerIdentifier)
        .cloned()
    {
        opts.insert(server_id);
    }

    msg.set_opts(opts);
    Ok(msg)
}

async fn configure_interface(
    handle: &Handle,
    name: &str,
    lease: &DhcpLease,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    if let Some(link) = links.try_next().await? {
        let prefix = ipv4_mask_to_prefix(lease.mask);
        handle
            .address()
            .add(link.header.index, lease.ip.into(), prefix)
            .execute()
            .await?;

        if let Some(gateway) = lease.router {
            handle
                .route()
                .add()
                .v4()
                .destination_prefix(Ipv4Addr::new(0, 0, 0, 0), 0)
                .gateway(gateway)
                .execute()
                .await?;
        }

        net_log(&format!(
            "DHCP configured: {} / {}{}",
            lease.ip,
            lease.mask,
            lease
                .router
                .map(|g| format!(" gw {}", g))
                .unwrap_or_else(|| "".to_string())
        ));
    }

    Ok(())
}

fn ipv4_mask_to_prefix(mask: Ipv4Addr) -> u8 {
    let m = u32::from(mask);
    m.count_ones() as u8
}

fn write_resolv_conf(servers: &[Ipv4Addr]) {
    if servers.is_empty() {
        return;
    }
    let mut out = String::new();
    for s in servers {
        out.push_str(&format!("nameserver {}\n", s));
    }
    let _ = std::fs::create_dir_all("/etc");
    let _ = std::fs::write("/etc/resolv.conf", out);
}
