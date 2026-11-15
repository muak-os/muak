//! Ordered stages of the child path.
//!
//! The child path cannot format messages, so it cannot report a `SandboxError`
//! the way the parent side does. A failure is reported as a fixed
//! `(Stage, Errno)` pair written to a CLOEXEC pipe, and `Stage` is the compact
//! value that says which step of the child path failed.

/// Step of the child path a failure is attributed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Namespace creation and joining.
    Namespaces = 1,
    /// Mount and mount point manipulation.
    Mounts = 2,
    /// Root filesystem switch.
    Rootfs = 3,
    /// Control group placement.
    Cgroup = 4,
    /// Credential and capability drop.
    Credentials = 5,
    /// Seccomp filter installation.
    Seccomp = 6,
    /// Resource limit configuration.
    Limits = 7,
    /// Final exec of the workload.
    Exec = 8,
}

impl Stage {
    /// Returns the code reported through the pipe.
    #[must_use = "the code is what the parent reads back from the pipe"]
    pub const fn code(self) -> u16 {
        match self {
            Self::Namespaces => 1,
            Self::Mounts => 2,
            Self::Rootfs => 3,
            Self::Cgroup => 4,
            Self::Credentials => 5,
            Self::Seccomp => 6,
            Self::Limits => 7,
            Self::Exec => 8,
        }
    }

    /// Returns the lowercase name of this stage.
    #[must_use = "the name is what callers format and compare against"]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Namespaces => "namespaces",
            Self::Mounts => "mounts",
            Self::Rootfs => "rootfs",
            Self::Cgroup => "cgroup",
            Self::Credentials => "credentials",
            Self::Seccomp => "seccomp",
            Self::Limits => "limits",
            Self::Exec => "exec",
        }
    }

    /// Returns the stage carrying `code`, or `None` when no stage does.
    #[must_use = "callers must handle a code they do not know"]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(Self::Namespaces),
            2 => Some(Self::Mounts),
            3 => Some(Self::Rootfs),
            4 => Some(Self::Cgroup),
            5 => Some(Self::Credentials),
            6 => Some(Self::Seccomp),
            7 => Some(Self::Limits),
            8 => Some(Self::Exec),
            _ => None,
        }
    }
}
