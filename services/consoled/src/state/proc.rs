//! `/proc`, `/sys`, and config file readers for the status panel.

use std::fs;
use std::path::Path;

use super::{CpuTicks, CpuUsage, MemoryInfo, PollState, SystemStatus, Uptime};

const CONFIG_PATH: &str = "/run/state/config.toml";
const SECURE_BOOT_EFIVAR: &str =
    "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

/// Reads a whole file into `buf` and returns its trimmed content.
pub(super) fn read_file<'a>(path: &str, buf: &'a mut String) -> Option<&'a str> {
    buf.clear();
    let mut file = fs::File::open(path).ok()?;
    std::io::Read::read_to_string(&mut file, buf).ok()?;

    Some(buf.trim())
}

pub(super) fn read_hostname(buf: &mut String) -> String {
    read_file("/proc/sys/kernel/hostname", buf)
        .filter(|&hostname| !hostname.is_empty() && hostname != "(none)")
        .unwrap_or("Muak")
        .to_owned()
}

pub(super) fn read_uptime(buf: &mut String) -> Uptime {
    let Some(content) = read_file("/proc/uptime", buf) else {
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

pub(super) fn read_cpu(buf: &mut String, prev: &mut CpuTicks) -> CpuUsage {
    let Some(content) = read_file("/proc/stat", buf) else {
        return CpuUsage::default();
    };
    let Some(cpu_line) = content.lines().find(|line| line.starts_with("cpu ")) else {
        return CpuUsage::default();
    };

    parse_cpu(cpu_line, prev)
}

pub(super) fn read_memory(buf: &mut String) -> MemoryInfo {
    let Some(content) = read_file("/proc/meminfo", buf) else {
        return MemoryInfo::default();
    };

    parse_memory(content)
}

pub(super) fn read_dns(buf: &mut String) -> Vec<String> {
    let Some(content) = read_file("/etc/resolv.conf", buf) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("nameserver ")
                .map(|addr| addr.trim().to_owned())
        })
        .collect()
}

pub(super) fn read_system_status() -> SystemStatus {
    if Path::new(CONFIG_PATH).exists() {
        SystemStatus::Installed
    } else {
        SystemStatus::Maintenance
    }
}

pub(super) fn read_secure_boot() -> bool {
    fs::read(SECURE_BOOT_EFIVAR)
        .ok()
        .and_then(|bytes| bytes.get(4).copied())
        .is_some_and(|byte| byte == 1)
}

/// Returns the configured NTP server, re-reading the config only when its
/// modification time changed since the last poll.
pub(super) fn read_ntp_server(poll: &mut PollState) -> Option<String> {
    let stamp = fs::metadata(CONFIG_PATH).ok()?.modified().ok()?;
    if poll.config_stamp == Some(stamp) {
        return poll.ntp_cache.clone();
    }

    let server = config::load_from_path(Path::new(CONFIG_PATH))
        .ok()
        .map(|cfg| cfg.host.ntp)
        .filter(|server| !server.is_empty());
    poll.config_stamp = Some(stamp);
    poll.ntp_cache.clone_from(&server);

    server
}

fn parse_cpu(line: &str, prev: &mut CpuTicks) -> CpuUsage {
    let mut total = 0_u64;
    let mut idle = 0_u64;
    let mut fields = 0_usize;

    for (index, field) in line.split_ascii_whitespace().skip(1).enumerate() {
        fields = fields.saturating_add(1);
        let Ok(value) = field.parse::<u64>() else {
            continue;
        };
        total = total.saturating_add(value);
        if index == 3 || index == 4 {
            idle = idle.saturating_add(value);
        }
    }

    if fields < 4 {
        return CpuUsage::default();
    }

    let delta_total = total.saturating_sub(prev.total);
    let delta_idle = idle.saturating_sub(prev.idle);
    let percent = if delta_total > 0 {
        let permille = delta_total
            .saturating_sub(delta_idle)
            .saturating_mul(1000)
            .div_euclid(delta_total);
        f64::from(u32::try_from(permille).unwrap_or(0)) / 10.0
    } else {
        0.0
    };

    *prev = CpuTicks { idle, total };

    CpuUsage { percent }
}

fn parse_memory(content: &str) -> MemoryInfo {
    let total_kb = content
        .lines()
        .find_map(|line| field_kb(line, "MemTotal:"))
        .unwrap_or(0);
    let available_kb = content
        .lines()
        .find_map(|line| field_kb(line, "MemAvailable:"))
        .unwrap_or(0);

    MemoryInfo {
        total_kb,
        used_kb: total_kb.saturating_sub(available_kb),
    }
}

fn field_kb(line: &str, key: &str) -> Option<u64> {
    line.strip_prefix(key)?
        .split_ascii_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_deltas_two_polls() {
        // ARRANGE
        let mut prev = CpuTicks::default();
        let first = "cpu  0 0 0 0 0 0 0 0 0 0";
        let second = "cpu  50 0 0 50 0 0 0 0 0 0";

        // ACT
        let initial = parse_cpu(first, &mut prev);
        let next = parse_cpu(second, &mut prev);

        // ASSERT: no delta on the first poll, 50% busy ticks on the second.
        assert!(initial.percent.abs() < f64::EPSILON);
        assert!((next.percent - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_cpu_short_line_is_default() {
        // ARRANGE
        let mut prev = CpuTicks::default();

        // ACT
        let usage = parse_cpu("cpu  1 2", &mut prev);

        // ASSERT
        assert!(usage.percent.abs() < f64::EPSILON);
    }

    #[test]
    fn parse_memory_finds_needed_fields_only() {
        // ARRANGE
        let content = "MemTotal:       16088520 kB\nSome: 1 kB\nMemFree:         123 kB\n\
                       MemAvailable:    8000000 kB\n";

        // ACT
        let memory = parse_memory(content);

        // ASSERT
        assert_eq!(memory.total_kb, 16_088_520);
        assert_eq!(memory.used_kb, 8_088_520);
    }

    #[test]
    fn parse_memory_missing_available_uses_total() {
        // ARRANGE
        let content = "MemTotal: 1000 kB\n";

        // ACT
        let memory = parse_memory(content);

        // ASSERT
        assert_eq!((memory.total_kb, memory.used_kb), (1000, 1000));
    }

    #[test]
    fn field_kb_parses_first_token() {
        // ACT / ASSERT
        assert_eq!(field_kb("MemTotal:       123 kB", "MemTotal:"), Some(123));
        assert_eq!(field_kb("Other: 5 kB", "MemTotal:"), None);
        assert_eq!(field_kb("MemTotal: junk", "MemTotal:"), None);
    }

    #[test]
    fn read_ntp_server_missing_config_is_none() {
        // ARRANGE
        let mut poll = PollState::default();

        // ACT
        let server = read_ntp_server(&mut poll);

        // ASSERT
        assert_eq!(server, None);
    }
}
