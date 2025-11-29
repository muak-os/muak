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

/// DHCPv6 retry configuration (based on RFC 8415)
const DHCPV6_SOL_MAX_RT: u64 = 120;  // Max SOLICIT timeout (seconds)
const DHCPV6_REQ_MAX_RT: u64 = 30;   // Max REQUEST timeout (seconds)
const DHCPV6_MAX_RETRIES: u32 = 4;   // Max retry attempts
const DHCPV6_INITIAL_TIMEOUT: u64 = 1; // Initial timeout (seconds)

/// Build DHCPv6 RENEW message (type 5)
fn build_renew_message(
    xid: u32,
    client_duid: &[u8],
    server_duid: &[u8],
    iaid: u32,
    address: Ipv6Addr,
    t1: u32,
    t2: u32,
) -> Result<Vec<u8>> {
    // Message type: RENEW (5)
    let mut msg = vec![0x05];
    
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
    msg.extend(&t1.to_be_bytes());   // T1
    msg.extend(&t2.to_be_bytes());   // T2
    
    // Sub-option: IA Address (5)
    msg.extend(&[0x00, 0x05]); // Option code 5
    msg.extend(&ia_addr_len.to_be_bytes());
    msg.extend(&address.octets());
    msg.extend(&0u32.to_be_bytes()); // preferred-lifetime (0 = use server default)
    msg.extend(&0u32.to_be_bytes()); // valid-lifetime (0 = use server default)
    
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

/// Internal function to perform DHCPv6 SOLICIT-ADVERTISE-REQUEST-REPLY handshake
async fn dhcpv6_full_handshake(
    socket: &UdpSocket,
    interface: &str,
    mac: &[u8; 6],
) -> Result<(Ipv6Config, DhcpLease, Vec<u8>)> {
    let duid = generate_duid_ll(mac);
    let iaid = generate_iaid(interface);
    let xid: u32 = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32) & 0x00FFFFFF;
    
    let server_addr = format!("[{}%{}]:{}", DHCPV6_ALL_SERVERS, interface, DHCPV6_SERVER_PORT);
    
    // Phase 1: SOLICIT with retries
    let solicit_msg = build_solicit_message(xid, &duid, iaid)?;
    let advertise = retry_send_receive(
        socket,
        &solicit_msg,
        &server_addr,
        xid,
        2, // Expected: ADVERTISE
        DHCPV6_SOL_MAX_RT,
        "SOLICIT",
    ).await?;
    
    let server_duid = advertise.server_duid
        .ok_or_else(|| anyhow::anyhow!("No server DUID in ADVERTISE"))?;
    let offered_addr = advertise.address
        .ok_or_else(|| anyhow::anyhow!("No address in ADVERTISE"))?;
    
    log!("network", "DHCPv6: got ADVERTISE address={}", offered_addr);
    
    // Phase 2: REQUEST with retries
    let request_msg = build_request_message(
        xid,
        &duid,
        &server_duid,
        iaid,
        offered_addr,
        advertise.preferred_lifetime,
        advertise.valid_lifetime,
    )?;
    
    let reply = retry_send_receive(
        socket,
        &request_msg,
        &server_addr,
        xid,
        7, // Expected: REPLY
        DHCPV6_REQ_MAX_RT,
        "REQUEST",
    ).await?;
    
    let address = reply.address
        .ok_or_else(|| anyhow::anyhow!("No address in REPLY"))?;
    
    log!("network", "DHCPv6: got REPLY address={}", address);
    
    // Build result
    let ipv6_cfg = Ipv6Config {
        address,
        prefix_len: 128,
        gateway: None,
        dns: reply.dns_servers.clone(),
    };
    
    let valid_lifetime = reply.valid_lifetime;
    let t1 = if reply.t1 > 0 { reply.t1 } else { valid_lifetime / 2 };
    let t2 = if reply.t2 > 0 { reply.t2 } else { (valid_lifetime * 7) / 8 };
    
    let lease = DhcpLease {
        obtained_at: SystemTime::now(),
        lease_time: Duration::from_secs(valid_lifetime as u64),
        renewal_time: Duration::from_secs(t1 as u64),
        rebind_time: Duration::from_secs(t2 as u64),
    };
    
    Ok((ipv6_cfg, lease, server_duid))
}

/// Retry sending a DHCPv6 message and receiving a response
async fn retry_send_receive(
    socket: &UdpSocket,
    msg: &[u8],
    server_addr: &str,
    expected_xid: u32,
    expected_msg_type: u8,
    max_timeout: u64,
    msg_name: &str,
) -> Result<Dhcpv6Response> {
    let mut current_timeout = DHCPV6_INITIAL_TIMEOUT;
    let mut buf = [0u8; 1500];
    
    for attempt in 1..=DHCPV6_MAX_RETRIES {
        log!("network", "DHCPv6: sending {} (attempt {}/{})", msg_name, attempt, DHCPV6_MAX_RETRIES);
        
        socket.send_to(msg, server_addr).await?;
        
        match timeout(Duration::from_secs(current_timeout), socket.recv_from(&mut buf)).await {
            Ok(Ok((len, _))) => {
                let response = parse_response(&buf[..len])?;
                
                if response.xid != expected_xid {
                    log!("network", "DHCPv6: XID mismatch, ignoring");
                    continue;
                }
                
                if response.msg_type != expected_msg_type {
                    log!("network", "DHCPv6: unexpected message type {}, expected {}", 
                         response.msg_type, expected_msg_type);
                    continue;
                }
                
                return Ok(response);
            }
            Ok(Err(e)) => {
                log!("network", "DHCPv6: receive error: {}", e);
            }
            Err(_) => {
                log!("network", "DHCPv6: timeout waiting for response");
            }
        }
        
        // Exponential backoff with cap
        current_timeout = (current_timeout * 2).min(max_timeout);
    }
    
    anyhow::bail!("DHCPv6: {} failed after {} attempts", msg_name, DHCPV6_MAX_RETRIES)
}

pub async fn run_dhcpv6_client(interface: &str, mac: &[u8; 6]) -> Result<(Ipv6Config, DhcpLease)> {
    log!("network", "DHCPv6: starting on {}", interface);
    
    let bind_addr = format!("[::]:{}", DHCPV6_CLIENT_PORT);
    let socket = UdpSocket::bind(&bind_addr).await?;
    setsockopt(&socket, BindToDevice, &OsString::from(interface))?;
    
    let (ipv6_cfg, lease, _server_duid) = dhcpv6_full_handshake(&socket, interface, mac).await?;
    
    log!(
        "network",
        "DHCPv6: acquired {} with {} DNS servers, valid for {}s",
        ipv6_cfg.address,
        ipv6_cfg.dns.len(),
        lease.lease_time.as_secs()
    );
    
    Ok((ipv6_cfg, lease))
}

/// Renewal context containing server info from initial lease
#[derive(Debug, Clone)]
pub struct Dhcpv6RenewalContext {
    pub server_duid: Vec<u8>,
    pub client_duid: Vec<u8>,
    pub iaid: u32,
}

/// Extended run_dhcpv6_client that also returns renewal context
pub async fn run_dhcpv6_client_with_context(
    interface: &str,
    mac: &[u8; 6],
) -> Result<(Ipv6Config, DhcpLease, Dhcpv6RenewalContext)> {
    log!("network", "DHCPv6: starting on {}", interface);
    
    let bind_addr = format!("[::]:{}", DHCPV6_CLIENT_PORT);
    let socket = UdpSocket::bind(&bind_addr).await?;
    setsockopt(&socket, BindToDevice, &OsString::from(interface))?;
    
    let client_duid = generate_duid_ll(mac);
    let iaid = generate_iaid(interface);
    
    let (ipv6_cfg, lease, server_duid) = dhcpv6_full_handshake(&socket, interface, mac).await?;
    
    let context = Dhcpv6RenewalContext {
        server_duid,
        client_duid,
        iaid,
    };
    
    log!(
        "network",
        "DHCPv6: acquired {} with {} DNS servers, valid for {}s",
        ipv6_cfg.address,
        ipv6_cfg.dns.len(),
        lease.lease_time.as_secs()
    );
    
    Ok((ipv6_cfg, lease, context))
}

/// Renew an existing DHCPv6 lease using RENEW message (type 5)
pub async fn renew_dhcpv6_lease(
    interface: &str,
    current_config: &Ipv6Config,
    context: &Dhcpv6RenewalContext,
) -> Result<(Ipv6Config, DhcpLease)> {
    log!("network", "DHCPv6: renewing lease for {} on {}", current_config.address, interface);
    
    let bind_addr = format!("[::]:{}", DHCPV6_CLIENT_PORT);
    let socket = UdpSocket::bind(&bind_addr).await?;
    setsockopt(&socket, BindToDevice, &OsString::from(interface))?;
    
    let xid: u32 = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u32) & 0x00FFFFFF;
    
    let server_addr = format!("[{}%{}]:{}", DHCPV6_ALL_SERVERS, interface, DHCPV6_SERVER_PORT);
    
    // Build RENEW message with current address info
    let renew_msg = build_renew_message(
        xid,
        &context.client_duid,
        &context.server_duid,
        context.iaid,
        current_config.address,
        0, // T1 - server will provide new values
        0, // T2
    )?;
    
    // Send RENEW with retries
    let reply = retry_send_receive(
        &socket,
        &renew_msg,
        &server_addr,
        xid,
        7, // Expected: REPLY
        DHCPV6_REQ_MAX_RT,
        "RENEW",
    ).await?;
    
    let address = reply.address
        .ok_or_else(|| anyhow::anyhow!("No address in REPLY to RENEW"))?;
    
    log!("network", "DHCPv6: renewed lease for {}", address);
    
    let ipv6_cfg = Ipv6Config {
        address,
        prefix_len: 128,
        gateway: current_config.gateway, // Preserve gateway from RA
        dns: reply.dns_servers.clone(),
    };
    
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
        "DHCPv6: renewal complete, valid for {}s",
        valid_lifetime
    );
    
    Ok((ipv6_cfg, lease))
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    // ========================================================================
    // DUID Generation Tests
    // ========================================================================

    #[test]
    fn test_generate_duid_ll_format() {
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let duid = generate_duid_ll(&mac);
        
        // DUID-LL should be 10 bytes: type(2) + hw_type(2) + mac(6)
        assert_eq!(duid.len(), 10);
        
        // Type should be 3 (DUID-LL)
        assert_eq!(duid[0], 0x00);
        assert_eq!(duid[1], 0x03);
        
        // Hardware type should be 1 (Ethernet)
        assert_eq!(duid[2], 0x00);
        assert_eq!(duid[3], 0x01);
        
        // MAC address should follow
        assert_eq!(&duid[4..10], &mac);
    }

    #[test]
    fn test_generate_duid_ll_different_macs() {
        let mac1 = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let mac2 = [0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa];
        
        let duid1 = generate_duid_ll(&mac1);
        let duid2 = generate_duid_ll(&mac2);
        
        // Different MACs should produce different DUIDs
        assert_ne!(duid1, duid2);
        
        // But same structure
        assert_eq!(duid1[0..4], duid2[0..4]); // Same type and hw_type
    }

    // ========================================================================
    // IAID Generation Tests
    // ========================================================================

    #[test]
    fn test_generate_iaid_consistency() {
        // Same interface name should always produce same IAID
        let iaid1 = generate_iaid("eth0");
        let iaid2 = generate_iaid("eth0");
        assert_eq!(iaid1, iaid2);
    }

    #[test]
    fn test_generate_iaid_different_interfaces() {
        let iaid_eth0 = generate_iaid("eth0");
        let iaid_eth1 = generate_iaid("eth1");
        let iaid_enp0s3 = generate_iaid("enp0s3");
        
        // Different interfaces should produce different IAIDs
        assert_ne!(iaid_eth0, iaid_eth1);
        assert_ne!(iaid_eth0, iaid_enp0s3);
        assert_ne!(iaid_eth1, iaid_enp0s3);
    }

    // ========================================================================
    // SOLICIT Message Tests
    // ========================================================================

    #[test]
    fn test_build_solicit_message_structure() {
        let mac = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let duid = generate_duid_ll(&mac);
        let iaid = 0x12345678u32;
        let xid = 0xABCDEFu32;
        
        let msg = build_solicit_message(xid, &duid, iaid).unwrap();
        
        // Message type should be SOLICIT (1)
        assert_eq!(msg[0], 0x01);
        
        // Transaction ID (24 bits)
        assert_eq!(msg[1], 0xAB);
        assert_eq!(msg[2], 0xCD);
        assert_eq!(msg[3], 0xEF);
    }

    #[test]
    fn test_build_solicit_contains_client_id() {
        let mac = [0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe];
        let duid = generate_duid_ll(&mac);
        let msg = build_solicit_message(0x123456, &duid, 1).unwrap();
        
        // Find Client ID option (option code 1)
        let mut found_client_id = false;
        let mut pos = 4; // Skip header
        
        while pos + 4 <= msg.len() {
            let opt_code = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
            let opt_len = u16::from_be_bytes([msg[pos + 2], msg[pos + 3]]) as usize;
            
            if opt_code == 1 {
                found_client_id = true;
                // Option should contain our DUID
                assert_eq!(opt_len, duid.len());
                assert_eq!(&msg[pos + 4..pos + 4 + opt_len], &duid[..]);
                break;
            }
            pos += 4 + opt_len;
        }
        
        assert!(found_client_id, "SOLICIT must contain Client ID option");
    }

    #[test]
    fn test_build_solicit_contains_ia_na() {
        let mac = [0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        let duid = generate_duid_ll(&mac);
        let iaid = 0xDEADBEEFu32;
        let msg = build_solicit_message(0x000001, &duid, iaid).unwrap();
        
        // Find IA_NA option (option code 3)
        let mut found_ia_na = false;
        let mut pos = 4;
        
        while pos + 4 <= msg.len() {
            let opt_code = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
            let opt_len = u16::from_be_bytes([msg[pos + 2], msg[pos + 3]]) as usize;
            
            if opt_code == 3 {
                found_ia_na = true;
                // IA_NA should be at least 12 bytes (IAID + T1 + T2)
                assert!(opt_len >= 12);
                
                // Check IAID
                let parsed_iaid = u32::from_be_bytes([
                    msg[pos + 4], msg[pos + 5], msg[pos + 6], msg[pos + 7]
                ]);
                assert_eq!(parsed_iaid, iaid);
                break;
            }
            pos += 4 + opt_len;
        }
        
        assert!(found_ia_na, "SOLICIT must contain IA_NA option");
    }

    #[test]
    fn test_build_solicit_contains_option_request() {
        let mac = [0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        let duid = generate_duid_ll(&mac);
        let msg = build_solicit_message(0x000001, &duid, 1).unwrap();
        
        // Find Option Request option (option code 6)
        let mut found_oro = false;
        let mut pos = 4;
        
        while pos + 4 <= msg.len() {
            let opt_code = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
            let opt_len = u16::from_be_bytes([msg[pos + 2], msg[pos + 3]]) as usize;
            
            if opt_code == 6 {
                found_oro = true;
                // Should request DNS (option 23)
                let requested = u16::from_be_bytes([msg[pos + 4], msg[pos + 5]]);
                assert_eq!(requested, 23, "Should request DNS servers (option 23)");
                break;
            }
            pos += 4 + opt_len;
        }
        
        assert!(found_oro, "SOLICIT must contain Option Request");
    }

    // ========================================================================
    // REQUEST Message Tests
    // ========================================================================

    #[test]
    fn test_build_request_message_type() {
        let client_duid = generate_duid_ll(&[0x00; 6]);
        let server_duid = vec![0x00, 0x01, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78];
        let addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        
        let msg = build_request_message(
            0xABCDEF, &client_duid, &server_duid, 1, addr, 3600, 7200
        ).unwrap();
        
        // Message type should be REQUEST (3)
        assert_eq!(msg[0], 0x03);
    }

    #[test]
    fn test_build_request_contains_server_id() {
        let client_duid = generate_duid_ll(&[0x11; 6]);
        let server_duid = vec![0x00, 0x02, 0xaa, 0xbb, 0xcc, 0xdd];
        let addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 0x100);
        
        let msg = build_request_message(
            0x123456, &client_duid, &server_duid, 42, addr, 1800, 3600
        ).unwrap();
        
        // Find Server ID option (option code 2)
        let mut found_server_id = false;
        let mut pos = 4;
        
        while pos + 4 <= msg.len() {
            let opt_code = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
            let opt_len = u16::from_be_bytes([msg[pos + 2], msg[pos + 3]]) as usize;
            
            if opt_code == 2 {
                found_server_id = true;
                assert_eq!(&msg[pos + 4..pos + 4 + opt_len], &server_duid[..]);
                break;
            }
            pos += 4 + opt_len;
        }
        
        assert!(found_server_id, "REQUEST must contain Server ID option");
    }

    #[test]
    fn test_build_request_contains_ia_address() {
        let client_duid = generate_duid_ll(&[0x22; 6]);
        let server_duid = vec![0x00, 0x01, 0x00, 0x01];
        let addr = Ipv6Addr::new(0x2001, 0xdb8, 0x1234, 0, 0, 0, 0, 0x42);
        let preferred = 1800u32;
        let valid = 3600u32;
        
        let msg = build_request_message(
            0x999999, &client_duid, &server_duid, 100, addr, preferred, valid
        ).unwrap();
        
        // The message should contain the address bytes somewhere
        let addr_bytes = addr.octets();
        let msg_str = msg.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        let addr_str = addr_bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        
        assert!(msg_str.contains(&addr_str), "REQUEST must contain the requested address");
    }

    // ========================================================================
    // Response Parsing Tests
    // ========================================================================

    #[test]
    fn test_parse_response_too_short() {
        let short_msg = vec![0x02, 0x00, 0x01]; // Only 3 bytes
        let result = parse_response(&short_msg);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_response_advertise() {
        // Build a minimal ADVERTISE response
        let mut msg = vec![
            0x02,                   // Message type: ADVERTISE
            0x12, 0x34, 0x56,       // Transaction ID
        ];
        
        // Add Server ID option (option 2)
        let server_duid = vec![0x00, 0x01, 0x00, 0x01, 0xaa, 0xbb, 0xcc, 0xdd];
        msg.extend(&[0x00, 0x02]); // Option code
        msg.extend(&(server_duid.len() as u16).to_be_bytes());
        msg.extend(&server_duid);
        
        let response = parse_response(&msg).unwrap();
        
        assert_eq!(response.msg_type, 2); // ADVERTISE
        assert_eq!(response.xid, 0x123456);
        assert_eq!(response.server_duid, Some(server_duid));
    }

    #[test]
    fn test_parse_response_with_ia_na_and_address() {
        let mut msg = vec![
            0x07,                   // Message type: REPLY
            0xAB, 0xCD, 0xEF,       // Transaction ID
        ];
        
        // Build IA_NA option with IA Address sub-option
        let addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x1);
        let preferred: u32 = 3600;
        let valid: u32 = 7200;
        let t1: u32 = 1800;
        let t2: u32 = 2700;
        
        // IA Address sub-option (option 5)
        let mut ia_addr = vec![0x00, 0x05]; // Sub-option code
        ia_addr.extend(&24u16.to_be_bytes()); // Length: 16 (addr) + 4 + 4
        ia_addr.extend(&addr.octets());
        ia_addr.extend(&preferred.to_be_bytes());
        ia_addr.extend(&valid.to_be_bytes());
        
        // IA_NA option (option 3)
        let iaid: u32 = 0x12345678;
        let ia_na_len = 12 + ia_addr.len(); // IAID(4) + T1(4) + T2(4) + sub-options
        
        msg.extend(&[0x00, 0x03]); // Option code 3
        msg.extend(&(ia_na_len as u16).to_be_bytes());
        msg.extend(&iaid.to_be_bytes());
        msg.extend(&t1.to_be_bytes());
        msg.extend(&t2.to_be_bytes());
        msg.extend(&ia_addr);
        
        let response = parse_response(&msg).unwrap();
        
        assert_eq!(response.msg_type, 7); // REPLY
        assert_eq!(response.xid, 0xABCDEF);
        assert_eq!(response.address, Some(addr));
        assert_eq!(response.preferred_lifetime, preferred);
        assert_eq!(response.valid_lifetime, valid);
        assert_eq!(response.t1, t1);
        assert_eq!(response.t2, t2);
    }

    #[test]
    fn test_parse_response_with_dns_servers() {
        let mut msg = vec![
            0x07,                   // Message type: REPLY
            0x00, 0x00, 0x01,       // Transaction ID
        ];
        
        // Add DNS servers option (option 23)
        let dns1 = Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888);
        let dns2 = Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8844);
        
        msg.extend(&[0x00, 0x17]); // Option code 23
        msg.extend(&32u16.to_be_bytes()); // Length: 2 addresses * 16 bytes
        msg.extend(&dns1.octets());
        msg.extend(&dns2.octets());
        
        let response = parse_response(&msg).unwrap();
        
        assert_eq!(response.dns_servers.len(), 2);
        assert_eq!(response.dns_servers[0], dns1);
        assert_eq!(response.dns_servers[1], dns2);
    }

    #[test]
    fn test_parse_response_ignores_unknown_options() {
        let mut msg = vec![
            0x02,                   // ADVERTISE
            0x11, 0x22, 0x33,       // XID
        ];
        
        // Add unknown option (option 999)
        msg.extend(&[0x03, 0xE7]); // Option code 999
        msg.extend(&[0x00, 0x04]); // Length 4
        msg.extend(&[0xDE, 0xAD, 0xBE, 0xEF]); // Random data
        
        // Should parse without error
        let response = parse_response(&msg).unwrap();
        assert_eq!(response.msg_type, 2);
        assert_eq!(response.xid, 0x112233);
    }

    // ========================================================================
    // Transaction ID Tests
    // ========================================================================

    #[test]
    fn test_xid_is_24_bits() {
        // DHCPv6 uses 24-bit transaction IDs, not 32-bit
        let mac = [0x00; 6];
        let duid = generate_duid_ll(&mac);
        
        // Try with a 32-bit value that exceeds 24 bits
        let xid = 0xFFFFFFFFu32;
        let msg = build_solicit_message(xid, &duid, 1).unwrap();
        
        // Only lower 24 bits should be in the message
        // The mask happens at usage time, but message should be valid
        assert_eq!(msg[1], 0xFF);
        assert_eq!(msg[2], 0xFF);
        assert_eq!(msg[3], 0xFF);
    }

    // ========================================================================
    // Edge Cases
    // ========================================================================

    #[test]
    fn test_parse_empty_ia_na() {
        let mut msg = vec![
            0x07, 0x00, 0x00, 0x01, // REPLY with XID
        ];
        
        // IA_NA with no sub-options (just IAID + T1 + T2)
        msg.extend(&[0x00, 0x03]); // Option 3
        msg.extend(&[0x00, 0x0C]); // Length 12
        msg.extend(&[0x00, 0x00, 0x00, 0x01]); // IAID
        msg.extend(&[0x00, 0x00, 0x00, 0x00]); // T1
        msg.extend(&[0x00, 0x00, 0x00, 0x00]); // T2
        
        let response = parse_response(&msg).unwrap();
        
        // Should parse but have no address
        assert!(response.address.is_none());
    }

    #[test]
    fn test_parse_truncated_option() {
        let msg = vec![
            0x02, 0x00, 0x00, 0x01, // ADVERTISE
            0x00, 0x02,             // Server ID option
            0x00, 0x10,             // Claims length 16
            0xAA, 0xBB,             // But only 2 bytes of data
        ];
        
        // Should not crash, just stop parsing
        let response = parse_response(&msg).unwrap();
        assert_eq!(response.msg_type, 2);
        // Server DUID should not be set due to truncation
        assert!(response.server_duid.is_none());
    }
}
