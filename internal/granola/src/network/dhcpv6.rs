use crate::log;
use anyhow::Result;
use nix::sys::socket::{setsockopt, sockopt::BindToDevice};
use std::ffi::OsString;
use std::net::Ipv6Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::model::{DhcpLease, Ipv6Config};

/// DHCPv6 multicast address for all DHCP relay agents and servers
const DHCPV6_ALL_SERVERS: &str = "ff02::1:2";
/// DHCPv6 client port
const DHCPV6_CLIENT_PORT: u16 = 546;
/// DHCPv6 server port  
const DHCPV6_SERVER_PORT: u16 = 547;

/// Generate a DUID-LL (Link-Layer) from MAC address
/// Format: type (2 bytes) + hardware type (2 bytes) + MAC (6 bytes)
fn generate_duid_ll(mac: &[u8; 6]) -> Vec<u8> {
    let mut duid = Vec::with_capacity(10);
    // DUID type 3 = DUID-LL (Link-layer address)
    duid.extend(&[0x00, 0x03]);
    // Hardware type 1 = Ethernet
    duid.extend(&[0x00, 0x01]);
    // MAC address
    duid.extend(mac);
    duid
}

/// Generate a unique IAID from interface index
fn generate_iaid(interface: &str) -> u32 {
    // Simple hash of interface name for consistent IAID
    interface.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32).wrapping_mul(31))
}

/// Build DHCPv6 SOLICIT message
fn build_solicit_message(xid: u32, duid: &[u8], iaid: u32) -> Result<Vec<u8>> {
    // Message type: SOLICIT (1)
    let mut msg = vec![0x01];
    
    // Transaction ID (3 bytes)
    msg.push(((xid >> 16) & 0xFF) as u8);
    msg.push(((xid >> 8) & 0xFF) as u8);
    msg.push((xid & 0xFF) as u8);
    
    // Option: Client Identifier (1)
    // Format: option-code (2) + option-len (2) + DUID
    msg.extend(&[0x00, 0x01]); // Option code 1
    msg.extend(&((duid.len() as u16).to_be_bytes())); // Option length
    msg.extend(duid);
    
    // Option: IA_NA (3) - Identity Association for Non-temporary Addresses
    // Format: option-code (2) + option-len (2) + IAID (4) + T1 (4) + T2 (4) + IA options
    msg.extend(&[0x00, 0x03]); // Option code 3
    msg.extend(&[0x00, 0x0c]); // Option length (12 bytes: IAID + T1 + T2, no sub-options)
    msg.extend(&iaid.to_be_bytes()); // IAID
    msg.extend(&[0x00, 0x00, 0x00, 0x00]); // T1 = 0 (let server decide)
    msg.extend(&[0x00, 0x00, 0x00, 0x00]); // T2 = 0 (let server decide)
    
    // Option: Elapsed Time (8)
    msg.extend(&[0x00, 0x08]); // Option code 8
    msg.extend(&[0x00, 0x02]); // Option length 2
    msg.extend(&[0x00, 0x00]); // Elapsed time = 0 (first message)
    
    // Option: Option Request (6) - request DNS servers
    msg.extend(&[0x00, 0x06]); // Option code 6
    msg.extend(&[0x00, 0x02]); // Option length 2
    msg.extend(&[0x00, 0x17]); // Request option 23 (DNS recursive name server)
    
    Ok(msg)
}

/// Build DHCPv6 REQUEST message
fn build_request_message(
    xid: u32,
    client_duid: &[u8],
    server_duid: &[u8],
    iaid: u32,
    address: Ipv6Addr,
    preferred_lifetime: u32,
    valid_lifetime: u32,
) -> Result<Vec<u8>> {
    // Message type: REQUEST (3)
    let mut msg = vec![0x03];
    
    // Transaction ID (3 bytes)
    msg.push(((xid >> 16) & 0xFF) as u8);
    msg.push(((xid >> 8) & 0xFF) as u8);
    msg.push((xid & 0xFF) as u8);
    
    // Option: Client Identifier (1)
    msg.extend(&[0x00, 0x01]);
    msg.extend(&((client_duid.len() as u16).to_be_bytes()));
    msg.extend(client_duid);
    
    // Option: Server Identifier (2)
    msg.extend(&[0x00, 0x02]);
    msg.extend(&((server_duid.len() as u16).to_be_bytes()));
    msg.extend(server_duid);
    
    // Option: IA_NA (3) with IA Address sub-option
    let ia_addr_len = 24u16; // 16 (addr) + 4 (preferred) + 4 (valid)
    let ia_na_len = 12 + 4 + ia_addr_len; // IAID + T1 + T2 + sub-option header + IA Address
    
    msg.extend(&[0x00, 0x03]); // Option code 3
    msg.extend(&ia_na_len.to_be_bytes());
    msg.extend(&iaid.to_be_bytes()); // IAID
    msg.extend(&[0x00, 0x00, 0x00, 0x00]); // T1
    msg.extend(&[0x00, 0x00, 0x00, 0x00]); // T2
    
    // Sub-option: IA Address (5)
    msg.extend(&[0x00, 0x05]); // Option code 5
    msg.extend(&ia_addr_len.to_be_bytes());
    msg.extend(&address.octets());
    msg.extend(&preferred_lifetime.to_be_bytes());
    msg.extend(&valid_lifetime.to_be_bytes());
    
    // Option: Elapsed Time (8)
    msg.extend(&[0x00, 0x08]);
    msg.extend(&[0x00, 0x02]);
    msg.extend(&[0x00, 0x00]);
    
    // Option: Option Request (6)
    msg.extend(&[0x00, 0x06]);
    msg.extend(&[0x00, 0x02]);
    msg.extend(&[0x00, 0x17]); // DNS servers
    
    Ok(msg)
}

/// Parse DHCPv6 message and extract options
struct Dhcpv6Response {
    msg_type: u8,
    xid: u32,
    server_duid: Option<Vec<u8>>,
    address: Option<Ipv6Addr>,
    preferred_lifetime: u32,
    valid_lifetime: u32,
    dns_servers: Vec<Ipv6Addr>,
    t1: u32,
    t2: u32,
}

fn parse_response(data: &[u8]) -> Result<Dhcpv6Response> {
    if data.len() < 4 {
        anyhow::bail!("DHCPv6 message too short");
    }
    
    let msg_type = data[0];
    let xid = ((data[1] as u32) << 16) | ((data[2] as u32) << 8) | (data[3] as u32);
    
    let mut response = Dhcpv6Response {
        msg_type,
        xid,
        server_duid: None,
        address: None,
        preferred_lifetime: 0,
        valid_lifetime: 0,
        dns_servers: Vec::new(),
        t1: 0,
        t2: 0,
    };
    
    let mut pos = 4;
    while pos + 4 <= data.len() {
        let opt_code = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let opt_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        
        if pos + opt_len > data.len() {
            break;
        }
        
        let opt_data = &data[pos..pos + opt_len];
        
        match opt_code {
            2 => {
                // Server Identifier
                response.server_duid = Some(opt_data.to_vec());
            }
            3 => {
                // IA_NA - parse sub-options for address
                if opt_len >= 12 {
                    response.t1 = u32::from_be_bytes([opt_data[4], opt_data[5], opt_data[6], opt_data[7]]);
                    response.t2 = u32::from_be_bytes([opt_data[8], opt_data[9], opt_data[10], opt_data[11]]);
                    
                    // Parse IA_NA sub-options
                    let mut sub_pos = 12;
                    while sub_pos + 4 <= opt_len {
                        let sub_code = u16::from_be_bytes([opt_data[sub_pos], opt_data[sub_pos + 1]]);
                        let sub_len = u16::from_be_bytes([opt_data[sub_pos + 2], opt_data[sub_pos + 3]]) as usize;
                        sub_pos += 4;
                        
                        if sub_pos + sub_len > opt_len {
                            break;
                        }
                        
                        if sub_code == 5 && sub_len >= 24 {
                            // IA Address
                            let addr_bytes: [u8; 16] = opt_data[sub_pos..sub_pos + 16].try_into()?;
                            response.address = Some(Ipv6Addr::from(addr_bytes));
                            response.preferred_lifetime = u32::from_be_bytes([
                                opt_data[sub_pos + 16],
                                opt_data[sub_pos + 17],
                                opt_data[sub_pos + 18],
                                opt_data[sub_pos + 19],
                            ]);
                            response.valid_lifetime = u32::from_be_bytes([
                                opt_data[sub_pos + 20],
                                opt_data[sub_pos + 21],
                                opt_data[sub_pos + 22],
                                opt_data[sub_pos + 23],
                            ]);
                        }
                        
                        sub_pos += sub_len;
                    }
                }
            }
            23 => {
                // DNS Recursive Name Server
                let mut dns_pos = 0;
                while dns_pos + 16 <= opt_len {
                    let addr_bytes: [u8; 16] = opt_data[dns_pos..dns_pos + 16].try_into()?;
                    response.dns_servers.push(Ipv6Addr::from(addr_bytes));
                    dns_pos += 16;
                }
            }
            _ => {}
        }
        
        pos += opt_len;
    }
    
    Ok(response)
}

pub async fn run_dhcpv6_client(interface: &str, mac: &[u8; 6]) -> Result<(Ipv6Config, DhcpLease)> {
    log!("network", "DHCPv6: starting on {}", interface);
    
    // Bind to link-local address on client port
    // Use [::] to bind to any IPv6 address
    let bind_addr = format!("[::]:{}", DHCPV6_CLIENT_PORT);
    let socket = UdpSocket::bind(&bind_addr).await?;
    setsockopt(&socket, BindToDevice, &OsString::from(interface))?;
    
    // Generate client identifiers
    let duid = generate_duid_ll(mac);
    let iaid = generate_iaid(interface);
    let xid: u32 = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32) & 0x00FFFFFF; // Only 24 bits for DHCPv6 XID
    
    // Build and send SOLICIT
    let solicit_msg = build_solicit_message(xid, &duid, iaid)?;
    let server_addr = format!("[{}%{}]:{}", DHCPV6_ALL_SERVERS, interface, DHCPV6_SERVER_PORT);
    
    log!("network", "DHCPv6: sending SOLICIT xid={:06x}", xid);
    socket.send_to(&solicit_msg, &server_addr).await?;
    
    // Receive ADVERTISE
    let mut buf = [0u8; 1500];
    let (len, _) = timeout(Duration::from_secs(10), socket.recv_from(&mut buf)).await??;
    let advertise = parse_response(&buf[..len])?;
    
    if advertise.msg_type != 2 {
        anyhow::bail!("Expected ADVERTISE (2), got message type {}", advertise.msg_type);
    }
    if advertise.xid != xid {
        anyhow::bail!("XID mismatch: expected {:06x}, got {:06x}", xid, advertise.xid);
    }
    
    let server_duid = advertise.server_duid
        .ok_or_else(|| anyhow::anyhow!("No server DUID in ADVERTISE"))?;
    let offered_addr = advertise.address
        .ok_or_else(|| anyhow::anyhow!("No address in ADVERTISE"))?;
    
    log!("network", "DHCPv6: got ADVERTISE address={}", offered_addr);
    
    // Build and send REQUEST
    let request_msg = build_request_message(
        xid,
        &duid,
        &server_duid,
        iaid,
        offered_addr,
        advertise.preferred_lifetime,
        advertise.valid_lifetime,
    )?;
    
    log!("network", "DHCPv6: sending REQUEST for {}", offered_addr);
    socket.send_to(&request_msg, &server_addr).await?;
    
    // Receive REPLY
    let (len, _) = timeout(Duration::from_secs(10), socket.recv_from(&mut buf)).await??;
    let reply = parse_response(&buf[..len])?;
    
    if reply.msg_type != 7 {
        anyhow::bail!("Expected REPLY (7), got message type {}", reply.msg_type);
    }
    
    let address = reply.address
        .ok_or_else(|| anyhow::anyhow!("No address in REPLY"))?;
    
    log!("network", "DHCPv6: got REPLY address={}", address);
    
    // Build IPv6 config
    // DHCPv6 doesn't provide prefix length directly, assume /128 for the address
    // Gateway comes from Router Advertisement, not DHCPv6
    let ipv6_cfg = Ipv6Config {
        address,
        prefix_len: 128,
        gateway: None, // Obtained via Router Advertisement, not DHCPv6
        dns: reply.dns_servers.clone(),
    };
    
    // Calculate lease times
    let valid_lifetime = reply.valid_lifetime;
    let t1 = if reply.t1 > 0 { reply.t1 } else { valid_lifetime / 2 };
    let t2 = if reply.t2 > 0 { reply.t2 } else { (valid_lifetime * 7) / 8 };
    
    let lease = DhcpLease {
        obtained_at: SystemTime::now(),
        lease_time: Duration::from_secs(valid_lifetime as u64),
        renewal_time: Duration::from_secs(t1 as u64),
        rebind_time: Duration::from_secs(t2 as u64),
    };
    
    log!(
        "network",
        "DHCPv6: acquired {} with {} DNS servers, valid for {}s",
        address,
        reply.dns_servers.len(),
        valid_lifetime
    );
    
    Ok((ipv6_cfg, lease))
}
