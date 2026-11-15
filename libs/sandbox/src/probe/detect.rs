//! Capability detection.
//!
//! The module has two halves. [`files`] inspects a directory tree, normally the
//! running root but any rootfs will do, so a test can point it at a fixture.
//! [`syscalls`] asks the running kernel directly and therefore cannot be
//! exercised against a fixture: it always reflects the machine this process
//! runs on. That split is why the filesystem half exists.
//!
//! Detection never fails. A missing or unreadable file is data, not an error:
//! the flag it would have set simply stays unset, which is normal on a kernel
//! that does not carry the facility.

use std::fs;
use std::path::Path;

use super::Capabilities;
use crate::sys::linux;

const NAMESPACES: [(Capabilities, &str, &str); 7] = [
    (Capabilities::MOUNT_NS, "mnt", "mnt"),
    (Capabilities::PID_NS, "pid", "pid"),
    (Capabilities::NET_NS, "net", "net"),
    (Capabilities::IPC_NS, "ipc", "ipc"),
    (Capabilities::UTS_NS, "uts", "uts"),
    (Capabilities::CGROUP_NS, "cgroup", "cgroup"),
    (Capabilities::TIME_NS, "time", "time"),
];

const FILESYSTEMS: [(Capabilities, &str); 6] = [
    (Capabilities::OVERLAYFS, "overlay"),
    (Capabilities::TMPFS, "tmpfs"),
    (Capabilities::DEVPTS, "devpts"),
    (Capabilities::PROC, "proc"),
    (Capabilities::SYSFS, "sysfs"),
    (Capabilities::EROFS, "erofs"),
];

/// Returns every capability detectable from the directory tree at `root`.
///
/// Every path is a fixed relative name joined under `root`, so no lookup is
/// aimed outside the tree. A symlink found inside the tree is followed the way
/// the kernel would follow it, so a fixture that links elsewhere is read there.
///
/// `cgroup.kill` and `cgroup.freeze` are never looked for. The kernel creates
/// both files only on a non-root cgroup, so they describe a cgroup the caller
/// creates rather than the one it sits in, and a caller at `0::/` would never
/// observe them. The cgroup phase checks for both right after `mkdir`,
/// falling back to `cgroup.procs` plus a per-pid kill.
#[must_use = "probing has no effect beyond the answer"]
pub fn files(root: &Path) -> Capabilities {
    Capabilities::empty()
        .insert(namespaces(root))
        .insert(user_namespaces(root))
        .insert(cgroups(root))
        .insert(filesystems(root))
        .insert(seccomp(root))
}

/// Returns every capability the running kernel answers for.
///
/// Each probe asks whether the syscall exists, never whether the call would
/// succeed. `CLONE_INTO_CGROUP` stays unset: it is only knowable by actually
/// cloning into a cgroup, so it is probed by use in the cgroup phase.
#[must_use = "probing has no effect beyond the answer"]
pub fn syscalls() -> Capabilities {
    let mut found = Capabilities::empty();
    if linux::probe::pivot_root() {
        found = found.insert(Capabilities::PIVOT_ROOT);
    }
    if linux::probe::open_tree() {
        found = found.insert(Capabilities::OPEN_TREE);
    }
    if linux::probe::mount_setattr() {
        found = found
            .insert(Capabilities::MOUNT_SETATTR)
            .insert(Capabilities::IDMAPPED_MOUNTS);
    }
    if linux::probe::clone3() {
        found = found.insert(Capabilities::CLONE3);
    }
    if linux::probe::pidfd() {
        found = found.insert(Capabilities::PIDFD);
    }
    if linux::probe::capabilities() {
        found = found.insert(Capabilities::CAPSET);
    }
    if linux::probe::cap_bounding() {
        found = found.insert(Capabilities::CAP_BOUNDING);
    }
    found
}

fn namespaces(root: &Path) -> Capabilities {
    let mut found = Capabilities::empty();
    for (flag, name, limit_stem) in NAMESPACES {
        if !root.join("proc/self/ns").join(name).exists() {
            continue;
        }
        let limit = format!("proc/sys/user/max_{limit_stem}_namespaces");
        if read(root, &limit).is_some_and(|text| text.trim() == "0") {
            continue;
        }
        found = found.insert(flag);
    }
    found
}

fn user_namespaces(root: &Path) -> Capabilities {
    let limit = read(root, "proc/sys/user/max_user_namespaces");
    // A zero limit forbids every user namespace, so neither branch below can
    // grant one whatever the clone sysctl says.
    if limit.as_deref().is_some_and(|text| text.trim() == "0") {
        return Capabilities::empty();
    }

    let mut found = Capabilities::empty();
    if limit.as_deref().is_some_and(positive) {
        found = found.insert(Capabilities::USER_NS);
    }
    // `unprivileged_userns_clone` is a Debian and Ubuntu patch that mainline
    // does not export, so its absence plus a positive limit is the mainline
    // shape of an unprivileged namespace.
    let unprivileged = match read(root, "proc/sys/kernel/unprivileged_userns_clone") {
        Some(text) => text.trim() == "1",
        None => limit.as_deref().is_some_and(positive),
    };
    if unprivileged {
        found = found
            .insert(Capabilities::USER_NS)
            .insert(Capabilities::UNPRIV_USER_NS);
    }
    found
}

fn cgroups(root: &Path) -> Capabilities {
    let mut found = Capabilities::empty();
    if read(root, "sys/fs/cgroup/cgroup.controllers").is_some() {
        found = found.insert(Capabilities::CGROUP_V2);
    }
    found
}

fn filesystems(root: &Path) -> Capabilities {
    let Some(text) = read(root, "proc/filesystems") else {
        return Capabilities::empty();
    };

    let mut found = Capabilities::empty();
    for (flag, wanted) in FILESYSTEMS {
        if text
            .lines()
            .any(|line| line.split_whitespace().last() == Some(wanted))
        {
            found = found.insert(flag);
        }
    }
    found
}

fn seccomp(root: &Path) -> Capabilities {
    let Some(text) = read(root, "proc/sys/kernel/seccomp/actions_avail") else {
        return Capabilities::empty();
    };

    // The sysctl only exists when seccomp is built in, so its presence is the
    // evidence that the kernel carries the action vocabulary.
    let mut found = Capabilities::empty().insert(Capabilities::SECCOMP);
    if text.split_whitespace().any(|action| action == "user_notif") {
        found = found.insert(Capabilities::SECCOMP_NOTIF);
    }
    found
}

fn positive(text: &str) -> bool {
    text.trim().parse::<u64>().is_ok_and(|value| value > 0)
}

fn read(root: &Path, relative: &str) -> Option<String> {
    fs::read_to_string(root.join(relative)).ok()
}
