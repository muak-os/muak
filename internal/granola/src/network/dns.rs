use crate::log;
use std::fs;
use std::net::Ipv4Addr;

use super::config::RESOLV_CONF_PATH;

pub fn configure_dns(nameservers: &[Ipv4Addr]) -> Result<(), Box<dyn std::error::Error>> {
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

    fs::write(RESOLV_CONF_PATH, content)?;
    log!(
        "network",
        "DNS configuration written to {}",
        RESOLV_CONF_PATH
    );

    Ok(())
}
