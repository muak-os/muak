use anyhow::Result;
use std::fs;
use std::net::Ipv4Addr;

use crate::config::RESOLV_CONF_PATH;

pub fn configure_dns(nameservers: &[Ipv4Addr]) -> Result<()> {
    if nameservers.is_empty() {
        kmsg::info!(@ "networkd", "No DNS servers to configure");
        return Ok(());
    }

    kmsg::info!(
        @ "networkd",
        "Configuring DNS with {} nameserver(s)",
        nameservers.len()
    );

    let mut content = String::new();
    for ns in nameservers {
        content.push_str(&format!("nameserver {}\n", ns));
        kmsg::info!(@ "networkd", "Adding nameserver: {}", ns);
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;
    kmsg::info!(
        @ "networkd",
        "DNS configuration written to {}",
        RESOLV_CONF_PATH
    );

    Ok(())
}
