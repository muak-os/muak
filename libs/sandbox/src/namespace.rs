//! Namespace kinds a child is created in.

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
    /// `SysV` IPC namespace: the child gets its own message queues and semaphores.
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
#[cfg(all(feature = "namespaces", feature = "spawn"))]
pub(crate) fn apply(set: Set) -> Result<(), Errno> {
    linux::namespace::unshare(set)
}
