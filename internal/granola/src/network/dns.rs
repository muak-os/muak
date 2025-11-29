use crate::log;
use anyhow::Result;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};

use super::config::RESOLV_CONF_PATH;

pub fn configure_dns(nameservers: &[Ipv4Addr]) -> Result<()> {
    if nameservers.is_empty() {
        log!("network", "No DNS servers to configure");
        return Ok(());
    }

    log!(
        "network",
        "Configuring DNS with {} nameserver(s)",
        nameservers.len()
    );

    let mut content = String::new();
    for ns in nameservers {
        content.push_str(&format!("nameserver {}\n", ns));
        log!("network", "Adding nameserver: {}", ns);
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;
    log!(
        "network",
        "DNS configuration written to {}",
        RESOLV_CONF_PATH
    );

    Ok(())
}

/// Configure DNS with IPv6 nameservers
pub fn configure_dns_v6(nameservers: &[Ipv6Addr]) -> Result<()> {
    if nameservers.is_empty() {
        log!("network", "No IPv6 DNS servers to configure");
        return Ok(());
    }

    log!(
        "network",
        "Configuring IPv6 DNS with {} nameserver(s)",
        nameservers.len()
    );

    // Read existing content to preserve IPv4 nameservers
    let existing = fs::read_to_string(RESOLV_CONF_PATH).unwrap_or_default();
    
    let mut content = existing;
    for ns in nameservers {
        let entry = format!("nameserver {}\n", ns);
        // Don't add duplicates
        if !content.contains(&entry) {
            content.push_str(&entry);
            log!("network", "Adding IPv6 nameserver: {}", ns);
        }
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;
    log!(
        "network",
        "IPv6 DNS configuration written to {}",
        RESOLV_CONF_PATH
    );

    Ok(())
}
