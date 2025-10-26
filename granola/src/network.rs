use crate::log;
use dhcproto::{v4, Decodable, Decoder, Encodable};
use futures::stream::TryStreamExt;
use netlink_packet_route::link::LinkAttribute;
use nix::libc;
use rand::Rng;
use rtnetlink::{new_connection, Handle};
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

async fn setup_loopback(handle: &Handle) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Setting up loopback interface");

    let mut links = handle.link().get().match_name("lo".to_string()).execute();
    if let Some(link) = links.try_next().await? {
        handle.link().set(link.header.index).up().execute().await?;
        log!("network", "Loopback interface is up");
    }

    Ok(())
}

async fn find_ethernet_interface(handle: &Handle) -> Result<String, Box<dyn std::error::Error>> {
    let mut links = handle.link().get().execute();

    while let Some(link) = links.try_next().await? {
        for attr in &link.attributes {
            if let LinkAttribute::IfName(name) = attr {
                if name.starts_with("eth") || name.starts_with("enp") {
                    log!("network", "Found ethernet interface: {}", name);
                    return Ok(name.clone());
                }
            }
        }
    }

    Err("No ethernet interface found".into())
}

async fn run_dhcp_client(
    interface: &str,
    handle: &Handle,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Starting DHCP client on {}", interface);

    let mut links = handle
        .link()
        .get()
        .match_name(interface.to_string())
        .execute();
    let link_index = if let Some(link) = links.try_next().await? {
        let index = link.header.index;
        handle.link().set(index).up().execute().await?;
        log!("network", "Interface {} is up", interface);
        index
    } else {
        return Err("Interface not found".into());
    };

    tokio::time::sleep(Duration::from_millis(100)).await;

    let socket = UdpSocket::bind("0.0.0.0:68").await?;
    socket.set_broadcast(true)?;

    unsafe {
        use std::os::unix::io::AsRawFd;
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

    let mut rng = rand::thread_rng();
    let xid: u32 = rng.gen();

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

    let mut request_msg = v4::Message::default()
        .set_flags(v4::Flags::default().set_broadcast())
        .set_chaddr(&crate::config::DEFAULT_MAC_ADDRESS)
        .set_xid(xid)
        .set_opcode(v4::Opcode::BootRequest)
        .to_vec()?;
    request_msg.extend(&[53, 1, 3]);
    request_msg.extend(&[50, 4]);
    request_msg.extend(&offer.yiaddr().octets());
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

    log!("network", "Network configuration complete");

    Ok(())
}

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Network manager started");

    let (connection, handle, _) = new_connection()?;
    tokio::spawn(connection);

    setup_loopback(&handle).await?;

    let interface = find_ethernet_interface(&handle).await?;

    run_dhcp_client(&interface, &handle).await?;

    log!("network", "Network manager exiting (no DHCP renewal implemented)");
    Ok(())
}
