//! Network state parsing from `/proc/net` and sysfs.

use core::net::Ipv6Addr;
use std::fs;
use std::path::Path;

use super::NetInterface;
use super::proc::read_file;

/// Reads interface, gateway, and DNS state, reusing the two scratch buffers.
pub(super) fn read_interfaces(main: &mut String, net: &mut String) -> Vec<NetInterface> {
    let fib_trie = read_file("/proc/net/fib_trie", net);
    let if_inet6 = read_file("/proc/net/if_inet6", main);

    let Ok(entries) = fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };

    let mut ifaces: Vec<NetInterface> = entries
        .filter_map(core::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "lo" || interface_type(&entry.path()) != 1 {
                return None;
            }

            let mut addresses = Vec::new();
            if let Some(fib) = fib_trie {
                addresses.extend(parse_fib_trie_for_iface(fib, &name));
            }
            if let Some(inet6) = if_inet6 {
                addresses.extend(parse_if_inet6_for_iface(inet6, &name));
            }

            Some(NetInterface { name, addresses })
        })
        .collect();

    ifaces.sort_by(|left, right| left.name.cmp(&right.name));

    ifaces
}

pub(super) fn read_gateway(buf: &mut String) -> Option<String> {
    let content = read_file("/proc/net/route", buf)?;

    content.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 3 && fields.get(1).copied() == Some("00000000") {
            fields.get(2).copied().and_then(parse_hex_gateway)
        } else {
            None
        }
    })
}

fn interface_type(sysfs_dir: &Path) -> u32 {
    fs::read_to_string(sysfs_dir.join("type"))
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn parse_if_inet6_for_iface(content: &str, iface: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_ascii_whitespace().collect();
            if fields.len() >= 6 && fields.get(5).copied() == Some(iface) {
                fields.first().copied().and_then(parse_ipv6_hex)
            } else {
                None
            }
        })
        .collect()
}

fn parse_fib_trie_for_iface(content: &str, target_iface: &str) -> Vec<String> {
    let mut addrs = Vec::new();
    let mut in_local_table = false;
    let mut current_prefix: Option<&str> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("Local:") {
            in_local_table = true;
            continue;
        }
        if trimmed.starts_with("Main:") {
            in_local_table = false;
            continue;
        }

        if !in_local_table {
            continue;
        }

        if trimmed.starts_with("|-- ") || trimmed.starts_with("+-- ") {
            current_prefix = trimmed.get(4..);
        }
        if trimmed.starts_with('/')
            && let Some(prefix) = current_prefix
            && let Some(rest) = trimmed.strip_prefix("/32 host LOCAL")
            && (rest.trim().is_empty() || trimmed.contains(target_iface))
            && !prefix.starts_with("127.")
        {
            addrs.push(prefix.to_owned());
        }
    }

    addrs
}

fn parse_ipv6_hex(hex: &str) -> Option<String> {
    if hex.len() != 32 {
        return None;
    }

    let mut octets = [0_u8; 16];
    for (index, group) in hex.as_bytes().as_chunks::<4>().0.iter().enumerate() {
        let value = u16::from_str_radix(core::str::from_utf8(group).ok()?, 16).ok()?;
        let slot = octets
            .get_mut(index.saturating_mul(2)..)?
            .first_chunk_mut::<2>()?;
        slot.copy_from_slice(&value.to_be_bytes());
    }

    Some(Ipv6Addr::from(octets).to_string())
}

fn parse_hex_gateway(hex: &str) -> Option<String> {
    let val = u32::from_str_radix(hex, 16).ok()?;
    Some(format!(
        "{}.{}.{}.{}",
        val & 0xFF,
        (val >> 8) & 0xFF,
        (val >> 16) & 0xFF,
        (val >> 24) & 0xFF,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_gateway_valid() {
        // ARRANGE
        let hex = "0101A8C0";

        // ACT / ASSERT
        assert_eq!(parse_hex_gateway(hex), Some("192.168.1.1".to_owned()));
        assert_eq!(parse_hex_gateway("ZZZZ"), None);
        assert_eq!(parse_hex_gateway(""), None);
    }

    #[test]
    fn parse_ipv6_hex_compresses_canonically() {
        // ARRANGE
        let hex = "00000000000000000000000000000001";

        // ACT / ASSERT
        assert_eq!(parse_ipv6_hex(hex), Some("::1".to_owned()));
    }

    #[test]
    fn parse_ipv6_hex_keeps_groups() {
        // ARRANGE
        let hex = "20010DB8000000000000000000000001";

        // ACT / ASSERT
        assert_eq!(parse_ipv6_hex(hex), Some("2001:db8::1".to_owned()));
    }

    #[test]
    fn parse_ipv6_hex_rejects_bad_input() {
        // ACT / ASSERT
        assert_eq!(parse_ipv6_hex("0000"), None);
        assert_eq!(parse_ipv6_hex(""), None);
        assert_eq!(parse_ipv6_hex("ZZZZ0000000000000000000000000000"), None);
    }

    #[test]
    fn fib_trie_no_local_table() {
        // ARRANGE
        let content = "Main:\n  +-- 0.0.0.0/0\n";

        // ACT
        let addrs = parse_fib_trie_for_iface(content, "eth0");

        // ASSERT
        assert!(addrs.is_empty());
    }

    #[test]
    fn fib_trie_excludes_loopback() {
        // ARRANGE
        let content = "Local:\n  +-- 127.0.0.1/8\n       /32 host LOCAL\n";

        // ACT / ASSERT
        assert!(parse_fib_trie_for_iface(content, "eth0").is_empty());
    }
}
