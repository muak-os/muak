//! The policy a workload is spawned under.
//!
//! A policy always describes what the caller wants, whatever this build can
//! compile. It is the resolve step in [`crate::resolve`] that compares the
//! description with the mechanisms this build carries and the facilities the
//! kernel offers. Namespaces, mounts, cgroups and seccomp are absent on
//! purpose: this phase only owns credentials, limits and the child path.

use crate::credentials::{Caps, Id};
use crate::limits::Limits;
use crate::probe::Capabilities;
use crate::resolve::Requirements;

/// Everything a child is configured with before exec.
#[derive(Clone, Debug)]
pub struct Policy<'a> {
    pub(crate) id: Id<'a>,
    pub(crate) caps: Option<Caps>,
    pub(crate) limits: Limits,
    pub(crate) no_new_privs: bool,
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

    /// Returns the kernel facilities this policy needs.
    ///
    /// A kept capability set needs capset and the bounding set drop: a set
    /// smaller than every capability drives `PR_CAPBSET_DROP`, which needs
    /// `CAP_SETPCAP` in the process. Limits need no kernel capability because
    /// rlimits are set per process without one; whether this build carries the
    /// limits mechanism is checked during resolution instead.
    ///
    /// This is what a build without `spawn` validates with: it has no
    /// `Policy::resolve` but still needs to know whether a policy can be
    /// honoured before it hands the policy to a process that can spawn.
    #[must_use = "the requirements are what the caller resolves"]
    pub const fn requirements(&self) -> Requirements {
        if self.caps.is_some() {
            Requirements::new()
                .require(Capabilities::CAPSET)
                .require(Capabilities::CAP_BOUNDING)
        } else {
            Requirements::new()
        }
    }
}

impl Default for Policy<'_> {
    fn default() -> Self {
        Self::new()
    }
}
