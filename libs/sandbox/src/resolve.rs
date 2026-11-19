//! Requirements a caller places on detected capabilities.

#[cfg(feature = "spawn")]
use std::path::Path;

use crate::error::{Mechanism, Result, SandboxError};
#[cfg(feature = "spawn")]
use crate::mount;
#[cfg(feature = "spawn")]
use crate::namespace;
#[cfg(feature = "spawn")]
use crate::policy::Policy;
use crate::probe::Capabilities;

// rustix converts a path through a stack buffer below this length and allocates above it, so a longer path is refused.
#[cfg(feature = "spawn")]
const PATH_LIMIT: usize = 256;

/// A policy that passed resolution and can be spawned.
#[cfg(feature = "spawn")]
#[derive(Clone, Copy, Debug)]
pub struct Resolved<'a> {
    pub(crate) policy: &'a Policy<'a>,
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
    /// # Errors
    ///
    /// Returns [`SandboxError::NotCompiled`] when a needed mechanism is absent from this build, and [`SandboxError::Unsupported`] when the kernel lacks a needed flag.
    pub fn resolve(self, caps: &Capabilities) -> Result<()> {
        self.0.flags().try_for_each(|flag| check(flag, *caps))
    }

    /// Returns the required flags this build compiles but the kernel lacks.
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
    /// # Errors
    ///
    /// Returns [`SandboxError::NotCompiled`] when a requested mechanism is absent from this build, and [`SandboxError::Unsupported`] when the kernel lacks a needed facility or the policy asks for a combination this phase cannot honour.
    pub fn resolve(&self, caps: &Capabilities) -> Result<Resolved<'_>> {
        semantics(self)?;
        hierarchy(self, *caps)?;
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

        Ok(Resolved { policy: self })
    }
}

// Every rule a policy has to follow on its own, before the machine is consulted, because the caller can only fix the policy.
#[cfg(feature = "spawn")]
fn semantics(policy: &Policy<'_>) -> Result<()> {
    let mounted = !policy.mounts.is_empty();
    if mounted && !policy.namespaces.contains(namespace::Set::MOUNT) {
        return Err(SandboxError::Unsupported {
            mechanism: Mechanism::Mounts,
            detail: "add MOUNT to the namespace set, a mount without it reaches the host",
        });
    }
    if policy.rootfs.is_some() && !policy.namespaces.contains(namespace::Set::MOUNT) {
        return Err(SandboxError::Unsupported {
            mechanism: Mechanism::Rootfs,
            detail: "add MOUNT to the namespace set, a root switch without it reaches the host",
        });
    }
    if let Some(root) = policy.rootfs {
        usable(Mechanism::Rootfs, root)?;
    }
    policy.mounts.iter().try_for_each(|entry| {
        usable(Mechanism::Mounts, entry.target)?;
        match entry.source {
            mount::Source::Bind(source) | mount::Source::BindReadOnly(source) => {
                usable(Mechanism::Mounts, source)
            }
            mount::Source::Tmpfs
            | mount::Source::Proc
            | mount::Source::Sysfs
            | mount::Source::Devpts => Ok(()),
        }
    })
}

// Asking for a limit the kernel enforces on the cgroup itself is what makes the unified hierarchy mandatory, and the check runs before the requirement set so the failure names the hierarchy to boot.
#[cfg(feature = "spawn")]
fn hierarchy(policy: &Policy<'_>, caps: Capabilities) -> Result<()> {
    if policy.cgroup.enforces() && !caps.contains(Capabilities::CGROUP_V2) {
        return Err(SandboxError::Unsupported {
            mechanism: Mechanism::Cgroup,
            detail: "CGROUP_V2, memory.max and pids.max need the unified hierarchy",
        });
    }

    Ok(())
}

#[cfg(feature = "spawn")]
fn usable(mechanism: Mechanism, path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(SandboxError::Unsupported {
            mechanism,
            detail: "use an absolute path, the child resolves it after the switch",
        });
    }
    if path.as_os_str().len() >= PATH_LIMIT {
        return Err(SandboxError::Unsupported {
            mechanism,
            detail: "use a path shorter than 256 bytes, the child path cannot allocate",
        });
    }

    Ok(())
}

fn check(flag: Capabilities, caps: Capabilities) -> Result<()> {
    // A flag without a mechanism is a programming error from the probe vocabulary, reported instead of panicking.
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

#[cfg(test)]
#[cfg(feature = "spawn")]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::mount::{Entry, Flags, Source};
    use crate::namespace::Set;
    use crate::policy::Policy;

    fn tmpfs(target: &Path) -> Entry<'_> {
        Entry {
            source: Source::Tmpfs,
            target,
            data: None,
            flags: Flags::new(),
        }
    }

    #[test]
    fn mounts_without_the_mount_namespace_are_refused() {
        // ARRANGE
        let entries = [tmpfs(Path::new("/root"))];
        let policy = Policy::new().mounts(&entries);

        // ACT
        let result = semantics(&policy);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Mounts,
                    detail
                }) if detail.contains("MOUNT")
            ),
            "expected a mount namespace complaint, got {result:?}"
        );
    }

    #[test]
    fn a_root_filesystem_without_the_mount_namespace_is_refused() {
        // ARRANGE
        let policy = Policy::new().rootfs(Path::new("/srv/root"));

        // ACT
        let result = semantics(&policy);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Rootfs,
                    detail
                }) if detail.contains("MOUNT")
            ),
            "expected a mount namespace complaint, got {result:?}"
        );
    }

    #[test]
    fn a_root_filesystem_without_extra_mounts_is_accepted() {
        // ARRANGE
        let policy = Policy::new()
            .namespaces(Set::MOUNT)
            .rootfs(Path::new("/srv/root"));

        // ACT
        let result = semantics(&policy);

        // ASSERT
        assert!(result.is_ok(), "a bare root filesystem failed: {result:?}");
    }

    #[test]
    fn a_mount_target_of_the_path_limit_is_refused() {
        // ARRANGE
        let long = format!("/{}", "a".repeat(PATH_LIMIT - 1));
        assert_eq!(long.len(), PATH_LIMIT);
        let entries = [tmpfs(Path::new(&long))];
        let policy = Policy::new().namespaces(Set::MOUNT).mounts(&entries);

        // ACT
        let result = semantics(&policy);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Mounts,
                    detail
                }) if detail.contains("256")
            ),
            "expected a path length complaint, got {result:?}"
        );
    }

    #[test]
    fn a_relative_mount_target_is_refused() {
        // ARRANGE
        let entries = [tmpfs(Path::new("relative"))];
        let policy = Policy::new().namespaces(Set::MOUNT).mounts(&entries);

        // ACT
        let result = semantics(&policy);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Mounts,
                    detail
                }) if detail.contains("absolute")
            ),
            "expected an absolute path complaint, got {result:?}"
        );
    }

    #[test]
    fn a_relative_bind_source_is_refused() {
        // ARRANGE
        let entries = [Entry {
            source: Source::BindReadOnly(Path::new("relative")),
            target: Path::new("/root/host"),
            data: None,
            flags: Flags::new(),
        }];
        let policy = Policy::new().namespaces(Set::MOUNT).mounts(&entries);

        // ACT
        let result = semantics(&policy);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Mounts,
                    detail
                }) if detail.contains("absolute")
            ),
            "expected an absolute path complaint, got {result:?}"
        );
    }

    #[test]
    fn a_relative_root_filesystem_is_refused() {
        // ARRANGE
        let policy = Policy::new()
            .namespaces(Set::MOUNT)
            .rootfs(Path::new("relative"));

        // ACT
        let result = semantics(&policy);

        // ASSERT
        assert!(
            matches!(
                result,
                Err(SandboxError::Unsupported {
                    mechanism: Mechanism::Rootfs,
                    detail
                }) if detail.contains("absolute")
            ),
            "expected an absolute path complaint, got {result:?}"
        );
    }
}
