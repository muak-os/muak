use crate::log;
use ipnetwork::IpNetwork;
use rustables::{
    Batch, Chain, ChainPolicy, ChainType, Hook, HookClass, MsgType, ProtocolFamily, Rule, Table,
};

const NAT_TABLE_NAME: &str = "nat";
const FILTER_TABLE_NAME: &str = "filter";

pub async fn setup_nat(
    wan_interface: &str,
    bridge_interface: &str,
    bridge_subnet: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("nat", "Setting up NAT rules with nftables");

    let network: IpNetwork = bridge_subnet.parse()?;
    log!("nat", "Parsed subnet: {}", network);

    // ===== NAT Table (IPv4 only for masquerade) =====
    log!("nat", "Creating NAT table (IPv4)");
    let nat_table = Table::new(ProtocolFamily::Ipv4).with_name(NAT_TABLE_NAME);

    let mut table_batch = Batch::new();
    table_batch.add(&nat_table, MsgType::Add);

    match table_batch.send() {
        Ok(_) => log!("nat", "✓ Created NAT table"),
        Err(e) => {
            log!("nat", "✗ Failed to create NAT table: {:?}", e);
            return Err(e.into());
        }
    }

    log!("nat", "Creating postrouting chain with NAT type");
    let postrouting_chain = Chain::new(&nat_table)
        .with_name("postrouting")
        .with_type(ChainType::Nat)
        .with_hook(Hook::new(HookClass::PostRouting, 100))
        .with_policy(ChainPolicy::Accept);

    let mut chain_batch = Batch::new();
    chain_batch.add(&postrouting_chain, MsgType::Add);

    match chain_batch.send() {
        Ok(_) => log!("nat", "✓ Created postrouting chain"),
        Err(e) => {
            log!("nat", "✗ Failed to create postrouting chain: {:?}", e);
            return Err(e.into());
        }
    }

    log!(
        "nat",
        "Adding masquerade rule for interface: {}",
        wan_interface
    );
    let mut rule_batch = Batch::new();

    Rule::new(&postrouting_chain)?
        .oiface(wan_interface)?
        .masquerade()
        .add_to_batch(&mut rule_batch);

    match rule_batch.send() {
        Ok(_) => log!("nat", "✓ Added masquerade rule"),
        Err(e) => {
            log!("nat", "✗ Failed to add masquerade rule: {:?}", e);
            return Err(e.into());
        }
    }

    // ===== Filter Table (INET for both IPv4 and IPv6) =====
    log!("nat", "Creating filter table with input and forward chains");
    let filter_table = Table::new(ProtocolFamily::Inet).with_name(FILTER_TABLE_NAME);

    let mut filter_batch = Batch::new();
    filter_batch.add(&filter_table, MsgType::Add);

    // Create INPUT chain to accept all local traffic (needed for gRPC, IPC, etc.)
    let input_chain = Chain::new(&filter_table)
        .with_name("input")
        .with_type(ChainType::Filter)
        .with_hook(Hook::new(HookClass::In, 0))
        .with_policy(ChainPolicy::Accept);

    filter_batch.add(&input_chain, MsgType::Add);

    let forward_chain = Chain::new(&filter_table)
        .with_name("forward")
        .with_type(ChainType::Filter)
        .with_hook(Hook::new(HookClass::Forward, 0))
        .with_policy(ChainPolicy::Drop);

    filter_batch.add(&forward_chain, MsgType::Add);

    match filter_batch.send() {
        Ok(_) => log!("nat", "Created filter table with input and forward chains"),
        Err(e) => {
            log!("nat", "Failed to create filter table/chains: {:?}", e);
            return Err(e.into());
        }
    }

    // Add filter rules
    log!("nat", "Adding filter rules");
    let mut fwd_rule_batch = Batch::new();

    // Accept established/related connections
    Rule::new(&forward_chain)?
        .established()?
        .accept()
        .add_to_batch(&mut fwd_rule_batch);

    // Accept traffic from bridge subnet
    Rule::new(&forward_chain)?
        .snetwork(network)?
        .accept()
        .add_to_batch(&mut fwd_rule_batch);

    match fwd_rule_batch.send() {
        Ok(_) => {
            log!(
                "nat",
                "✓ NAT configured: {} (subnet {}) -> {}",
                bridge_interface,
                bridge_subnet,
                wan_interface
            );
            Ok(())
        }
        Err(e) => {
            log!("nat", "Failed to add filter rules: {:?}", e);
            Err(e.into())
        }
    }
}

pub async fn teardown_nat() -> Result<(), Box<dyn std::error::Error>> {
    log!("nat", "Tearing down NAT rules");

    let mut batch = Batch::new();

    // Delete NAT table (IPv4)
    let nat_table = Table::new(ProtocolFamily::Ipv4).with_name(NAT_TABLE_NAME);
    batch.add(&nat_table, MsgType::Del);

    // Delete filter table
    let filter_table = Table::new(ProtocolFamily::Inet).with_name(FILTER_TABLE_NAME);
    batch.add(&filter_table, MsgType::Del);

    // Ignore errors when deleting (tables might not exist)
    let _ = batch.send();

    log!("nat", "NAT teardown complete");

    Ok(())
}

pub async fn enable_ip_forwarding() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write("/proc/sys/net/ipv4/ip_forward", "1")?;
    log!("nat", "IP forwarding enabled");
    Ok(())
}
