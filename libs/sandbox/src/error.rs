//! Error type, mechanism names and the shared result alias.

use core::fmt;

use thiserror::Error;

/// Raw kernel error number.
///
/// The type is a plain integer on purpose: it can be built between fork and
/// exec, where no allocator is reachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(i32);

/// Mechanism a failure belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mechanism {
    /// Capability probing of the running kernel.
    Probe,
    /// Namespace creation and joining.
    Namespaces,
    /// Mount and mount point manipulation.
    Mounts,
    /// Root filesystem assembly.
    Rootfs,
    /// Control group placement, on the unified v2 hierarchy only.
    Cgroup,
    /// Overlay layer assembly.
    Overlay,
    /// Capability set manipulation.
    Caps,
    /// Seccomp filter installation.
    Seccomp,
    /// Resource limit configuration.
    Limits,
    /// Process spawning and execution.
    Spawn,
}

/// Failure returned by sandbox operations.
///
/// Every variant is allocation free so that it can be constructed between fork
/// and exec.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum SandboxError {
    /// Reports an I/O failure that no mechanism claims.
    #[error("I/O error: {0}")]
    Io(Errno),
    /// Reports a mechanism that this build left out.
    #[error("{mechanism} is not compiled into this build")]
    NotCompiled {
        /// Mechanism that was requested.
        mechanism: Mechanism,
    },
    /// Reports a mechanism the running kernel does not support.
    #[error("{mechanism} is unsupported here: {detail}")]
    Unsupported {
        /// Mechanism that was requested.
        mechanism: Mechanism,
        /// What exactly is missing.
        detail: &'static str,
    },
    /// Reports a mechanism the kernel refused to grant.
    #[error("{mechanism} was denied: {source}")]
    Denied {
        /// Mechanism that was refused.
        mechanism: Mechanism,
        /// Kernel error number behind the refusal.
        source: Errno,
    },
}

/// Shared result type for sandbox operations.
pub type Result<T> = core::result::Result<T, SandboxError>;

impl Errno {
    /// Wraps a raw kernel error number.
    #[must_use = "the error number is the only value this type carries"]
    pub const fn new(raw: i32) -> Self {
        Self(raw)
    }

    /// Returns the raw kernel error number.
    #[must_use = "the raw number is the value the caller asked for"]
    pub const fn raw(self) -> i32 {
        self.0
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "errno {}", self.0)
    }
}

impl core::error::Error for Errno {}

impl Mechanism {
    /// Returns the lowercase name of this mechanism.
    #[must_use = "the name is what callers format and compare against"]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Namespaces => "namespaces",
            Self::Mounts => "mounts",
            Self::Rootfs => "rootfs",
            Self::Cgroup => "cgroup",
            Self::Overlay => "overlay",
            Self::Caps => "caps",
            Self::Seccomp => "seccomp",
            Self::Limits => "limits",
            Self::Spawn => "spawn",
        }
    }

    /// Returns whether this build compiled the mechanism in.
    #[must_use = "the answer separates a build problem from a kernel problem"]
    pub const fn compiled(self) -> bool {
        match self {
            Self::Probe => cfg!(feature = "probe"),
            Self::Namespaces => cfg!(feature = "namespaces"),
            Self::Mounts => cfg!(feature = "mounts"),
            Self::Rootfs => cfg!(feature = "rootfs"),
            Self::Cgroup => cfg!(feature = "cgroupv2"),
            Self::Overlay => cfg!(feature = "overlay"),
            Self::Caps => cfg!(feature = "caps"),
            Self::Seccomp => cfg!(feature = "seccomp"),
            Self::Limits => cfg!(feature = "limits"),
            Self::Spawn => cfg!(feature = "spawn"),
        }
    }
}

impl fmt::Display for Mechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
