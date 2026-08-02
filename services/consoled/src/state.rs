//! System state collection from /proc and /sys.

extern crate alloc;

use alloc::collections::BTreeMap;
use std::fs;
use std::path::Path;

const CONFIG_PATH: &str = "/run/state/config.toml";
const SECURE_BOOT_EFIVAR: &str =
    "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemStatus {
    Installed,
    Maintenance,
}

/// Snapshot of system state at a point in time.
#[derive(Debug, Clone)]
pub struct SystemState {
    pub hostname: String,
    pub version: String,
    pub uptime: Uptime,
    pub cpu: CpuUsage,
    pub memory: MemoryInfo,
    pub system_status: SystemStatus,
    pub secure_boot: bool,
    pub ntp_server: Option<String>,
    pub interfaces: Vec<NetInterface>,
    pub gateway: Option<String>,
    pub dns_servers: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Uptime {
    pub days: u64,
    pub hours: u64,
    pub minutes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CpuUsage {
    pub percent: f64,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryInfo {
    pub total_kb: u64,
    pub used_kb: u64,
}

#[derive(Debug, Clone)]
pub struct NetInterface {
    pub name: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct CpuTicks {
    idle: u64,
    total: u64,
}

/// Holds previous poll data for computing deltas.
#[derive(Debug, Default)]
pub struct PollState {
    prev_cpu: CpuTicks,
}

impl MemoryInfo {
    pub fn percent(&self) -> f64 {
        if self.total_kb == 0 {
            return 0.0;
        }
        let permille = self.used_kb.saturating_mul(1000).div_euclid(self.total_kb);
        f64::from(u32::try_from(permille).unwrap_or(0)) / 10.0
    }
}

/// Collects a full system state snapshot.
pub fn collect(poll: &mut PollState) -> SystemState {
    SystemState {
        hostname: read_hostname(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        uptime: read_uptime(),
        cpu: read_cpu(poll),
        memory: read_memory(),
        system_status: read_system_status(),
        secure_boot: read_secure_boot(),
        ntp_server: read_ntp_server(),
        interfaces: read_interfaces(),
        gateway: read_default_gateway(),
        dns_servers: read_dns(),
    }
}

fn read_hostname() -> String {
    read_trimmed("/proc/sys/kernel/hostname")
        .filter(|hostname| !hostname.is_empty() && hostname != "(none)")
        .unwrap_or_else(|| "Muak".to_owned())
}

fn read_uptime() -> Uptime {
    let Some(content) = read_trimmed("/proc/uptime") else {
        return Uptime::default();
    };
    let secs = content
        .split_ascii_whitespace()
        .next()
        .and_then(|value| value.split('.').next())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    Uptime {
        days: secs.div_euclid(86400),
        hours: secs.rem_euclid(86400).div_euclid(3600),
        minutes: secs.rem_euclid(3600).div_euclid(60),
    }
}

fn read_cpu(poll: &mut PollState) -> CpuUsage {
    let Some(content) = read_trimmed("/proc/stat") else {
        return CpuUsage::default();
    };
    let Some(cpu_line) = content.lines().find(|line| line.starts_with("cpu ")) else {
        return CpuUsage::default();
    };

    let fields: Vec<u64> = cpu_line
        .split_ascii_whitespace()
        .skip(1)
        .filter_map(|field| field.parse().ok())
        .collect();

    if fields.len() < 4 {
        return CpuUsage::default();
    }

    let idle = fields
        .get(3)
        .copied()
        .unwrap_or(0)
        .saturating_add(fields.get(4).copied().unwrap_or(0));
    let total: u64 = fields.iter().sum();

    let current = CpuTicks { idle, total };
    let delta_total = current.total.saturating_sub(poll.prev_cpu.total);
    let delta_idle = current.idle.saturating_sub(poll.prev_cpu.idle);

    let percent = if delta_total > 0 {
        let permille = delta_total
            .saturating_sub(delta_idle)
            .saturating_mul(1000)
            .div_euclid(delta_total);
        f64::from(u32::try_from(permille).unwrap_or(0)) / 10.0
    } else {
        0.0
    };

    poll.prev_cpu = current;

    CpuUsage { percent }
}

fn read_memory() -> MemoryInfo {
    let Some(content) = read_trimmed("/proc/meminfo") else {
        return MemoryInfo::default();
    };

    let fields: BTreeMap<&str, u64> = content
        .lines()
        .filter_map(|line| {
            let mut parts = line.split(':');
            let key = parts.next()?.trim();
            let val = parts
                .next()?
                .trim()
                .split_ascii_whitespace()
                .next()?
                .parse()
                .ok()?;
            Some((key, val))
        })
        .collect();

    let total_kb = fields.get("MemTotal").copied().unwrap_or(0);
    let available_kb = fields.get("MemAvailable").copied().unwrap_or(0);

    MemoryInfo {
        total_kb,
        used_kb: total_kb.saturating_sub(available_kb),
    }
}

fn read_interfaces() -> Vec<NetInterface> {
    let net_dir = Path::new("/sys/class/net");
    let Ok(entries) = fs::read_dir(net_dir) else {
        return Vec::new();
    };

    let mut ifaces: Vec<NetInterface> = entries
        .filter_map(core::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "lo" {
                return None;
            }

            let iface_type: u32 = read_trimmed(entry.path().join("type").to_str()?)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);

            if iface_type != 1 {
                return None;
            }

            let addrs = read_interface_addresses(&name);

            Some(NetInterface {
                name,
                addresses: addrs,
            })
        })
        .collect();

    ifaces.sort_by(|left, right| left.name.cmp(&right.name));
    ifaces
}

fn read_interface_addresses(iface: &str) -> Vec<String> {
    let mut addrs = Vec::new();

    if let Some(content) = read_trimmed("/proc/net/fib_trie") {
        addrs.extend(parse_fib_trie_for_iface(&content, iface));
    }

    if let Some(content) = read_trimmed("/proc/net/if_inet6") {
        addrs.extend(parse_if_inet6_for_iface(&content, iface));
    }

    addrs
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

    let chunks: Vec<&str> = (0_usize..8)
        .map(|i| {
            let start = i.wrapping_mul(4);
            let end = i.saturating_add(1).wrapping_mul(4);
            hex.get(start..end).unwrap_or_default()
        })
        .collect();
    let full = chunks.join(":");

    Some(
        full.replace(":0000", ":0")
            .trim_start_matches("0000:")
            .to_owned(),
    )
}

fn read_default_gateway() -> Option<String> {
    let content = read_trimmed("/proc/net/route")?;

    content.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 3 && fields.get(1).copied() == Some("00000000") {
            fields.get(2).copied().and_then(parse_hex_gateway)
        } else {
            None
        }
    })
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

fn read_dns() -> Vec<String> {
    let Some(content) = read_trimmed("/etc/resolv.conf") else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("nameserver ")
                .map(|addr| addr.trim().to_owned())
        })
        .collect()
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|content| content.trim().to_owned())
}

fn read_system_status() -> SystemStatus {
    if Path::new(CONFIG_PATH).exists() {
        SystemStatus::Installed
    } else {
        SystemStatus::Maintenance
    }
}

fn read_secure_boot() -> bool {
    fs::read(SECURE_BOOT_EFIVAR)
        .ok()
        .and_then(|bytes| bytes.get(4).copied())
        .is_some_and(|byte| byte == 1)
}

fn read_ntp_server() -> Option<String> {
    config::load_from_path(Path::new(CONFIG_PATH))
        .ok()
        .map(|cfg| cfg.host.ntp)
        .filter(|server| !server.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_gateway_valid() {
        // ARRANGE
        let hex = "0101A8C0";

        // ACT
        let result = parse_hex_gateway(hex);

        // ASSERT
        assert_eq!(result, Some("192.168.1.1".to_owned()));
    }

    #[test]
    fn parse_hex_gateway_invalid() {
        assert!(parse_hex_gateway("ZZZZ").is_none());
        assert!(parse_hex_gateway("").is_none());
    }

    #[test]
    fn parse_ipv6_hex_valid() {
        // ARRANGE
        let hex = "00000000000000000000000000000001";

        // ACT
        let result = parse_ipv6_hex(hex);

        // ASSERT
        assert!(result.is_some());
    }

    #[test]
    fn parse_ipv6_hex_invalid_length() {
        assert!(parse_ipv6_hex("0000").is_none());
        assert!(parse_ipv6_hex("").is_none());
    }

    #[test]
    fn uptime_default() {
        let uptime = Uptime::default();
        assert_eq!(uptime.days, 0);
        assert_eq!(uptime.hours, 0);
        assert_eq!(uptime.minutes, 0);
    }

    #[test]
    fn memory_info_percent_zero_total() {
        let memory = MemoryInfo {
            total_kb: 0,
            used_kb: 0,
        };
        assert!((memory.percent() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn memory_info_percent_half() {
        let memory = MemoryInfo {
            total_kb: 1000,
            used_kb: 500,
        };
        assert!((memory.percent() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cpu_ticks_default() {
        let ticks = CpuTicks::default();
        assert_eq!(ticks.idle, 0);
        assert_eq!(ticks.total, 0);
    }

    #[test]
    fn poll_state_default() {
        let poll = PollState::default();
        assert_eq!(poll.prev_cpu.idle, 0);
        assert_eq!(poll.prev_cpu.total, 0);
    }

    #[test]
    fn fib_trie_no_local_table() {
        let content = "Main:\n  +-- 0.0.0.0/0\n";
        let addrs = parse_fib_trie_for_iface(content, "eth0");
        assert!(addrs.is_empty());
    }
}
