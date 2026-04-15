//! DNS resolver configuration via atomic writes to resolv.conf.

use std::fmt::Write;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

use anyhow::Result;

pub const RESOLV_CONF_PATH: &str = "/run/resolv.conf";

/// Atomically writes all known nameservers (V4 and V6).
pub fn write_resolv_conf(path: &Path, v4: &[Ipv4Addr], v6: &[Ipv6Addr]) -> Result<()> {
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

    let tmp_path = format!("{}.tmp", path.display());
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, path)?;

    println!(
        "DNS configuration written to {} ({} v4, {} v6)",
        path.display(),
        v4.len(),
        v6.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::path::Path;

    use super::*;

    #[test]
    fn write_resolv_conf_skips_when_both_empty() {
        // ACT
        let result = write_resolv_conf(Path::new(RESOLV_CONF_PATH), &[], &[]);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn write_resolv_conf_with_v4_servers_writes_content() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("resolv.conf");
        let v4 = vec![Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(1, 1, 1, 1)];

        // ACT
        write_resolv_conf(&path, &v4, &[]).expect("write failed");

        // ASSERT
        let content = std::fs::read_to_string(&path).expect("read failed");
        assert!(content.contains("nameserver 8.8.8.8"));
        assert!(content.contains("nameserver 1.1.1.1"));
    }

    #[test]
    fn write_resolv_conf_with_v6_servers_writes_content() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("resolv.conf");
        let v6 = vec![
            "2001:4860:4860::8888"
                .parse::<Ipv6Addr>()
                .expect("valid addr"),
            "2606:4700:4700::1111"
                .parse::<Ipv6Addr>()
                .expect("valid addr"),
        ];

        // ACT
        write_resolv_conf(&path, &[], &v6).expect("write failed");

        // ASSERT
        let content = std::fs::read_to_string(&path).expect("read failed");
        assert!(content.contains("nameserver 2001:4860:4860::8888"));
        assert!(content.contains("nameserver 2606:4700:4700::1111"));
    }

    #[test]
    fn write_resolv_conf_with_mixed_servers_writes_content() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("resolv.conf");
        let v4 = vec![Ipv4Addr::new(8, 8, 4, 4)];
        let v6 = vec![
            "2001:4860:4860::8844"
                .parse::<Ipv6Addr>()
                .expect("valid addr"),
        ];

        // ACT
        write_resolv_conf(&path, &v4, &v6).expect("write failed");

        // ASSERT
        let content = std::fs::read_to_string(&path).expect("read failed");
        assert!(content.contains("nameserver 8.8.4.4"));
        assert!(content.contains("nameserver 2001:4860:4860::8844"));
    }
}
