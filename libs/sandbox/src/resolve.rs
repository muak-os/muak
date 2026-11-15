//! Requirements a caller places on detected capabilities.
//!
//! A caller builds a requirement set, then asks it to resolve against the
//! capabilities the machine reported. A build problem is reported before a
//! kernel problem, so a missing mechanism is never mistaken for an unsupported
//! kernel. Refusal, the `Denied` variant, is not produced here: it comes from
//! the child path through [`crate::spawn`].

use crate::error::{Mechanism, Result, SandboxError};
#[cfg(feature = "spawn")]
use crate::policy::Policy;
use crate::probe::Capabilities;

/// A policy that passed resolution and can be spawned.
#[cfg(feature = "spawn")]
#[derive(Clone, Copy, Debug)]
pub struct Resolved<'a> {
    pub(crate) policy: &'a Policy<'a>,
    pub(crate) pidfd: bool,
}

/// Kernel facilities a caller requires before a sandbox can be built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requirements(Capabilities);

impl Requirements {
    /// Returns an empty requirement set.
    #[must_use = "the empty set is the starting point of a union"]
    pub const fn new() -> Self {
        Self(Capabilities::empty())
    }

    /// Returns the union of both requirement sets.
    #[must_use = "callers need the union to keep accumulating flags"]
    pub const fn require(self, needed: Capabilities) -> Self {
        Self(self.0.insert(needed))
    }

    /// Returns every requested flag.
    #[must_use = "the bits are the value the caller asked for"]
    pub const fn needs(self) -> Capabilities {
        self.0
    }

    /// Returns the first requirement that cannot be met, in bit order.
    ///
    /// For each flag the mechanism is checked first: a mechanism this build
    /// left out fails with `NotCompiled` before the kernel is consulted, and a
    /// flag the kernel lacks fails with `Unsupported`.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::NotCompiled`] when a needed mechanism is absent
    /// from this build, and [`SandboxError::Unsupported`] when the kernel lacks
    /// a needed flag.
    pub fn resolve(self, caps: &Capabilities) -> Result<()> {
        self.0.flags().try_for_each(|flag| check(flag, *caps))
    }

    /// Returns the required flags this build compiles but the kernel lacks.
    ///
    /// This is what a relaxed caller logs before degrading.
    #[must_use = "the absent flags are what the caller reports"]
    pub fn missing(self, caps: &Capabilities) -> Capabilities {
        self.0
            .flags()
            .filter(|flag| absent_from(*flag, *caps))
            .fold(Capabilities::empty(), Capabilities::insert)
    }
}

#[cfg(feature = "spawn")]
impl Policy<'_> {
    /// Resolves this policy against the capabilities of the running machine.
    ///
    /// Requirements are resolved first, so a mechanism this build left out is
    /// reported before a kernel limitation. The checks that only a policy can
    /// fail follow: a non-root identity cannot keep capabilities without
    /// ambient capabilities, and a requested limit needs the limits mechanism.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::NotCompiled`] when a requested mechanism is
    /// absent from this build, [`SandboxError::Unsupported`] when the kernel
    /// lacks a needed facility or the policy asks for a combination this phase
    /// cannot honour.
    pub fn resolve(&self, caps: &Capabilities) -> Result<Resolved<'_>> {
        self.requirements().resolve(caps)?;
        let ambient = self
            .caps
            .is_some_and(|kept| self.id.uid != 0 && !kept.is_empty());
        if ambient {
            return Err(SandboxError::Unsupported {
                mechanism: Mechanism::Caps,
                detail: "non-root capabilities need ambient capabilities",
            });
        }
        if self.limits.requested() && !Mechanism::Limits.compiled() {
            return Err(SandboxError::NotCompiled {
                mechanism: Mechanism::Limits,
            });
        }
        Ok(Resolved {
            policy: self,
            pidfd: caps.contains(Capabilities::PIDFD),
        })
    }
}

fn check(flag: Capabilities, caps: Capabilities) -> Result<()> {
    // A flag without a mechanism is a programming error. Report it instead of
    // panicking. The probe mechanism is the closest owner, because such a flag
    // can only come from the probe vocabulary.
    let Some(mechanism) = flag.mechanism() else {
        return Err(SandboxError::Unsupported {
            mechanism: Mechanism::Probe,
            detail: "unknown mechanism",
        });
    };
    if !mechanism.compiled() {
        return Err(SandboxError::NotCompiled { mechanism });
    }
    if !caps.contains(flag) {
        return Err(SandboxError::Unsupported {
            mechanism,
            detail: flag.name().unwrap_or("unknown flag"),
        });
    }
    Ok(())
}

fn absent_from(flag: Capabilities, caps: Capabilities) -> bool {
    flag.mechanism().is_some_and(Mechanism::compiled) && !caps.contains(flag)
}
