use super::ip_allocator::IpAllocator;
use crate::log;
use dhcproto::{v4, Decodable, Decoder, Encodable};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::net::UdpSocket;

pub struct DhcpServer {
    server_ip: Ipv4Addr,
    netmask: Ipv4Addr,
    gateway: Ipv4Addr,
    lease_time: u32,
    allocator: Arc<IpAllocator>,
    leases: Arc<Mutex<HashMap<[u8; 6], Ipv4Addr>>>, // MAC -> IP mapping
}

impl DhcpServer {
    pub fn new(
        server_ip: Ipv4Addr,
        netmask: Ipv4Addr,
        gateway: Ipv4Addr,
        lease_time: u32,
        allocator: Arc<IpAllocator>,
    ) -> Self {
        Self {
            server_ip,
            netmask,
            gateway,
            lease_time,
            allocator,
            leases: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn run(&self, bridge_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        log!("dhcp", "Starting DHCP server on {}", self.server_ip);

        let socket = UdpSocket::bind("0.0.0.0:67").await?;
        socket.set_broadcast(true)?;

        // Bind to the bridge interface
        unsafe {
            use std::os::unix::io::AsRawFd;
            let fd = socket.as_raw_fd();
            let iface_cstr = std::ffi::CString::new(bridge_name)?;
            let ret = nix::libc::setsockopt(
                fd,
                nix::libc::SOL_SOCKET,
                nix::libc::SO_BINDTODEVICE,
                iface_cstr.as_ptr() as *const nix::libc::c_void,
                iface_cstr.as_bytes_with_nul().len() as nix::libc::socklen_t,
            );
            if ret != 0 {
                return Err("Failed to bind socket to bridge device".into());
            }
        }

        log!("dhcp", "DHCP server listening on {}:67", self.server_ip);

        let mut buf = [0u8; 1500];
        loop {
            let (len, src) = socket.recv_from(&mut buf).await?;

            if let Err(e) = self.handle_dhcp_packet(&socket, &buf[..len], src).await {
                log!("dhcp", "Error handling DHCP packet: {}", e);
            }
        }
    }

    async fn handle_dhcp_packet(
        &self,
        socket: &UdpSocket,
        data: &[u8],
        _src: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut decoder = Decoder::new(data);
        let msg = v4::Message::decode(&mut decoder)?;

        let client_mac = msg.chaddr();
        let xid = msg.xid();

        // Find DHCP message type
        let mut msg_type: Option<v4::MessageType> = None;
        for (_code, opt) in msg.opts().iter() {
            if let v4::DhcpOption::MessageType(mt) = opt {
                msg_type = Some(*mt);
                break;
            }
        }

        match msg_type {
            Some(v4::MessageType::Discover) => {
                // DHCPDISCOVER
                log!(
                    "dhcp",
                    "Received DHCPDISCOVER from {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    client_mac[0],
                    client_mac[1],
                    client_mac[2],
                    client_mac[3],
                    client_mac[4],
                    client_mac[5]
                );

                self.send_offer(socket, xid, client_mac).await?;
            }
            Some(v4::MessageType::Request) => {
                // DHCPREQUEST
                log!(
                    "dhcp",
                    "Received DHCPREQUEST from {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    client_mac[0],
                    client_mac[1],
                    client_mac[2],
                    client_mac[3],
                    client_mac[4],
                    client_mac[5]
                );

                self.send_ack(socket, xid, client_mac).await?;
            }
            Some(v4::MessageType::Release) => {
                // DHCPRELEASE
                log!(
                    "dhcp",
                    "Received DHCPRELEASE from {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    client_mac[0],
                    client_mac[1],
                    client_mac[2],
                    client_mac[3],
                    client_mac[4],
                    client_mac[5]
                );

                self.handle_release(client_mac).await?;
            }
            _ => {
                log!("dhcp", "Received unknown DHCP message type: {:?}", msg_type);
            }
        }

        Ok(())
    }

    async fn send_offer(
        &self,
        socket: &UdpSocket,
        xid: u32,
        client_mac: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let offered_ip = self.get_or_allocate_ip(client_mac)?;

        let mut offer = v4::Message::default()
            .set_opcode(v4::Opcode::BootReply)
            .set_xid(xid)
            .set_flags(v4::Flags::default().set_broadcast())
            .set_yiaddr(offered_ip)
            .set_siaddr(self.server_ip)
            .set_chaddr(client_mac)
            .to_vec()?;

        // Add DHCP options
        offer.extend(&[53, 1, 2]); // Message type: DHCPOFFER
        offer.extend(&[54, 4]); // Server identifier
        offer.extend(&self.server_ip.octets());
        offer.extend(&[51, 4]); // Lease time
        offer.extend(&self.lease_time.to_be_bytes());
        offer.extend(&[1, 4]); // Subnet mask
        offer.extend(&self.netmask.octets());
        offer.extend(&[3, 4]); // Router
        offer.extend(&self.gateway.octets());
        offer.push(255); // End

        socket.send_to(&offer, "255.255.255.255:68").await?;
        log!("dhcp", "Sent DHCPOFFER: {} to client", offered_ip);

        Ok(())
    }

    async fn send_ack(
        &self,
        socket: &UdpSocket,
        xid: u32,
        client_mac: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let assigned_ip = self.get_or_allocate_ip(client_mac)?;

        let mut ack = v4::Message::default()
            .set_opcode(v4::Opcode::BootReply)
            .set_xid(xid)
            .set_flags(v4::Flags::default().set_broadcast())
            .set_yiaddr(assigned_ip)
            .set_siaddr(self.server_ip)
            .set_chaddr(client_mac)
            .to_vec()?;

        // Add DHCP options
        ack.extend(&[53, 1, 5]); // Message type: DHCPACK
        ack.extend(&[54, 4]); // Server identifier
        ack.extend(&self.server_ip.octets());
        ack.extend(&[51, 4]); // Lease time
        ack.extend(&self.lease_time.to_be_bytes());
        ack.extend(&[1, 4]); // Subnet mask
        ack.extend(&self.netmask.octets());
        ack.extend(&[3, 4]); // Router
        ack.extend(&self.gateway.octets());
        ack.push(255); // End

        socket.send_to(&ack, "255.255.255.255:68").await?;
        log!("dhcp", "Sent DHCPACK: {} to client", assigned_ip);

        Ok(())
    }

    fn get_or_allocate_ip(
        &self,
        client_mac: &[u8],
    ) -> Result<Ipv4Addr, Box<dyn std::error::Error>> {
        let mut mac_array = [0u8; 6];
        mac_array.copy_from_slice(&client_mac[0..6]);

        let mut leases = self.leases.lock().unwrap();

        if let Some(&ip) = leases.get(&mac_array) {
            Ok(ip)
        } else {
            let ip = self.allocator.allocate()?;
            leases.insert(mac_array, ip);
            Ok(ip)
        }
    }

    async fn handle_release(&self, client_mac: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let mut mac_array = [0u8; 6];
        mac_array.copy_from_slice(&client_mac[0..6]);

        let mut leases = self.leases.lock().unwrap();

        if let Some(ip) = leases.remove(&mac_array) {
            drop(leases); // Release the lock before calling allocator
            self.allocator.release(ip)?;
            log!("dhcp", "Released IP {} for client", ip);
        }

        Ok(())
    }
}
