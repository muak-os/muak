use crate::log;
use anyhow::Result;
use dhcproto::{Decodable, Decoder, Encodable, v4};
use nix::sys::socket::{setsockopt, sockopt::BindToDevice};
use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::model::{DhcpLease, IpConfig};

pub async fn run_dhcp_client(interface: &str, mac: &[u8; 6]) -> Result<(IpConfig, DhcpLease)> {
    log!("network", "DHCP: starting on {}", interface);

    let socket = UdpSocket::bind("0.0.0.0:68").await?;
    socket.set_broadcast(true)?;
    setsockopt(&socket, BindToDevice, &OsString::from(interface))?;

    let xid: u32 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32;

    let mut discover_msg = v4::Message::default()
        .set_flags(v4::Flags::default().set_broadcast())
        .set_chaddr(mac)
        .set_xid(xid)
        .set_opcode(v4::Opcode::BootRequest)
        .to_vec()?;
    discover_msg.extend(&[53, 1, 1]); // DHCPDISCOVER
    discover_msg.push(255);

    log!("network", "DHCP: sending DISCOVER xid={}", xid);
    socket.send_to(&discover_msg, "255.255.255.255:67").await?;

    let mut buf = [0u8; 1500];
    let (len, _) = timeout(Duration::from_secs(10), socket.recv_from(&mut buf)).await??;
    let mut decoder = Decoder::new(&buf[..len]);
    let offer = v4::Message::decode(&mut decoder)?;
    log!("network", "DHCP: got OFFER yiaddr={}", offer.yiaddr());

    // Extract server identifier
    let mut server_id: Option<Ipv4Addr> = None;
    for (_code, opt) in offer.opts().iter() {
        if let v4::DhcpOption::ServerIdentifier(sid) = opt {
            server_id = Some(*sid);
            break;
        }
    }
    let server_id =
        server_id.ok_or_else(|| anyhow::anyhow!("no server identifier in DHCPOFFER"))?;

    // Build REQUEST
    let mut request_msg = v4::Message::default()
        .set_flags(v4::Flags::default().set_broadcast())
        .set_chaddr(mac)
        .set_xid(xid)
        .set_opcode(v4::Opcode::BootRequest)
        .to_vec()?;
    request_msg.extend(&[53, 1, 3]); // DHCPREQUEST
    request_msg.extend(&[50, 4]); // Requested IP option
    request_msg.extend(&offer.yiaddr().octets());
    request_msg.extend(&[54, 4]); // Server ID option
    request_msg.extend(&server_id.octets());
    request_msg.push(255);

    log!("network", "DHCP: sending REQUEST for {}", offer.yiaddr());
    socket.send_to(&request_msg, "255.255.255.255:67").await?;

    let (len, _) = timeout(Duration::from_secs(10), socket.recv_from(&mut buf)).await??;
    let mut decoder = Decoder::new(&buf[..len]);
    let ack = v4::Message::decode(&mut decoder)?;
    log!("network", "DHCP: got ACK yiaddr={}", ack.yiaddr());

    let ip = ack.yiaddr();
    let mut netmask: Option<Ipv4Addr> = None;
    let mut gateway: Option<Ipv4Addr> = None;
    let mut dns_servers: Vec<Ipv4Addr> = Vec::new();
    let mut lease_seconds: u32 = 3600;

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
        .unwrap_or(24);

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
