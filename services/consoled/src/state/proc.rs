//! `/proc`, `/sys`, and config file readers for the status panel.

use std::fs;
use std::path::Path;

use super::{CpuTicks, CpuUsage, MemoryInfo, PollState, SystemStatus, Uptime};

const CONFIG_PATH: &str = "/run/state/config.toml";
const PLATFORM_KEY_PATH: &str = "/run/state/secrets/secureboot/pk.der";
const FALLBACK_NAME: &str = "Muak";

/// Reads a whole file into `buf` and returns its trimmed content.
pub(super) fn read_file<'a>(path: &str, buf: &'a mut String) -> Option<&'a str> {
    buf.clear();
    let mut file = fs::File::open(path).ok()?;
    std::io::Read::read_to_string(&mut file, buf).ok()?;

    Some(buf.trim())
}

pub(super) fn read_host_name(poll: &mut PollState) -> String {
    host_name(config(poll))
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

pub(super) fn read_secure_boot(
    poll: &mut PollState,
) -> (
    firmware::secureboot::Status,
    Option<firmware::secureboot::Mismatch>,
) {
    let store = firmware::system();
    let state = firmware::secureboot::state(&store).unwrap_or_default();
    let ours = fs::read(PLATFORM_KEY_PATH).is_ok_and(|certificate| {
        firmware::secureboot::platform_key_matches(&store, &certificate).unwrap_or(false)
    });
    let mismatch = firmware::secureboot::mismatch(config_wants_secureboot(poll), &state, ours);

    (firmware::secureboot::status(&state, ours), mismatch)
}

pub(super) fn read_ntp_server(poll: &mut PollState) -> Option<String> {
    config(poll)
        .map(|cfg| cfg.host.ntp.clone())
        .filter(|server| !server.is_empty())
}

fn config_wants_secureboot(poll: &mut PollState) -> bool {
    config(poll).is_some_and(|cfg| cfg.host.secureboot)
}

fn config(poll: &mut PollState) -> Option<&config::system::Config> {
    let stamp = fs::metadata(CONFIG_PATH)
        .and_then(|meta| meta.modified())
        .ok();
    if stamp.is_some() && poll.config_stamp == stamp {
        return poll.config.as_ref();
    }

    poll.config_stamp = stamp;
    poll.config = stamp.and_then(|_| load_config(Path::new(CONFIG_PATH)));

    poll.config.as_ref()
}

fn load_config(path: &Path) -> Option<config::system::Config> {
    path.exists()
        .then(|| config::system::load_from_path(path).ok())
        .flatten()
}

fn host_name(config: Option<&config::system::Config>) -> String {
    config
        .map(|cfg| cfg.host.name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or(FALLBACK_NAME)
        .to_owned()
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
    let permille = if delta_total > 0 {
        delta_total
            .saturating_sub(delta_idle)
            .saturating_mul(1000)
            .div_euclid(delta_total)
    } else {
        0
    };

    *prev = CpuTicks { idle, total };

    CpuUsage { permille }
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
        assert_eq!(initial.permille, 0);
        assert_eq!(next.permille, 500);
    }

    #[test]
    fn parse_cpu_short_line_is_default() {
        // ARRANGE
        let mut prev = CpuTicks::default();

        // ACT
        let usage = parse_cpu("cpu  1 2", &mut prev);

        // ASSERT
        assert_eq!(usage.permille, 0);
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

    fn write_config(dir: &Path, name: &str) -> std::path::PathBuf {
        write_config_parts(dir, name, "pool.ntp.org", true)
    }

    fn write_config_parts(
        dir: &Path,
        name: &str,
        ntp: &str,
        secureboot: bool,
    ) -> std::path::PathBuf {
        let path = dir.join("config.toml");
        let contents = format!(
            "api_version = \"muak.dev/config/v1-beta\"\n[host]\nname = {name:?}\nntp = {ntp:?}\n\
             secureboot = {secureboot}\n"
        );
        fs::write(&path, contents).unwrap();

        path
    }

    #[test]
    fn host_name_uses_trimmed_config_name() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(dir.path(), "  lab-node  ");

        // ACT
        let name = host_name(load_config(&path).as_ref());

        // ASSERT
        assert_eq!(name, "lab-node");
    }

    #[test]
    fn host_name_empty_config_name_falls_back() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(dir.path(), "");

        // ACT
        let name = host_name(load_config(&path).as_ref());

        // ASSERT
        assert_eq!(name, "Muak");
    }

    #[test]
    fn host_name_blank_config_name_falls_back() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(dir.path(), "   ");

        // ACT
        let name = host_name(load_config(&path).as_ref());

        // ASSERT
        assert_eq!(name, "Muak");
    }

    #[test]
    fn missing_config_falls_back_without_builtin_default() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.toml");

        // ACT
        let config = load_config(&path);

        // ASSERT: the raw loader would yield the built-in `muak` default.
        assert!(config.is_none());
        assert_eq!(host_name(config.as_ref()), "Muak");
        assert_eq!(
            config::system::load_from_path(&path).unwrap().host.name,
            "muak"
        );
    }

    #[test]
    fn ntp_server_absent_without_config() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.toml");

        // ACT
        let server = load_config(&path)
            .map(|cfg| cfg.host.ntp)
            .filter(|server| !server.is_empty());

        // ASSERT
        assert_eq!(server, None);
    }

    #[test]
    fn secureboot_disabled_config_is_false() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let disabled = write_config_parts(dir.path(), "lab-node", "", false);

        // ACT
        let wants = load_config(&disabled).is_some_and(|cfg| cfg.host.secureboot);

        // ASSERT
        assert!(!wants);
    }
}
