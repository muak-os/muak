//! Kernel capabilities the sandbox can detect.

#[cfg(feature = "probe")]
pub mod detect;

use core::fmt;
#[cfg(feature = "probe")]
use std::path::Path;

use crate::error::Mechanism;

const NAMED: [(Capabilities, &str); 27] = [
    (Capabilities::MOUNT_NS, "MOUNT_NS"),
    (Capabilities::PID_NS, "PID_NS"),
    (Capabilities::NET_NS, "NET_NS"),
    (Capabilities::IPC_NS, "IPC_NS"),
    (Capabilities::UTS_NS, "UTS_NS"),
    (Capabilities::USER_NS, "USER_NS"),
    (Capabilities::CGROUP_NS, "CGROUP_NS"),
    (Capabilities::TIME_NS, "TIME_NS"),
    (Capabilities::CGROUP_V2, "CGROUP_V2"),
    (Capabilities::OVERLAYFS, "OVERLAYFS"),
    (Capabilities::TMPFS, "TMPFS"),
    (Capabilities::DEVPTS, "DEVPTS"),
    (Capabilities::PROC, "PROC"),
    (Capabilities::SYSFS, "SYSFS"),
    (Capabilities::EROFS, "EROFS"),
    (Capabilities::SECCOMP, "SECCOMP"),
    (Capabilities::SECCOMP_NOTIF, "SECCOMP_NOTIF"),
    (Capabilities::PIVOT_ROOT, "PIVOT_ROOT"),
    (Capabilities::OPEN_TREE, "OPEN_TREE"),
    (Capabilities::MOUNT_SETATTR, "MOUNT_SETATTR"),
    (Capabilities::IDMAPPED_MOUNTS, "IDMAPPED_MOUNTS"),
    (Capabilities::CAPSET, "CAPSET"),
    (Capabilities::CAP_BOUNDING, "CAP_BOUNDING"),
    (Capabilities::PIDFD, "PIDFD"),
    (Capabilities::CLONE3, "CLONE3"),
    (Capabilities::CLONE_INTO_CGROUP, "CLONE_INTO_CGROUP"),
    (Capabilities::UNPRIV_USER_NS, "UNPRIV_USER_NS"),
];

/// Set of kernel facilities present on the running kernel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities(u32);

impl Capabilities {
    /// Kernel offers mount namespace creation.
    pub const MOUNT_NS: Self = Self(0x0000_0001);
    /// Kernel offers PID namespace creation.
    pub const PID_NS: Self = Self(0x0000_0002);
    /// Kernel offers network namespace creation.
    pub const NET_NS: Self = Self(0x0000_0004);
    /// Kernel offers `SysV` IPC namespace creation.
    pub const IPC_NS: Self = Self(0x0000_0008);
    /// Kernel offers UTS namespace creation.
    pub const UTS_NS: Self = Self(0x0000_0010);
    /// Kernel offers user namespace creation.
    pub const USER_NS: Self = Self(0x0000_0020);
    /// Kernel offers cgroup namespace creation.
    pub const CGROUP_NS: Self = Self(0x0000_0040);
    /// Kernel offers time namespace creation.
    pub const TIME_NS: Self = Self(0x0000_0080);
    /// Kernel offers the cgroup v2 unified hierarchy.
    pub const CGROUP_V2: Self = Self(0x0000_0100);
    /// Kernel offers the overlay filesystem.
    pub const OVERLAYFS: Self = Self(0x0000_0800);
    /// Kernel offers the tmpfs filesystem.
    pub const TMPFS: Self = Self(0x0000_1000);
    /// Kernel offers the devpts filesystem.
    pub const DEVPTS: Self = Self(0x0000_2000);
    /// Kernel offers the proc filesystem.
    pub const PROC: Self = Self(0x0000_4000);
    /// Kernel offers the sysfs filesystem.
    pub const SYSFS: Self = Self(0x0000_8000);
    /// Kernel offers the EROFS filesystem.
    pub const EROFS: Self = Self(0x0001_0000);
    /// Kernel offers seccomp filter installation.
    pub const SECCOMP: Self = Self(0x0002_0000);
    /// Kernel offers seccomp user notification.
    pub const SECCOMP_NOTIF: Self = Self(0x0004_0000);
    /// Kernel offers the pivot root syscall.
    pub const PIVOT_ROOT: Self = Self(0x0008_0000);
    /// Kernel offers the open tree syscall.
    pub const OPEN_TREE: Self = Self(0x0010_0000);
    /// Kernel offers the mount setattr syscall.
    pub const MOUNT_SETATTR: Self = Self(0x0020_0000);
    /// Kernel offers ID-mapped mounts.
    pub const IDMAPPED_MOUNTS: Self = Self(0x0040_0000);
    /// Kernel offers the capset syscall.
    pub const CAPSET: Self = Self(0x0080_0000);
    /// Kernel offers capability bounding set changes.
    pub const CAP_BOUNDING: Self = Self(0x0100_0000);
    /// Kernel offers pidfd open and signalling calls.
    pub const PIDFD: Self = Self(0x0200_0000);
    /// Kernel offers the clone3 syscall.
    pub const CLONE3: Self = Self(0x0400_0000);
    /// Kernel offers the clone into cgroup clone3 flag.
    pub const CLONE_INTO_CGROUP: Self = Self(0x0800_0000);
    /// Kernel offers unprivileged user namespace creation.
    pub const UNPRIV_USER_NS: Self = Self(0x2000_0000);

    /// Returns the empty set.
    #[must_use = "the empty set is the starting point of a union"]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Returns the union of both sets.
    #[must_use = "callers need the union to keep accumulating flags"]
    pub const fn insert(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns whether every flag of `other` is present.
    #[must_use = "the answer decides the requested sandbox"]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns whether at least one flag of `other` is present.
    #[must_use = "the answer decides the requested sandbox"]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Returns whether no flag is present.
    #[must_use = "the answer decides whether probing found anything"]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns the raw bit set.
    #[must_use = "the bits are the value the caller asked for"]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns the name of a single flag, or `None` for any other set.
    #[must_use = "the name is what a failure reports"]
    pub const fn name(self) -> Option<&'static str> {
        name_in(&NAMED, self)
    }

    /// Returns the mechanism of a single flag, or `None` for any other set.
    #[must_use = "the mechanism decides whether a requirement can be met"]
    pub const fn mechanism(self) -> Option<Mechanism> {
        match self {
            Self::MOUNT_NS
            | Self::PID_NS
            | Self::NET_NS
            | Self::IPC_NS
            | Self::UTS_NS
            | Self::CGROUP_NS
            | Self::TIME_NS
            | Self::USER_NS
            | Self::UNPRIV_USER_NS => Some(Mechanism::Namespaces),
            Self::CGROUP_V2 => Some(Mechanism::Cgroup),
            Self::OVERLAYFS => Some(Mechanism::Overlay),
            Self::TMPFS
            | Self::DEVPTS
            | Self::PROC
            | Self::SYSFS
            | Self::OPEN_TREE
            | Self::MOUNT_SETATTR
            | Self::IDMAPPED_MOUNTS => Some(Mechanism::Mounts),
            Self::EROFS | Self::PIVOT_ROOT => Some(Mechanism::Rootfs),
            Self::SECCOMP | Self::SECCOMP_NOTIF => Some(Mechanism::Seccomp),
            Self::CAPSET | Self::CAP_BOUNDING => Some(Mechanism::Caps),
            Self::PIDFD | Self::CLONE3 | Self::CLONE_INTO_CGROUP => Some(Mechanism::Spawn),
            _ => None,
        }
    }

    /// Yields one single flag per set bit, in bit order.
    pub fn flags(self) -> impl Iterator<Item = Self> {
        NAMED
            .into_iter()
            .filter(move |entry| self.contains(entry.0))
            .map(|entry| entry.0)
    }
}

impl fmt::Display for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }

        let mut separator = "";
        for entry in NAMED.into_iter().filter(|entry| self.contains(entry.0)) {
            write!(f, "{separator}{}", entry.1)?;
            separator = ", ";
        }

        Ok(())
    }
}

/// Returns the capabilities detectable on the running machine.
#[cfg(feature = "probe")]
#[must_use = "probing has no effect beyond the answer"]
pub fn current() -> Capabilities {
    at(Path::new("/"))
}

/// Returns the capabilities inferred from the tree at `root` plus those of the running kernel.
#[cfg(feature = "probe")]
#[must_use = "probing has no effect beyond the answer"]
pub fn at(root: &Path) -> Capabilities {
    detect::files(root).insert(detect::syscalls())
}

const fn name_in(
    entries: &[(Capabilities, &'static str)],
    flag: Capabilities,
) -> Option<&'static str> {
    match *entries {
        [(candidate, name), ref rest @ ..] if flag.0 == candidate.0 => Some(name),
        [_, ref rest @ ..] => name_in(rest, flag),
        [] => None,
    }
}
