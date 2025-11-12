use crate::log;
use dhcproto::{Decodable, Decoder, Encodable, v4};
use nix::sys::socket::{setsockopt, sockopt::BindToDevice};
use rtnetlink::Handle;
use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::time::timeout;

pub async fn run_dhcp_client(
    interface: &str,
    handle: &Handle,
    link_index: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Starting DHCP client on {}", interface);

    tokio::time::sleep(Duration::from_millis(100)).await;

    let socket = UdpSocket::bind("0.0.0.0:68").await?;
    socket.set_broadcast(true)?;

    setsockopt(&socket, BindToDevice, &OsString::from(interface))?;

    let xid: u32 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32;

    let mut discover_msg = v4::Message::default()
        .set_flags(v4::Flags::default().set_broadcast())
        .set_chaddr(&crate::config::DEFAULT_MAC_ADDRESS)
        .set_xid(xid)
        .set_opcode(v4::Opcode::BootRequest)
        .to_vec()?;
    discover_msg.extend(&[53, 1, 1]);
    discover_msg.push(255);

    log!("network", "Sending DHCPDISCOVER");
    socket.send_to(&discover_msg, "255.255.255.255:67").await?;

    let mut buf = [0u8; 1500];
    let (len, _) = timeout(Duration::from_secs(10), socket.recv_from(&mut buf)).await??;

    let mut decoder = Decoder::new(&buf[..len]);
    let offer = v4::Message::decode(&mut decoder)?;
    log!("network", "Received DHCPOFFER: {}", offer.yiaddr());

    // Extract server identifier from DHCPOFFER
    let mut server_id: Option<Ipv4Addr> = None;
    for (_code, opt) in offer.opts().iter() {
        if let v4::DhcpOption::ServerIdentifier(sid) = opt {
            server_id = Some(*sid);
            break;
        }
    }
    let server_id = server_id.ok_or("No server identifier in DHCPOFFER")?;

    let mut request_msg = v4::Message::default()
        .set_flags(v4::Flags::default().set_broadcast())
        .set_chaddr(&crate::config::DEFAULT_MAC_ADDRESS)
        .set_xid(xid)
        .set_opcode(v4::Opcode::BootRequest)
        .to_vec()?;
    request_msg.extend(&[53, 1, 3]); // Option 53: DHCP Message Type = REQUEST
    request_msg.extend(&[50, 4]); // Option 50: Requested IP Address
    request_msg.extend(&offer.yiaddr().octets());
    request_msg.extend(&[54, 4]); // Option 54: Server Identifier
    request_msg.extend(&server_id.octets());
    request_msg.push(255);

    log!("network", "Sending DHCPREQUEST");
    socket.send_to(&request_msg, "255.255.255.255:67").await?;

    let (len, _) = timeout(Duration::from_secs(10), socket.recv_from(&mut buf)).await??;

    let mut decoder = Decoder::new(&buf[..len]);
    let ack = v4::Message::decode(&mut decoder)?;

    log!("network", "Received DHCPACK: {}", ack.yiaddr());

    let ip = ack.yiaddr();
    let mut netmask: Option<Ipv4Addr> = None;
    let mut gateway: Option<Ipv4Addr> = None;
    let mut dns_servers: Vec<Ipv4Addr> = Vec::new();

    for (_code, opt) in ack.opts().iter() {
        match opt {
            v4::DhcpOption::SubnetMask(mask) => {
                netmask = Some(*mask);
            }
            v4::DhcpOption::Router(routers) => {
                if !routers.is_empty() {
                    gateway = Some(routers[0]);
                }
            }
            v4::DhcpOption::DomainNameServer(servers) => {
                dns_servers = servers.clone();
            }
            _ => {}
        }
    }

    let prefix_len: u8 = netmask
        .map(|m| m.octets().iter().map(|b| b.count_ones()).sum::<u32>() as u8)
        .unwrap_or(24);

    log!(
        "network",
        "Configuring interface {} with IP {}/{}",
        interface,
        ip,
        prefix_len
    );

    handle
        .address()
        .add(link_index, ip.into(), prefix_len)
        .execute()
        .await?;

    if let Some(gw) = gateway {
        log!("network", "Setting default gateway: {}", gw);
        handle.route().add().v4().gateway(gw).execute().await?;
    }

    if !dns_servers.is_empty() {
        super::dns::configure_dns(&dns_servers)?;
    }

    Ok(())
}
