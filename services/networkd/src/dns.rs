//! DNS resolver configuration via atomic writes to resolv.conf.

use std::fmt::Write;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;

const RESOLV_CONF_PATH: &str = "/run/resolv.conf";

/// Atomically writes all known nameservers (v4 and v6) to resolv.conf.
pub fn write_resolv_conf(v4: &[Ipv4Addr], v6: &[Ipv6Addr]) -> Result<()> {
    if v4.is_empty() && v6.is_empty() {
        println!("No DNS servers to configure");
        return Ok(());
    }

    let mut content = String::new();
    for ns in v4 {
        let _ = writeln!(content, "nameserver {}", ns);
        println!("DNS: nameserver {}", ns);
    }
    for ns in v6 {
        let _ = writeln!(content, "nameserver {}", ns);
        println!("DNS: nameserver {}", ns);
    }

    let tmp_path = format!("{}.tmp", RESOLV_CONF_PATH);
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, RESOLV_CONF_PATH)?;

    println!(
        "DNS configuration written to {} ({} v4, {} v6)",
        RESOLV_CONF_PATH,
        v4.len(),
        v6.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_resolv_conf_skips_when_both_empty() {
        // ACT
        let result = write_resolv_conf(&[], &[]);

        // ASSERT
        assert!(result.is_ok());
    }
}
