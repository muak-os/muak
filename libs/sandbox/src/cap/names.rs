//! Text names for Linux capabilities.
//!
//! A configuration file names a capability in text, so the table here is the
//! single place that turns a name into a [`Cap`] and back. The names are the
//! kernel ones without the `CAP_` prefix, in lowercase, which is what a service
//! file spells. The methods that use the table live on [`Cap`] itself, because
//! an inherent impl block cannot be split across two files.

use super::Cap;

const NAMED: [(&str, Cap); 41] = [
    ("chown", Cap::Chown),
    ("dac_override", Cap::DacOverride),
    ("dac_read_search", Cap::DacReadSearch),
    ("fowner", Cap::Fowner),
    ("fsetid", Cap::Fsetid),
    ("kill", Cap::Kill),
    ("setgid", Cap::Setgid),
    ("setuid", Cap::Setuid),
    ("setpcap", Cap::Setpcap),
    ("linux_immutable", Cap::LinuxImmutable),
    ("net_bind_service", Cap::NetBindService),
    ("net_broadcast", Cap::NetBroadcast),
    ("net_admin", Cap::NetAdmin),
    ("net_raw", Cap::NetRaw),
    ("ipc_lock", Cap::IpcLock),
    ("ipc_owner", Cap::IpcOwner),
    ("sys_module", Cap::SysModule),
    ("sys_rawio", Cap::SysRawio),
    ("sys_chroot", Cap::SysChroot),
    ("sys_ptrace", Cap::SysPtrace),
    ("sys_pacct", Cap::SysPacct),
    ("sys_admin", Cap::SysAdmin),
    ("sys_boot", Cap::SysBoot),
    ("sys_nice", Cap::SysNice),
    ("sys_resource", Cap::SysResource),
    ("sys_time", Cap::SysTime),
    ("sys_tty_config", Cap::SysTtyConfig),
    ("mknod", Cap::Mknod),
    ("lease", Cap::Lease),
    ("audit_write", Cap::AuditWrite),
    ("audit_control", Cap::AuditControl),
    ("setfcap", Cap::Setfcap),
    ("mac_override", Cap::MacOverride),
    ("mac_admin", Cap::MacAdmin),
    ("syslog", Cap::Syslog),
    ("wake_alarm", Cap::WakeAlarm),
    ("block_suspend", Cap::BlockSuspend),
    ("audit_read", Cap::AuditRead),
    ("perfmon", Cap::Perfmon),
    ("bpf", Cap::Bpf),
    ("checkpoint_restore", Cap::CheckpointRestore),
];

pub(crate) const fn named(cap: Cap) -> &'static str {
    name_in(&NAMED, cap)
}

pub(crate) const fn lookup(name: &str) -> Option<Cap> {
    cap_in(&NAMED, name)
}

const fn name_in(entries: &[(&'static str, Cap)], cap: Cap) -> &'static str {
    match *entries {
        [(name, candidate), ref rest @ ..] if candidate.number() == cap.number() => name,
        [_, ref rest @ ..] => name_in(rest, cap),
        [] => "unknown",
    }
}

const fn cap_in(entries: &[(&'static str, Cap)], name: &str) -> Option<Cap> {
    match *entries {
        [(candidate, cap), ref rest @ ..] if equal(candidate, name) => Some(cap),
        [_, ref rest @ ..] => cap_in(rest, name),
        [] => None,
    }
}

const fn equal(left: &str, right: &str) -> bool {
    same_bytes(left.as_bytes(), right.as_bytes())
}

const fn same_bytes(left: &[u8], right: &[u8]) -> bool {
    match (left.split_first(), right.split_first()) {
        (Some((left_byte, left_rest)), Some((right_byte, right_rest))) => {
            *left_byte == *right_byte && same_bytes(left_rest, right_rest)
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_round_trips_through_its_name() {
        // ARRANGE
        let all = Cap::ALL;

        // ACT
        let resolved: Vec<_> = all.iter().map(|cap| Cap::from_name(cap.name())).collect();

        // ASSERT
        for (cap, resolved) in all.iter().zip(resolved) {
            assert_eq!(
                resolved,
                Some(*cap),
                "{} resolved to {resolved:?}",
                cap.name()
            );
        }
    }

    #[test]
    fn unknown_name_resolves_to_none() {
        // ARRANGE
        let name = "CAP_CHOWN";

        // ACT
        let resolved = Cap::from_name(name);

        // ASSERT
        assert_eq!(resolved, None);
    }
}
