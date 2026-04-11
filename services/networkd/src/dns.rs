//! DNS resolver configuration

use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;

const RESOLV_CONF_PATH: &str = "/run/resolv.conf";

pub fn configure_dns(nameservers: &[Ipv4Addr]) -> Result<()> {
    if nameservers.is_empty() {
        println!("No DNS servers to configure");
        return Ok(());
    }

    println!("Configuring DNS with {} nameserver(s)", nameservers.len());

    let mut content = String::new();
    for ns in nameservers {
        content.push_str(&format!("nameserver {}\n", ns));
        println!("Adding nameserver: {}", ns);
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;
    println!("DNS configuration written to {}", RESOLV_CONF_PATH);

    Ok(())
}

pub fn configure_dns_v6(nameservers: &[Ipv6Addr]) -> Result<()> {
    if nameservers.is_empty() {
        println!("No IPv6 DNS servers to configure");
        return Ok(());
    }

    println!(
        "Configuring IPv6 DNS with {} nameserver(s)",
        nameservers.len()
    );

    let existing = fs::read_to_string(RESOLV_CONF_PATH).unwrap_or_default();

    let mut content = existing;
    for ns in nameservers {
        let entry = format!("nameserver {}\n", ns);
        if !content.contains(&entry) {
            content.push_str(&entry);
            println!("Adding IPv6 nameserver: {}", ns);
        }
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;
    println!("IPv6 DNS configuration written to {}", RESOLV_CONF_PATH);

    Ok(())
}
