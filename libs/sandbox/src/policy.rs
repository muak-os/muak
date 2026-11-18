//! The policy a workload is spawned under.
//!
//! A policy always describes what the caller wants, whatever this build can
//! compile. It is the resolve step in [`crate::resolve`] that compares the
//! description with the mechanisms this build carries and the facilities the
//! kernel offers. Seccomp is absent on purpose: it belongs to a later phase.
//! Namespaces, mounts, the root filesystem and the cgroup limits are present as
//! data only, so resolution can report them as not compiled.

use std::path::Path;

use crate::credentials::{Caps, Id};
use crate::limits::Limits;
use crate::probe::Capabilities;
use crate::resolve::Requirements;
use crate::{cgroup, mount, namespace};

/// Everything a child is configured with before exec.
#[derive(Clone, Debug)]
pub struct Policy<'a> {
    pub(crate) id: Id<'a>,
    pub(crate) caps: Option<Caps>,
    pub(crate) limits: Limits,
    pub(crate) no_new_privs: bool,
    pub(crate) namespaces: namespace::Set,
    pub(crate) mounts: &'a [mount::Entry<'a>],
    pub(crate) rootfs: Option<&'a Path>,
    pub(crate) cgroup: cgroup::Limits,
}

impl<'a> Policy<'a> {
    /// Returns a policy that keeps the root identity and changes nothing.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn new() -> Self {
        Self {
            id: Id::root(),
            caps: None,
            limits: Limits::new(),
            no_new_privs: false,
            namespaces: namespace::Set::empty(),
            mounts: &[],
            rootfs: None,
            cgroup: cgroup::Limits::new(),
        }
    }

    /// Returns this policy with `id` as the child identity.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn id(mut self, id: Id<'a>) -> Self {
        self.id = id;
        self
    }

    /// Returns this policy keeping exactly `caps`.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn caps(mut self, caps: Caps) -> Self {
        self.caps = Some(caps);
        self
    }

    /// Returns this policy applying `limits` before exec.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns this policy setting `PR_SET_NO_NEW_PRIVS` in the child.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn no_new_privs(mut self) -> Self {
        self.no_new_privs = true;
        self
    }

    /// Returns this policy unsharing exactly `namespaces` in the child.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn namespaces(mut self, namespaces: namespace::Set) -> Self {
        self.namespaces = namespaces;
        self
    }

    /// Returns this policy mounting `entries` in the child, in order.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn mounts(mut self, entries: &'a [mount::Entry<'a>]) -> Self {
        self.mounts = entries;
        self
    }

    /// Returns this policy switching the child into `root` before exec.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn rootfs(mut self, root: &'a Path) -> Self {
        self.rootfs = Some(root);
        self
    }

    /// Returns this policy applying `limits` to the cgroup the child is born
    /// in.
    #[must_use = "the policy is the value the caller asked for"]
    pub const fn cgroup(mut self, limits: cgroup::Limits) -> Self {
        self.cgroup = limits;
        self
    }

    /// Returns the kernel facilities this policy needs.
    ///
    /// A kept capability set needs capset and the bounding set drop: a set
    /// smaller than every capability drives `PR_CAPBSET_DROP`, which needs
    /// `CAP_SETPCAP` in the process. Limits need no kernel capability because
    /// rlimits are set per process without one; whether this build carries the
    /// limits mechanism is checked during resolution instead. A namespace needs
    /// the flag that names it, a mount needs the mount namespace plus the flag
    /// of the filesystem it mounts, and a root filesystem needs the mount
    /// namespace plus `pivot_root`. A cgroup limit needs the unified v2
    /// hierarchy, which is the only hierarchy this crate drives.
    ///
    /// This is what a build without `spawn` validates with: it has no
    /// `Policy::resolve` but still needs to know whether a policy can be
    /// honoured before it hands the policy to a process that can spawn.
    #[must_use = "the requirements are what the caller resolves"]
    pub fn requirements(&self) -> Requirements {
        let mut requirements = if self.caps.is_some() {
            Requirements::new()
                .require(Capabilities::CAPSET)
                .require(Capabilities::CAP_BOUNDING)
        } else {
            Requirements::new()
        };
        for kind in self.namespaces.flags() {
            requirements = requirements.require(namespace_requirement(kind));
        }
        // A mount needs the mount namespace whatever it mounts, because a
        // mount made outside one reaches the host.
        if !self.mounts.is_empty() {
            requirements = requirements.require(Capabilities::MOUNT_NS);
            let filesystems = self
                .mounts
                .iter()
                .filter_map(|entry| source_requirement(entry.source));
            requirements = filesystems.fold(requirements, Requirements::require);
        }
        if self.rootfs.is_some() {
            requirements = requirements
                .require(Capabilities::MOUNT_NS)
                .require(Capabilities::PIVOT_ROOT);
        }
        if self.cgroup.requested() {
            requirements = requirements.require(Capabilities::CGROUP_V2);
        }
        requirements
    }
}

impl Default for Policy<'_> {
    fn default() -> Self {
        Self::new()
    }
}

const fn namespace_requirement(kind: namespace::Set) -> Capabilities {
    match kind {
        namespace::Set::MOUNT => Capabilities::MOUNT_NS,
        namespace::Set::NETWORK => Capabilities::NET_NS,
        namespace::Set::IPC => Capabilities::IPC_NS,
        namespace::Set::UTS => Capabilities::UTS_NS,
        namespace::Set::CGROUP => Capabilities::CGROUP_NS,
        _ => Capabilities::empty(),
    }
}

const fn source_requirement(source: mount::Source<'_>) -> Option<Capabilities> {
    match source {
        mount::Source::Bind(_) | mount::Source::BindReadOnly(_) => None,
        mount::Source::Tmpfs => Some(Capabilities::TMPFS),
        mount::Source::Proc => Some(Capabilities::PROC),
        mount::Source::Sysfs => Some(Capabilities::SYSFS),
        mount::Source::Devpts => Some(Capabilities::DEVPTS),
    }
}
