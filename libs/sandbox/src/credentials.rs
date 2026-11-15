//! Credentials and capability sets a workload inherits.
//!
//! The types here always compile because a policy must always be expressible,
//! even in a build that cannot issue the syscalls behind it. Only the kernel
//! side in [`crate::sys::linux`] is feature gated.
//!
//! [`Caps`] describes the capabilities a child keeps, not the ones that are
//! dropped. The empty set therefore drops everything, which is the useful
//! default, while [`Caps::all`] keeps the bounding set untouched.
//!
//! Ambient capabilities and securebits are deliberately absent. Ambient
//! capabilities are what a non-root process needs to launch a child that keeps
//! privileges, so a policy that asks for both a non-root uid and a non-empty
//! set is rejected during resolution instead of silently losing privileges.
//! Securebits would make that same combination permanent across exec and are
//! not needed while ambient capabilities are missing.

use crate::cap;

/// Identity a child assumes before exec.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Id<'a> {
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) groups: Option<&'a [u32]>,
}

/// Set of capabilities a child keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps(u64);

impl<'a> Id<'a> {
    /// Returns the root identity with an empty supplementary group list.
    #[must_use = "the identity is the value the caller asked for"]
    pub const fn root() -> Self {
        Self {
            uid: 0,
            gid: 0,
            groups: Some(&[]),
        }
    }

    /// Returns the root identity keeping the parent supplementary groups.
    #[must_use = "the identity is the value the caller asked for"]
    pub const fn inherit() -> Self {
        Self {
            uid: 0,
            gid: 0,
            groups: None,
        }
    }

    /// Returns the identity with `uid` and `gid`, keeping the parent groups.
    #[must_use = "the identity is the value the caller asked for"]
    pub const fn new(uid: u32, gid: u32) -> Self {
        Self {
            uid,
            gid,
            groups: None,
        }
    }

    /// Returns this identity with `groups` as its supplementary groups.
    ///
    /// An empty slice clears the parent groups, while leaving the setter out
    /// keeps them. Clearing is what a caller dropping to another uid wants:
    /// once `setresuid` ran, the parent groups can no longer be removed.
    #[must_use = "the identity is the value the caller asked for"]
    pub const fn groups(self, groups: &'a [u32]) -> Self {
        Self {
            groups: Some(groups),
            ..self
        }
    }
}

impl Caps {
    /// Returns the empty set, which keeps no capability and drops everything.
    #[must_use = "the set is the value the caller asked for"]
    pub const fn none() -> Self {
        Self(0)
    }

    /// Returns the set carrying every capability.
    #[must_use = "the set is the value the caller asked for"]
    pub const fn all() -> Self {
        Self(0x0000_01ff_ffff_ffff)
    }

    /// Returns this set plus `cap`, meaning the capability is kept.
    #[must_use = "the set is the value the caller asked for"]
    pub const fn allow(self, cap: cap::Cap) -> Self {
        Self(self.0 | cap::bit(cap))
    }

    /// Returns whether `cap` is kept by this set.
    #[must_use = "the answer decides which capability is dropped"]
    pub const fn contains(self, cap: cap::Cap) -> bool {
        self.0 & cap::bit(cap) != 0
    }

    /// Returns whether this set keeps no capability.
    #[must_use = "the answer decides whether any capability is dropped"]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns the raw bit set.
    #[must_use = "the bits are the value the caller asked for"]
    pub const fn bits(self) -> u64 {
        self.0
    }
}
