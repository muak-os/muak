use anyhow::Result;
use dhcproto::{Decodable, Decoder, Encodable, v4};
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::model::{DhcpLease, IpConfig};
use crate::socket;

mod option {
    pub const SUBNET_MASK: u8 = 1;
    pub const ROUTER: u8 = 3;
    pub const DNS_SERVER: u8 = 6;
    pub const REQUESTED_IP: u8 = 50;
    pub const LEASE_TIME: u8 = 51;
    pub const MESSAGE_TYPE: u8 = 53;
    pub const SERVER_ID: u8 = 54;
    pub const PARAM_REQUEST_LIST: u8 = 55;
    pub const END: u8 = 255;
}

mod message_type {
    pub const DISCOVER: u8 = 1;
    pub const REQUEST: u8 = 3;
}

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;

const DHCP_TIMEOUT_SECS: u64 = 10;
const DEFAULT_LEASE_SECS: u32 = 3600;
const DEFAULT_PREFIX_LEN: u8 = 24;

fn append_param_request_list(msg: &mut Vec<u8>) {
    const REQUESTED_PARAMS: &[u8] = &[
        option::SUBNET_MASK,
        option::ROUTER,
        option::DNS_SERVER,
        option::LEASE_TIME,
    ];
    msg.push(option::PARAM_REQUEST_LIST);
    msg.push(REQUESTED_PARAMS.len() as u8);
    msg.extend_from_slice(REQUESTED_PARAMS);
}

pub async fn run_dhcp_client(interface: &str, mac: &[u8; 6]) -> Result<(IpConfig, DhcpLease)> {
    kmsg::info!("DHCP: starting on {}", interface);

    let socket = UdpSocket::bind(("0.0.0.0", DHCP_CLIENT_PORT)).await?;
    socket.set_broadcast(true)?;
    socket::socket_bind_device(&socket, interface)?;

    let xid: u32 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_nanos() as u32;

    let mut discover_msg = v4::Message::default()
        .set_flags(v4::Flags::default().set_broadcast())
        .set_chaddr(mac)
        .set_xid(xid)
        .set_opcode(v4::Opcode::BootRequest)
        .to_vec()?;
    discover_msg.extend(&[option::MESSAGE_TYPE, 1, message_type::DISCOVER]);
    append_param_request_list(&mut discover_msg);
    discover_msg.push(option::END);

    kmsg::info!("DHCP: sending DISCOVER xid={}", xid);
    socket
        .send_to(&discover_msg, ("255.255.255.255", DHCP_SERVER_PORT))
        .await?;

    let mut buf = [0u8; 1500];
    let (len, _) = timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        socket.recv_from(&mut buf),
    )
    .await??;
    let mut decoder = Decoder::new(&buf[..len]);
    let offer = v4::Message::decode(&mut decoder)?;
    kmsg::info!("DHCP: got OFFER yiaddr={}", offer.yiaddr());

    let mut server_id: Option<Ipv4Addr> = None;
    for (_code, opt) in offer.opts().iter() {
        if let v4::DhcpOption::ServerIdentifier(sid) = opt {
            server_id = Some(*sid);
            break;
        }
    }
    let server_id =
        server_id.ok_or_else(|| anyhow::anyhow!("no server identifier in DHCPOFFER"))?;

    let mut request_msg = v4::Message::default()
        .set_flags(v4::Flags::default().set_broadcast())
        .set_chaddr(mac)
        .set_xid(xid)
        .set_opcode(v4::Opcode::BootRequest)
        .to_vec()?;
    request_msg.extend(&[option::MESSAGE_TYPE, 1, message_type::REQUEST]);
    request_msg.extend(&[option::REQUESTED_IP, 4]);
    request_msg.extend(&offer.yiaddr().octets());
    request_msg.extend(&[option::SERVER_ID, 4]);
    request_msg.extend(&server_id.octets());
    append_param_request_list(&mut request_msg);
    request_msg.push(option::END);

    kmsg::info!("DHCP: sending REQUEST for {}", offer.yiaddr());
    socket
        .send_to(&request_msg, ("255.255.255.255", DHCP_SERVER_PORT))
        .await?;

    let (len, _) = timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        socket.recv_from(&mut buf),
    )
    .await??;
    let mut decoder = Decoder::new(&buf[..len]);
    let ack = v4::Message::decode(&mut decoder)?;
    kmsg::info!("DHCP: got ACK yiaddr={}", ack.yiaddr());

    let ip = ack.yiaddr();
    let mut netmask: Option<Ipv4Addr> = None;
    let mut gateway: Option<Ipv4Addr> = None;
    let mut dns_servers: Vec<Ipv4Addr> = Vec::new();
    let mut lease_seconds: u32 = DEFAULT_LEASE_SECS;

    for (_code, opt) in ack.opts().iter() {
        match opt {
            v4::DhcpOption::SubnetMask(mask) => netmask = Some(*mask),
            v4::DhcpOption::Router(routers) if !routers.is_empty() => gateway = Some(routers[0]),
            v4::DhcpOption::DomainNameServer(servers) => dns_servers = servers.clone(),
            v4::DhcpOption::AddressLeaseTime(ls) => lease_seconds = *ls,
            _ => {}
        }
    }

    let prefix_len: u8 = netmask
        .map(|m| m.octets().iter().map(|b| b.count_ones()).sum::<u32>() as u8)
        .unwrap_or(DEFAULT_PREFIX_LEN);

    let ip_cfg = IpConfig {
        address: ip,
        prefix_len,
        gateway,
        dns: dns_servers.clone(),
    };

    let renewal = lease_seconds / 2; // T1
    let rebind = (lease_seconds * 7) / 8; // T2
    let lease = DhcpLease {
        obtained_at: SystemTime::now(),
        lease_time: Duration::from_secs(lease_seconds as u64),
        renewal_time: Duration::from_secs(renewal as u64),
        rebind_time: Duration::from_secs(rebind as u64),
    };

    Ok((ip_cfg, lease))
}
