//! Namespace kinds a child is created in.
//!
//! A [`Set`] always compiles, because a policy must always be expressible even
//! in a build that cannot issue `unshare`. The call that applies it, `apply`,
//! needs `namespaces` and `spawn`: the child path between fork and exec is its
//! only caller, and [`crate::sys::linux`] carries the call it makes.
//!
//! Only the kinds whose name is the whole of the mechanism live here. Each one
//! gives the child a private copy of a kernel table and needs nothing else.
//!
//! # Out of scope
//!
//! The PID namespace is absent. `unshare(CLONE_NEWPID)` changes the namespace
//! of processes created afterwards and leaves the caller in the old one, so
//! honouring it needs a second fork inside the child path. That fork would
//! change what `Child::pid` means and how waiting works, so it belongs to a
//! later phase.
//!
//! The user namespace is absent too. Unsharing one without writing a uid and a
//! gid map leaves the process as `nobody` for every file on the host, which is
//! a broken sandbox rather than a confined one, and the mapping is the whole
//! point of the mechanism.

#[cfg(all(feature = "namespaces", feature = "spawn"))]
use crate::error::Errno;
#[cfg(all(feature = "namespaces", feature = "spawn"))]
use crate::sys::linux;

/// Every namespace kind, in bit order.
pub(crate) const KINDS: [Set; 5] = [Set::MOUNT, Set::NETWORK, Set::IPC, Set::UTS, Set::CGROUP];

/// Set of namespace kinds to unshare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Set(u32);

impl Set {
    /// Mount namespace: the child gets a mount table of its own.
    pub const MOUNT: Self = Self(0x0000_0001);
    /// Network namespace: the child gets its own interfaces, routes and ports.
    pub const NETWORK: Self = Self(0x0000_0002);
    /// `SysV` IPC namespace: the child gets its own message queues and
    /// semaphores.
    pub const IPC: Self = Self(0x0000_0004);
    /// UTS namespace: the child gets its own host name and NIS domain name.
    pub const UTS: Self = Self(0x0000_0008);
    /// Cgroup namespace: the child sees its own cgroup as the root of the tree.
    pub const CGROUP: Self = Self(0x0000_0010);

    /// Returns the empty set, which asks for no namespace at all.
    #[must_use = "the empty set is the starting point of a union"]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Returns the union of both sets.
    #[must_use = "callers need the union to keep accumulating kinds"]
    pub const fn insert(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns whether every kind of `other` is present.
    #[must_use = "the answer decides which kinds are unshared"]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns whether at least one kind of `other` is present.
    #[must_use = "the answer decides which kinds are unshared"]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Returns whether no kind is present.
    #[must_use = "the answer decides whether unshare is called at all"]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns the raw bit set.
    #[must_use = "the bits are the value the caller asked for"]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Yields one single kind per set bit, in bit order.
    pub fn flags(self) -> impl Iterator<Item = Self> {
        KINDS.into_iter().filter(move |kind| self.contains(*kind))
    }
}

/// Unshares every kind of `set` in one call.
///
/// The caller refuses an empty set instead of calling this, because `unshare`
/// accepts zero flags and would report a success that unshared nothing.
#[cfg(all(feature = "namespaces", feature = "spawn"))]
pub(crate) fn apply(set: Set) -> Result<(), Errno> {
    linux::namespace::unshare(set)
}
