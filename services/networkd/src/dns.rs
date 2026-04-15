//! DNS resolver configuration via atomic writes to resolv.conf.

use std::fmt::Write;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use anyhow::Result;

pub const RESOLV_CONF_PATH: &str = "/run/resolv.conf";

/// Tracks the current nameserver lists and the path to resolv.conf.
#[derive(Debug, Clone)]
pub struct DnsState {
    pub v4: Vec<Ipv4Addr>,
    pub v6: Vec<Ipv6Addr>,
    pub resolv_conf: PathBuf,
    tmp_path: PathBuf,
}

impl Default for DnsState {
    fn default() -> Self {
        Self::with_path(PathBuf::from(RESOLV_CONF_PATH))
    }
}

impl DnsState {
    /// Creates a `DnsState` that writes to `path` instead of the default.
    pub fn with_path(path: PathBuf) -> Self {
        let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
        Self {
            v4: Vec::new(),
            v6: Vec::new(),
            resolv_conf: path,
            tmp_path,
        }
    }

    /// Returns `true` if `v4` and `v6` are identical to the cached lists.
    pub fn is_unchanged(&self, v4: &[Ipv4Addr], v6: &[Ipv6Addr]) -> bool {
        self.v4 == v4 && self.v6 == v6
    }

    /// Replaces the cached lists and atomically flushes resolv.conf.
    pub fn update(&mut self, v4: Vec<Ipv4Addr>, v6: Vec<Ipv6Addr>) -> Result<()> {
        self.v4 = v4;
        self.v6 = v6;
        write_resolv_conf(&self.resolv_conf, &self.tmp_path, &self.v4, &self.v6)
    }
}

/// Atomically writes all known nameservers (V4 and V6).
pub fn write_resolv_conf(
    path: &Path,
    tmp_path: &Path,
    v4: &[Ipv4Addr],
    v6: &[Ipv6Addr],
) -> Result<()> {
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

    fs::write(tmp_path, &content)?;
    fs::rename(tmp_path, path)?;

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
    fn dns_state_default_is_empty() {
        // ACT
        let dns = DnsState::default();

        // ASSERT
        assert!(dns.v4.is_empty());
        assert!(dns.v6.is_empty());
        assert_eq!(dns.resolv_conf, PathBuf::from(RESOLV_CONF_PATH));
    }

    #[test]
    fn dns_state_is_unchanged_detects_no_diff() {
        // ARRANGE
        let mut dns = DnsState::default();
        dns.v4 = vec![Ipv4Addr::new(8, 8, 8, 8)];

        // ACT / ASSERT
        assert!(dns.is_unchanged(&[Ipv4Addr::new(8, 8, 8, 8)], &[]));
        assert!(!dns.is_unchanged(&[Ipv4Addr::new(1, 1, 1, 1)], &[]));
    }

    #[test]
    fn dns_state_update_writes_file_and_caches() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("resolv.conf");
        let mut dns = DnsState::with_path(path.clone());

        // ACT
        dns.update(vec![Ipv4Addr::new(8, 8, 8, 8)], vec![])
            .expect("update failed");

        // ASSERT
        assert_eq!(dns.v4, vec![Ipv4Addr::new(8, 8, 8, 8)]);
        let content = std::fs::read_to_string(&path).expect("read failed");
        assert!(content.contains("nameserver 8.8.8.8"));
    }

    #[test]
    fn write_resolv_conf_skips_when_both_empty() {
        // ACT
        let path = Path::new(RESOLV_CONF_PATH);
        let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
        let result = write_resolv_conf(path, &tmp_path, &[], &[]);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn write_resolv_conf_with_v4_servers_writes_content() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("resolv.conf");
        let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
        let v4 = vec![Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(1, 1, 1, 1)];

        // ACT
        write_resolv_conf(&path, &tmp_path, &v4, &[]).expect("write failed");

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
        let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
        let v6 = vec![
            "2001:4860:4860::8888"
                .parse::<Ipv6Addr>()
                .expect("valid addr"),
            "2606:4700:4700::1111"
                .parse::<Ipv6Addr>()
                .expect("valid addr"),
        ];

        // ACT
        write_resolv_conf(&path, &tmp_path, &[], &v6).expect("write failed");

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
        let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
        let v4 = vec![Ipv4Addr::new(8, 8, 4, 4)];
        let v6 = vec![
            "2001:4860:4860::8844"
                .parse::<Ipv6Addr>()
                .expect("valid addr"),
        ];

        // ACT
        write_resolv_conf(&path, &tmp_path, &v4, &v6).expect("write failed");

        // ASSERT
        let content = std::fs::read_to_string(&path).expect("read failed");
        assert!(content.contains("nameserver 8.8.4.4"));
        assert!(content.contains("nameserver 2001:4860:4860::8844"));
    }
}
