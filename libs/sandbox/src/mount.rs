//! Mounts a child performs before exec.

use core::ffi::CStr;
use std::path::Path;

#[cfg(all(feature = "mounts", feature = "spawn"))]
use crate::error::Errno;
#[cfg(all(feature = "mounts", feature = "spawn"))]
use crate::sys::linux;

/// Filesystem a mount entry brings into the child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'a> {
    /// Bind mount of an existing path.
    Bind(&'a Path),
    /// Bind mount of an existing path, read only whatever the source allows.
    BindReadOnly(&'a Path),
    /// Empty in memory filesystem, the usual sandbox root.
    Tmpfs,
    /// Kernel process table.
    Proc,
    /// Kernel device and bus tree.
    Sysfs,
    /// Pseudo terminal multiplexer.
    Devpts,
}

/// One mount the child performs before exec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// What is mounted.
    pub source: Source<'a>,
    /// Where it is mounted, resolved in the child after the namespaces exist.
    pub target: &'a Path,
    /// Option string the filesystem understands, ignored by bind mounts.
    pub data: Option<&'a CStr>,
    /// Flags the new mount carries.
    pub flags: Flags,
}

/// Flags a mount carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flags {
    /// Mount read only.
    pub read_only: bool,
    /// Ignore set-user-ID and set-group-ID bits.
    pub no_suid: bool,
    /// Refuse device nodes.
    pub no_dev: bool,
    /// Refuse execution.
    pub no_exec: bool,
    /// Apply the mount to every submount of the source or target.
    pub recursive: bool,
}

impl Flags {
    /// Returns the secure default flags.
    #[must_use = "the flags are the value the caller asked for"]
    pub const fn new() -> Self {
        Self {
            read_only: false,
            no_suid: true,
            no_dev: true,
            no_exec: true,
            recursive: true,
        }
    }
}

impl Default for Flags {
    fn default() -> Self {
        Self::new()
    }
}

/// Makes the mount namespace private, then mounts every entry in order.
#[cfg(all(feature = "mounts", feature = "spawn"))]
pub(crate) fn apply(entries: &[Entry<'_>]) -> Result<(), Errno> {
    linux::mount::private_root()?;

    entries.iter().try_for_each(entry)
}

#[cfg(all(feature = "mounts", feature = "spawn"))]
fn entry(entry: &Entry<'_>) -> Result<(), Errno> {
    match entry.source {
        Source::Bind(source) => linux::mount::bind(source, entry.target, entry.flags),
        Source::BindReadOnly(source) => {
            let flags = Flags {
                read_only: true,
                ..entry.flags
            };

            linux::mount::bind(source, entry.target, flags)
        }
        Source::Tmpfs => linux::mount::filesystem(c"tmpfs", entry.target, entry.data, entry.flags),
        Source::Proc => linux::mount::filesystem(c"proc", entry.target, entry.data, entry.flags),
        Source::Sysfs => linux::mount::filesystem(c"sysfs", entry.target, entry.data, entry.flags),
        Source::Devpts => {
            linux::mount::filesystem(c"devpts", entry.target, entry.data, entry.flags)
        }
    }
}
