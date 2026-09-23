//! System state collection from /proc and /sys.

mod netinfo;
mod proc;

use std::time::SystemTime;

use netinfo::read_interfaces;
use proc::{
    read_cpu, read_dns, read_hostname, read_memory, read_ntp_server, read_secure_boot,
    read_system_status, read_uptime,
};

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

/// Holds previous poll data and reusable read buffers so periodic collection
/// allocates nothing beyond the returned snapshot.
#[derive(Debug, Default)]
pub struct PollState {
    prev_cpu: CpuTicks,
    scratch: String,
    net_scratch: String,
    config_stamp: Option<SystemTime>,
    ntp_cache: Option<String>,
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
    let hostname = read_hostname(&mut poll.scratch);
    let uptime = read_uptime(&mut poll.scratch);
    let cpu = read_cpu(&mut poll.scratch, &mut poll.prev_cpu);
    let memory = read_memory(&mut poll.scratch);
    let ntp_server = read_ntp_server(poll);
    let interfaces = read_interfaces(&mut poll.scratch, &mut poll.net_scratch);
    let gateway = netinfo::read_gateway(&mut poll.scratch);
    let dns_servers = read_dns(&mut poll.scratch);

    SystemState {
        hostname,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        uptime,
        cpu,
        memory,
        system_status: read_system_status(),
        secure_boot: read_secure_boot(),
        ntp_server,
        interfaces,
        gateway,
        dns_servers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_info_percent_zero_total() {
        // ARRANGE
        let memory = MemoryInfo {
            total_kb: 0,
            used_kb: 0,
        };

        // ACT / ASSERT
        assert!((memory.percent() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn memory_info_percent_half() {
        // ARRANGE
        let memory = MemoryInfo {
            total_kb: 1000,
            used_kb: 500,
        };

        // ACT / ASSERT
        assert!((memory.percent() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn uptime_default_is_zeroed() {
        // ACT
        let uptime = Uptime::default();

        // ASSERT
        assert_eq!((uptime.days, uptime.hours, uptime.minutes), (0, 0, 0));
    }

    #[test]
    fn cpu_ticks_default_is_zeroed() {
        // ACT
        let ticks = CpuTicks::default();

        // ASSERT
        assert_eq!((ticks.idle, ticks.total), (0, 0));
    }

    #[test]
    fn poll_state_default_is_empty() {
        // ACT
        let poll = PollState::default();

        // ASSERT
        assert_eq!(poll.prev_cpu.idle, 0);
        assert!(poll.scratch.is_empty());
        assert!(poll.net_scratch.is_empty());
        assert!(poll.config_stamp.is_none());
        assert!(poll.ntp_cache.is_none());
    }
}
