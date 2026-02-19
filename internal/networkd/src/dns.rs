use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;

use crate::config::RESOLV_CONF_PATH;

pub fn configure_dns(nameservers: &[Ipv4Addr]) -> Result<()> {
    if nameservers.is_empty() {
        kmsg::info!(@ "networkd", "No DNS servers to configure");
        return Ok(());
    }

    kmsg::info!("Configuring DNS with {} nameserver(s)", nameservers.len());

    let mut content = String::new();
    for ns in nameservers {
        content.push_str(&format!("nameserver {}\n", ns));
        kmsg::debug!("Adding nameserver: {}", ns);
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;
    kmsg::debug!("DNS configuration written to {}", RESOLV_CONF_PATH);

    Ok(())
}

pub fn configure_dns_v6(nameservers: &[Ipv6Addr]) -> Result<()> {
    if nameservers.is_empty() {
        kmsg::info!("No IPv6 DNS servers to configure");
        return Ok(());
    }

    kmsg::info!(
        "Configuring IPv6 DNS with {} nameserver(s)",
        nameservers.len()
    );

    let existing = fs::read_to_string(RESOLV_CONF_PATH).unwrap_or_default();

    let mut content = existing;
    for ns in nameservers {
        let entry = format!("nameserver {}\n", ns);
        if !content.contains(&entry) {
            content.push_str(&entry);
            kmsg::debug!("Adding IPv6 nameserver: {}", ns);
        }
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;
    kmsg::debug!("IPv6 DNS configuration written to {}", RESOLV_CONF_PATH);

    Ok(())
}
