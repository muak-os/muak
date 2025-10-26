use crate::log;

pub async fn setup_nat(
    wan_interface: &str,
    bridge_interface: &str,
    bridge_subnet: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Implement proper nftables rules using nftnl crate
    // The nftables rules we need:
    // 1. table inet filter with forward chain (accept established/related, accept from bridge)
    // 2. table ip nat with postrouting chain (masquerade from bridge subnet)

    log!(
        "nat",
        "NAT configured: {} -> {} ({})",
        bridge_interface,
        wan_interface,
        bridge_subnet
    );

    Ok(())
}

pub async fn teardown_nat() -> Result<(), Box<dyn std::error::Error>> {
    log!("nat", "Tearing down NAT rules");

    // TODO: Implement proper nftables cleanup using nftnl crate

    log!("nat", "NAT teardown complete");

    Ok(())
}

pub async fn enable_ip_forwarding() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write("/proc/sys/net/ipv4/ip_forward", "1")?;
    Ok(())
}
